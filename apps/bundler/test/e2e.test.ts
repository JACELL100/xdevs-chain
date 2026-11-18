// End-to-end bundling on a real dev chain (anvil --hardfork osaka with the xDEVS genesis alloc, or the xDEVS node):
// several senders in one bundle, chained same-sender ops, operator accounting, and ERC-7562 reputation against a
// misbehaving paymaster (throttled after invalidated ops, banned after making a bundle revert on chain).
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import type { Server } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { after, before, describe, it } from 'node:test'
import {
  type Address,
  type Hex,
  createPublicClient,
  createWalletClient,
  encodeFunctionData,
  http,
  pad,
  parseAbi,
  parseEther,
  parseGwei,
  toHex,
} from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { DEV_KEYS, ENTRY_POINT, ROLES } from '../scripts/devnet.ts'
import { Bundler } from '../src/bundler.ts'
import type { BundlerConfig } from '../src/config.ts'
import { RpcCodes } from '../src/errors.ts'
import { createBundlerServer } from '../src/server.ts'
import { type UserOperation, fromRpcUserOp, getUserOpHash, toRpcUserOp } from '../src/userop.ts'
import { devChain, jsonRpc } from './helpers.ts'
import { TOGGLE_PAYMASTER_BYTECODE } from './test-contracts-artifact.ts'

const FACTORY: Address = '0x0000000000000000000000000000000000001006'
const BENEFICIARY: Address = '0x000000000000000000000000000000000000bEEF'
const SIGNER = '(uint8 kind, bytes32 x, bytes32 y)'
const factoryAbi = parseAbi([
  `function getAddress(${SIGNER}[] signers, bytes32 salt) view returns (address)`,
  `function createAccount(${SIGNER}[] signers, bytes32 salt) returns (address)`,
])
const accountAbi = parseAbi(['function execute(address target, uint256 value, bytes data) returns (uint256)'])
const togglePmAbi = parseAbi([
  'function setBroken(bool b)',
  'function deposit() payable',
  'function addStake(uint32 unstakeDelaySec) payable',
])

let chain: Awaited<ReturnType<typeof devChain>>['chain']
let devnet: Awaited<ReturnType<typeof devChain>>['devnet']
const servers: Server[] = []
const bundlers: Bundler[] = []

const owner = privateKeyToAccount(DEV_KEYS[ROLES.eoa7702])
const faucet = privateKeyToAccount(DEV_KEYS[ROLES.faucet])
const recipient = privateKeyToAccount(DEV_KEYS[ROLES.recipient]).address

const client = () => createPublicClient({ transport: http(chain.rpcUrl), pollingInterval: 100 })
const faucetWallet = () => createWalletClient({ account: faucet, transport: http(chain.rpcUrl) })
const rpc = (method: string, params: unknown[] = []) => jsonRpc(chain.rpcUrl, method, params)

before(async () => {
  ;({ chain, devnet } = await devChain({ token: false }))
})

after(async () => {
  for (const b of bundlers) await b.stop()
  for (const b of bundlers) await b.idle()
  for (const s of servers) s.close()
  await chain?.stop()
})

/** A bundler (manual bundling) + its JSON-RPC URL. Each instance gets its own operator key. */
async function startBundler(key: Hex, config: Partial<BundlerConfig> = {}) {
  const b = new Bundler({ rpcUrl: chain.rpcUrl, privateKey: key, log: () => {}, config: { bundleIntervalMs: 0, beneficiary: BENEFICIARY, ...config } })
  await b.init()
  bundlers.push(b)
  const s = createBundlerServer(b)
  servers.push(s)
  await new Promise<void>((r) => s.listen(0, '127.0.0.1', r))
  return { b, url: `http://127.0.0.1:${(s.address() as { port: number }).port}` }
}

const signers = [{ kind: 1, x: pad(owner.address), y: pad('0x00') }] as const

async function counterfactual(salt: bigint, fund: bigint) {
  const saltHex = pad(toHex(salt))
  const address = await client().readContract({ address: FACTORY, abi: factoryAbi, functionName: 'getAddress', args: [signers, saltHex] })
  if (fund > 0n) await mined(await faucetWallet().sendTransaction({ chain: null, to: address, value: fund }))
  return { address, factoryData: encodeFunctionData({ abi: factoryAbi, functionName: 'createAccount', args: [signers, saltHex] }) }
}

async function deployed(salt: bigint): Promise<Address> {
  const { address } = await counterfactual(salt, 0n)
  await mined(await faucetWallet().writeContract({ chain: null, address: FACTORY, abi: factoryAbi, functionName: 'createAccount', args: [signers, pad(toHex(salt))] }))
  return address
}

async function deployAll(salts: bigint[]): Promise<Address[]> {
  const out: Address[] = []
  for (const s of salts) out.push(await deployed(s)) // sequential: one faucet nonce at a time
  return out
}

async function mined(hash: Hex) {
  const r = await client().waitForTransactionReceipt({ hash })
  assert.equal(r.status, 'success')
  return r
}

async function deployTogglePaymaster(): Promise<Address> {
  const r = await mined(await faucetWallet().deployContract({ chain: null, abi: togglePmAbi, bytecode: TOGGLE_PAYMASTER_BYTECODE }))
  const pm = r.contractAddress as Address
  await mined(await faucetWallet().writeContract({ chain: null, address: pm, abi: togglePmAbi, functionName: 'addStake', args: [86_400], value: parseEther('1') }))
  await mined(await faucetWallet().writeContract({ chain: null, address: pm, abi: togglePmAbi, functionName: 'deposit', value: parseEther('5') }))
  return pm
}

async function setBroken(pm: Address, broken: boolean, fees?: { maxFeePerGas: bigint; maxPriorityFeePerGas: bigint }) {
  return faucetWallet().writeContract({ chain: null, address: pm, abi: togglePmAbi, functionName: 'setBroken', args: [broken], ...fees })
}

async function fees() {
  const f = await client().estimateFeesPerGas()
  return { maxFeePerGas: f.maxFeePerGas * 2n, maxPriorityFeePerGas: f.maxPriorityFeePerGas }
}

const transfer = (wei: bigint) => encodeFunctionData({ abi: accountAbi, functionName: 'execute', args: [recipient, wei, '0x'] })

/** Estimates through the bundler (dummy signature), then signs with the owner key. */
async function signedOp(url: string, o: Partial<UserOperation> & { sender: Address }): Promise<UserOperation> {
  const base: UserOperation = {
    nonce: 0n, factory: null, factoryData: null, callData: '0x',
    callGasLimit: 0n, verificationGasLimit: 0n, preVerificationGas: 0n, ...(await fees()), signature: '0x',
    paymaster: null, paymasterVerificationGasLimit: null, paymasterPostOpGasLimit: null, paymasterData: null,
    ...o,
  }
  const est = await jsonRpc(url, 'eth_estimateUserOperationGas', [toRpcUserOp({ ...base, signature: toHex(new Uint8Array(65).fill(0x11)) }), ENTRY_POINT])
  assert.equal(est.error, undefined, JSON.stringify(est.error))
  const op = fromRpcUserOp({ ...toRpcUserOp(base), ...est.result })
  op.signature = await owner.sign({ hash: getUserOpHash(op, devnet.chainId) })
  return op
}

async function send(url: string, op: UserOperation) {
  return jsonRpc(url, 'eth_sendUserOperation', [toRpcUserOp(op), ENTRY_POINT])
}

describe('bundler end to end', () => {
  it('puts ops from several senders into ONE bundle and the operator does not lose money', async () => {
    const { b, url } = await startBundler(DEV_KEYS[ROLES.bundler])
    const pm = await deployTogglePaymaster()
    const ops: UserOperation[] = []
    for (const salt of [101n, 102n, 103n]) {
      const { address, factoryData } = await counterfactual(salt, parseEther('1'))
      ops.push(await signedOp(url, { sender: address, factory: FACTORY, factoryData, callData: transfer(parseEther('0.01')) }))
    }
    // A fourth sender whose gas is paid by a (staked) paymaster.
    ops.push(await signedOp(url, { sender: await deployed(104n), paymaster: pm, paymasterData: '0x', callData: transfer(0n) }))

    const hashes: Hex[] = []
    for (const op of ops) {
      const r = await send(url, op)
      assert.equal(r.error, undefined, JSON.stringify(r.error))
      hashes.push(r.result)
    }
    assert.equal((await jsonRpc(url, 'eth_getUserOperationReceipt', [hashes[0]])).result, null, 'pending: no receipt yet')
    assert.equal((await jsonRpc(url, 'eth_getUserOperationByHash', [hashes[0]])).result.transactionHash, null)

    const op0 = b.account.address
    const [opBefore, benBefore] = await Promise.all([client().getBalance({ address: op0 }), client().getBalance({ address: BENEFICIARY })])
    const txs = await b.bundleNow()
    assert.equal(txs.length, 1, 'one bundle transaction')
    await b.idle()

    const receipts = await Promise.all(hashes.map(async (h) => (await jsonRpc(url, 'eth_getUserOperationReceipt', [h])).result))
    for (const r of receipts) {
      assert.equal(r.success, true)
      assert.equal(r.receipt.transactionHash, txs[0])
      // Per-op log slice: no other op's UserOperationEvent leaks into it.
      assert.equal(r.logs.filter((l: { address: string }) => l.address.toLowerCase() === ENTRY_POINT.toLowerCase()).length, 0)
    }
    const [opAfter, benAfter] = await Promise.all([client().getBalance({ address: op0 }), client().getBalance({ address: BENEFICIARY })])
    const spent = opBefore - opAfter
    const collected = benAfter - benBefore
    assert.ok(spent > 0n)
    assert.ok(collected >= spent, `beneficiary got ${collected} < operator spent ${spent}`)
    assert.equal(b.metrics.get('bundler_bundles_included_total'), 1)
    assert.match(b.metricsText(), /bundler_ops_included_total 4/)
  })

  it('accepts a second op of the same sender while the first is pending and bundles both in order', async () => {
    const { b, url } = await startBundler(DEV_KEYS[ROLES.bundler])
    const { address, factoryData } = await counterfactual(201n, parseEther('1'))
    const first = await signedOp(url, { sender: address, factory: FACTORY, factoryData, callData: transfer(1n) })
    // The account is not deployed yet, so nonce 1 cannot be estimated alone: reuse the first op's limits.
    const second: UserOperation = { ...first, nonce: 1n, factory: null, factoryData: null, callData: transfer(2n) }
    second.preVerificationGas = b.preVerificationGas(second)
    second.signature = await owner.sign({ hash: getUserOpHash(second, devnet.chainId) })
    const gap: UserOperation = { ...second, nonce: 2n }
    gap.signature = await owner.sign({ hash: getUserOpHash(gap, devnet.chainId) })

    const r2early = await send(url, second)
    assert.ok(r2early.error, 'nonce 1 before nonce 0 is pending: rejected (AA25 in simulation)')
    const r1 = await send(url, first)
    assert.equal(r1.error, undefined, JSON.stringify(r1.error))
    const r3 = await send(url, gap)
    assert.equal(r3.error?.code, RpcCodes.invalidParams, 'nonce gap')
    const r2 = await send(url, second)
    assert.equal(r2.error, undefined, JSON.stringify(r2.error))
    const txs = await b.bundleNow()
    assert.equal(txs.length, 1)
    await b.idle()
    const a = (await jsonRpc(url, 'eth_getUserOperationReceipt', [r1.result])).result
    const c = (await jsonRpc(url, 'eth_getUserOperationReceipt', [r2.result])).result
    assert.equal(a.success && c.success, true)
    assert.equal(a.receipt.transactionHash, c.receipt.transactionHash)
  })

  it('throttles a paymaster whose ops stop validating after admission', async () => {
    const { b, url } = await startBundler(DEV_KEYS[9], {
      minInclusionRateDenominator: 1,
      throttlingSlack: 1,
      banSlack: 3,
      throttledEntityMempoolCount: 1,
    })
    const pm = await deployTogglePaymaster()
    const senders = await deployAll([301n, 302n, 303n, 304n])
    const opFor = (sender: Address) => signedOp(url, { sender, paymaster: pm, paymasterData: '0x' })

    const accepted = [await send(url, await opFor(senders[0]!)), await send(url, await opFor(senders[1]!))]
    for (const r of accepted) assert.equal(r.error, undefined, JSON.stringify(r.error))
    await mined(await setBroken(pm, true))
    assert.deepEqual(await b.bundleNow(), [], 'both ops fail re-simulation: nothing submitted')
    assert.equal(b.mempool.size, 0, 'invalidated ops dropped')
    assert.equal(b.reputation.status(pm), 'throttled')

    await mined(await setBroken(pm, false))
    const ok = await send(url, await opFor(senders[2]!))
    assert.equal(ok.error, undefined, 'a throttled paymaster may still have one op in the mempool')
    const refused = await send(url, await opFor(senders[3]!))
    assert.equal(refused.error?.code, RpcCodes.bannedOrThrottled, JSON.stringify(refused.error))
    assert.match(refused.error.message, /throttled/)
    // Inclusion improves its reputation again.
    await b.bundleNow()
    await b.idle()
    assert.equal((await jsonRpc(url, 'eth_getUserOperationReceipt', [ok.result])).result.success, true)
  })

  it('bans a paymaster that makes a bundle revert on chain (front-run invalidation)', async () => {
    const { b, url } = await startBundler(DEV_KEYS[9])
    const pm = await deployTogglePaymaster()
    const [s1, s2] = await deployAll([401n, 402n])
    const r = await send(url, await signedOp(url, { sender: s1!, paymaster: pm, paymasterData: '0x' }))
    assert.equal(r.error, undefined, JSON.stringify(r.error))

    await rpc('evm_setAutomine', [false])
    try {
      const [bundleTx] = await b.bundleNow()
      assert.ok(bundleTx)
      // The attacker's tx pays a higher tip, so it lands before the bundle in the same block.
      await setBroken(pm, true, { maxFeePerGas: parseGwei('200'), maxPriorityFeePerGas: parseGwei('100') })
      await rpc('evm_mine')
    } finally {
      await rpc('evm_setAutomine', [true])
    }
    await b.idle()
    assert.equal(b.metrics.get('bundler_bundles_failed_total', { stage: 'reverted' }), 1)
    assert.equal(b.reputation.status(pm), 'banned')
    assert.equal(b.mempool.size, 0)

    await mined(await setBroken(pm, false))
    const again = await send(url, await signedOp(url, { sender: s2!, paymaster: pm, paymasterData: '0x' }))
    assert.equal(again.error?.code, RpcCodes.bannedOrThrottled)
    assert.match(again.error.message, /banned/)
    const health = (await (await fetch(`${url}/health`)).json()) as { status: string }
    assert.equal(health.status, 'ok')
  })

  it('refuses unstaked paymasters and enforces the sponsorship policy (activation only)', async () => {
    const { b, url } = await startBundler(DEV_KEYS[ROLES.bundler])
    // Unstaked paymaster.
    const r0 = await mined(await faucetWallet().deployContract({ chain: null, abi: togglePmAbi, bytecode: TOGGLE_PAYMASTER_BYTECODE }))
    const unstaked = r0.contractAddress as Address
    await mined(await faucetWallet().writeContract({ chain: null, address: unstaked, abi: togglePmAbi, functionName: 'deposit', value: parseEther('1') }))
    const s = await deployed(501n)
    const u = await send(url, await signedOp(url, { sender: s, paymaster: unstaked, paymasterData: '0x' }))
    assert.equal(u.error?.code, RpcCodes.stakeTooLow)

    // Sponsorship policy on the XdevsPaymaster (open mode): only the activation op of an account is sponsored.
    const sponsored = { paymaster: '0x0000000000000000000000000000000000001007' as Address, paymasterData: devnet.sponsor as Hex }
    const notActivation = await send(url, await signedOp(url, { sender: s, ...sponsored }))
    assert.equal(notActivation.error?.code, RpcCodes.rejectedByPaymaster)
    assert.match(notActivation.error.message, /sponsorship policy/)
    const { address, factoryData } = await counterfactual(502n, 0n)
    const act = await send(url, await signedOp(url, { sender: address, factory: FACTORY, factoryData, ...sponsored }))
    assert.equal(act.error, undefined, JSON.stringify(act.error))
    await b.bundleNow()
    await b.idle()
    const more = await send(url, await signedOp(url, { sender: address, nonce: 1n, ...sponsored }))
    assert.match(String(more.error?.message), /already used its 1 sponsored/)
  })

  it('persists the mempool and reputation across a restart', async () => {
    const dataDir = mkdtempSync(join(tmpdir(), 'bundler-e2e-'))
    try {
      const first = await startBundler(DEV_KEYS[ROLES.bundler], { dataDir })
      const { address, factoryData } = await counterfactual(601n, parseEther('1'))
      const r = await send(first.url, await signedOp(first.url, { sender: address, factory: FACTORY, factoryData }))
      assert.equal(r.error, undefined, JSON.stringify(r.error))
      await first.b.stop()

      const second = await startBundler(DEV_KEYS[ROLES.bundler], { dataDir })
      assert.equal(second.b.mempool.size, 1, 'pending op restored')
      assert.equal(second.b.reputation.get(address).opsSeen, 1, 'reputation restored')
      assert.equal((await second.b.bundleNow()).length, 1)
      await second.b.idle()
      assert.equal((await jsonRpc(second.url, 'eth_getUserOperationReceipt', [r.result])).result.success, true)
    } finally {
      rmSync(dataDir, { recursive: true, force: true })
    }
  })
})

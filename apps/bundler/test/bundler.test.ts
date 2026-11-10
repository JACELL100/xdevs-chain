// Integration tests of the dev bundler against a real dev chain (xDEVS node, or anvil --hardfork osaka).
import assert from 'node:assert/strict'
import type { Server } from 'node:http'
import { after, before, describe, it } from 'node:test'
import {
  type Address,
  type Hex,
  concat,
  createPublicClient,
  createWalletClient,
  encodeFunctionData,
  http,
  pad,
  parseAbi,
  parseEther,
  toHex,
} from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { DEV_KEYS, ENTRY_POINT, PAYMASTER, ROLES } from '../scripts/devnet.ts'
import { entryPointAbi } from '../src/abi.ts'
import { Bundler } from '../src/bundler.ts'
import { RpcCodes } from '../src/errors.ts'
import { createBundlerServer } from '../src/server.ts'
import {
  PAYMASTER_SIG_MAGIC,
  type UserOperation,
  fromRpcUserOp,
  getUserOpHash,
  packUserOp,
  toRpcUserOp,
  unpackUserOp,
} from '../src/userop.ts'
import { devChain, jsonRpc } from './helpers.ts'

const FACTORY: Address = '0x0000000000000000000000000000000000001006'
const SIGNER = '(uint8 kind, bytes32 x, bytes32 y)'
const factoryAbi = parseAbi([
  `function getAddress(${SIGNER}[] signers, bytes32 salt) view returns (address)`,
  `function createAccount(${SIGNER}[] signers, bytes32 salt) returns (address)`,
])
const accountAbi = parseAbi(['function execute(address target, uint256 value, bytes data) returns (uint256)'])

let chain: Awaited<ReturnType<typeof devChain>>['chain']
let devnet: Awaited<ReturnType<typeof devChain>>['devnet']
let bundler: Bundler
let server: Server
let url: string

const owner = privateKeyToAccount(DEV_KEYS[ROLES.eoa7702])
const recipient = privateKeyToAccount(DEV_KEYS[ROLES.recipient]).address
const faucet = privateKeyToAccount(DEV_KEYS[ROLES.faucet])

before(async () => {
  ;({ chain, devnet } = await devChain({ token: false }))
  bundler = new Bundler({ rpcUrl: chain.rpcUrl, privateKey: DEV_KEYS[ROLES.bundler], log: () => {} })
  await bundler.init()
  server = createBundlerServer(bundler)
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r))
  url = `http://127.0.0.1:${(server.address() as { port: number }).port}`
})

after(async () => {
  await bundler?.idle()
  server?.close()
  await chain?.stop()
})

function client() {
  return createPublicClient({ transport: http(chain.rpcUrl), pollingInterval: 200 })
}

async function newAccount(salt: bigint, fund: bigint) {
  const signers = [{ kind: 1, x: pad(owner.address), y: pad('0x00') }] as const
  const saltHex = pad(toHex(salt))
  const address = await client().readContract({ address: FACTORY, abi: factoryAbi, functionName: 'getAddress', args: [signers, saltHex] })
  if (fund > 0n) {
    const w = createWalletClient({ account: faucet, transport: http(chain.rpcUrl) })
    await client().waitForTransactionReceipt({ hash: await w.sendTransaction({ chain: null, to: address, value: fund }) })
  }
  const factoryData = encodeFunctionData({ abi: factoryAbi, functionName: 'createAccount', args: [signers, saltHex] })
  return { address, factoryData }
}

async function fees() {
  const f = await client().estimateFeesPerGas()
  return { maxFeePerGas: f.maxFeePerGas * 2n, maxPriorityFeePerGas: f.maxPriorityFeePerGas }
}

async function estimateAndSign(op: UserOperation): Promise<UserOperation> {
  const est = await jsonRpc(url, 'eth_estimateUserOperationGas', [toRpcUserOp({ ...op, signature: toHex(new Uint8Array(65).fill(0x11)) }), ENTRY_POINT])
  assert.equal(est.error, undefined, JSON.stringify(est.error))
  const filled = fromRpcUserOp({ ...toRpcUserOp(op), ...est.result })
  const hash = getUserOpHash(filled, devnet.chainId)
  filled.signature = await owner.sign({ hash })
  return filled
}

async function waitReceipt(hash: Hex) {
  for (let i = 0; i < 200; i++) {
    const r = await jsonRpc(url, 'eth_getUserOperationReceipt', [hash])
    if (r.result) return r.result
    await new Promise((res) => setTimeout(res, 250))
  }
  throw new Error('no receipt')
}

describe(`dev bundler`, () => {
  it('serves chain id and the v0.9 EntryPoint', async () => {
    assert.equal((await jsonRpc(url, 'eth_chainId')).result, toHex(devnet.chainId))
    assert.deepEqual((await jsonRpc(url, 'eth_supportedEntryPoints')).result, [ENTRY_POINT])
    assert.equal((await jsonRpc(url, 'eth_foo')).error.code, RpcCodes.methodNotFound)
  })

  it('computes the userOpHash exactly like EntryPoint.getUserOpHash (with and without paymaster signature)', async () => {
    const base: UserOperation = {
      sender: owner.address, nonce: 7n, factory: FACTORY, factoryData: '0x1234', callData: '0xdeadbeef',
      callGasLimit: 100_000n, verificationGasLimit: 200_000n, preVerificationGas: 50_000n,
      maxFeePerGas: 3_000_000_000n, maxPriorityFeePerGas: 1_000_000_000n,
      paymaster: PAYMASTER, paymasterVerificationGasLimit: 60_000n, paymasterPostOpGasLimit: 40_000n,
      paymasterData: devnet.sponsor, signature: '0x',
    }
    const sigSuffix = concat([toHex(new Uint8Array(65).fill(7)), '0x0041', PAYMASTER_SIG_MAGIC])
    for (const op of [base, { ...base, paymaster: null, factory: null }, { ...base, paymasterData: concat([devnet.sponsor, toHex(new Uint8Array(12)), sigSuffix]) }]) {
      const onChain = await client().readContract({ address: ENTRY_POINT, abi: entryPointAbi, functionName: 'getUserOpHash', args: [packUserOp(op)] })
      assert.equal(getUserOpHash(op, devnet.chainId), onChain)
      assert.deepEqual(toRpcUserOp(unpackUserOp(packUserOp(op))), toRpcUserOp(op))
    }
  })

  it('deploys a counterfactual account and executes its first op (account pays gas)', async () => {
    const { address, factoryData } = await newAccount(1n, parseEther('1'))
    const op = await estimateAndSign({
      sender: address, nonce: 0n, factory: FACTORY, factoryData,
      callData: encodeFunctionData({ abi: accountAbi, functionName: 'execute', args: [recipient, parseEther('0.1'), '0x'] }),
      callGasLimit: 0n, verificationGasLimit: 0n, preVerificationGas: 0n, ...(await fees()), signature: '0x',
    })
    const before = await client().getBalance({ address: recipient })
    const sent = await jsonRpc(url, 'eth_sendUserOperation', [toRpcUserOp(op), ENTRY_POINT])
    assert.equal(sent.error, undefined, JSON.stringify(sent.error))
    const receipt = await waitReceipt(sent.result)
    assert.equal(receipt.success, true)
    assert.equal(receipt.sender.toLowerCase(), address.toLowerCase())
    assert.equal((await client().getBalance({ address: recipient })) - before, parseEther('0.1'))
    const byHash = await jsonRpc(url, 'eth_getUserOperationByHash', [sent.result])
    assert.equal(byHash.result.userOperation.sender.toLowerCase(), address.toLowerCase())
    assert.equal(byHash.result.transactionHash, receipt.receipt.transactionHash)

    // A fresh bundler instance (no memory) finds the op through EntryPoint logs.
    const fresh = new Bundler({ rpcUrl: chain.rpcUrl, privateKey: DEV_KEYS[ROLES.bundler], log: () => {} })
    await fresh.init()
    const r2 = (await fresh.handle('eth_getUserOperationReceipt', [sent.result])) as { success: boolean }
    assert.equal(r2.success, true)
    const o2 = (await fresh.handle('eth_getUserOperationByHash', [sent.result])) as { userOperation: { nonce: Hex } }
    assert.equal(o2.userOperation.nonce, '0x0')
  })

  it('sponsors a deployment through the XdevsPaymaster (account holds no tXDS)', async () => {
    const { address, factoryData } = await newAccount(2n, 0n)
    const op = await estimateAndSign({
      sender: address, nonce: 0n, factory: FACTORY, factoryData, callData: '0x',
      callGasLimit: 0n, verificationGasLimit: 0n, preVerificationGas: 0n, ...(await fees()),
      paymaster: PAYMASTER, paymasterData: devnet.sponsor, signature: '0x',
    })
    assert.ok((op.paymasterVerificationGasLimit ?? 0n) > 0n)
    const sent = await jsonRpc(url, 'eth_sendUserOperation', [toRpcUserOp(op), ENTRY_POINT])
    assert.equal(sent.error, undefined, JSON.stringify(sent.error))
    const receipt = await waitReceipt(sent.result)
    assert.equal(receipt.success, true)
    assert.equal(receipt.paymaster.toLowerCase(), PAYMASTER.toLowerCase())
    assert.notEqual(await client().getCode({ address }), undefined)
    assert.equal(await client().getBalance({ address }), 0n)
  })

  it('rejects a bad signature (AA24 -> -32507), a wrong EntryPoint, and reports execution reverts in estimation', async () => {
    const { address, factoryData } = await newAccount(3n, parseEther('1'))
    const op = await estimateAndSign({
      sender: address, nonce: 0n, factory: FACTORY, factoryData, callData: '0x',
      callGasLimit: 0n, verificationGasLimit: 0n, preVerificationGas: 0n, ...(await fees()), signature: '0x',
    })
    const bad = { ...op, signature: await privateKeyToAccount(DEV_KEYS[ROLES.spender]).sign({ hash: getUserOpHash(op, devnet.chainId) }) }
    const r = await jsonRpc(url, 'eth_sendUserOperation', [toRpcUserOp(bad), ENTRY_POINT])
    assert.equal(r.error?.code, RpcCodes.invalidSignature, JSON.stringify(r.error))
    assert.match(r.error.message, /AA24/)
    const r2 = await jsonRpc(url, 'eth_sendUserOperation', [toRpcUserOp(op), PAYMASTER])
    assert.equal(r2.error?.code, RpcCodes.invalidParams)
    // execute() of a call that reverts: sending 1000 tXDS from an account holding 1.
    const r3 = await jsonRpc(url, 'eth_estimateUserOperationGas', [
      toRpcUserOp({ ...op, callData: encodeFunctionData({ abi: accountAbi, functionName: 'execute', args: [recipient, parseEther('1000'), '0x'] }), signature: toHex(new Uint8Array(65).fill(1)) }),
      ENTRY_POINT,
    ])
    assert.equal(r3.error?.code, RpcCodes.executionReverted, JSON.stringify(r3))
  })
})

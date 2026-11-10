// Phase 4 exit criteria (plan §27), headless, on a real dev chain (the xDEVS node when its binary is found, else
// anvil --hardfork osaka with the dev genesis alloc) with the dev bundler and the XdevsPaymaster predeploy:
//   (a) a new user creates an account with ONLY a passkey (software P-256 key standing in for the authenticator),
//       it is deployed by a sponsored UserOperation, and it receives faucet funds;
//   (b) "stolen key": the thief's large transfer is QUEUED by the Guardian Vault, the owner sees it in the queue
//       (alert), and a guardian quorum (2-of-3) cancels it within the window;
//   (c) the pre-sign preview flags an unlimited approval with a clear human-readable warning (advice, not fact).
// Plus: approvals manager (list + revoke) and the EIP-7702 dev path with the XIP-0001 EOA lock.
import assert from 'node:assert/strict'
import type { Server } from 'node:http'
import { after, before, describe, it } from 'node:test'
import {
  type Address,
  type Hex,
  type PublicClient,
  createPublicClient,
  createWalletClient,
  encodeFunctionData,
  http,
  maxUint256,
  parseEther,
  toHex,
} from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { DEV_KEYS, ROLES } from '../../bundler/scripts/devnet.ts'
import { startBundler } from '../../bundler/src/index.ts'
import { devChain } from '../../bundler/test/helpers.ts'
import { accountAbi, erc20Abi, nftAbi, permit2Abi } from '../src/lib/abi.ts'
import {
  PERMIT2_EXPIRING,
  buildInitParams,
  counterfactualAddress,
  executeCallData,
  factoryData,
  readAccountState,
  readEoaLock,
  recommendedPolicy,
  saltFrom,
  SMART_ACCOUNT_FACTORY,
} from '../src/lib/account.ts'
import { listApprovals, revokeAllCalls } from '../src/lib/approvals.ts'
import { enableEoaLock, upgradeEoa } from '../src/lib/eip7702.ts'
import { previewTransaction } from '../src/lib/preview.ts'
import { checkRecipient, resolveRecipient } from '../src/lib/recipient.ts'
import { sendUserOp } from '../src/lib/send.ts'
import { SoftPasskey, passkeySignerStruct } from '../src/lib/signers.ts'
import { checkVaultOutcome, listVaultQueue, setVaultPolicyCallData } from '../src/lib/vault.ts'

let chain: Awaited<ReturnType<typeof devChain>>['chain']
let devnet: Awaited<ReturnType<typeof devChain>>['devnet']
let bundlerServer: Server
let bundlerUrl: string
let client: PublicClient

const addr = (role: keyof typeof ROLES) => privateKeyToAccount(DEV_KEYS[ROLES[role]]).address
const walletOf = (role: keyof typeof ROLES) =>
  createWalletClient({ account: privateKeyToAccount(DEV_KEYS[ROLES[role]]), transport: http(chain.rpcUrl) })

before(async () => {
  ;({ chain, devnet } = await devChain())
  client = createPublicClient({ transport: http(chain.rpcUrl), pollingInterval: 200 })
  const b = await startBundler({ BUNDLER_PRIVATE_KEY: DEV_KEYS[ROLES.bundler], RPC_URL: chain.rpcUrl, PORT: '0', QUIET: '1' })
  bundlerServer = b.server
  bundlerUrl = b.url
})

after(async () => {
  bundlerServer?.close()
  await chain?.stop()
})

async function txAndWait(role: keyof typeof ROLES, to: Address, data: Hex = '0x', value = 0n) {
  const hash = await walletOf(role).sendTransaction({ chain: null, to, data, value })
  const r = await client.waitForTransactionReceipt({ hash })
  assert.equal(r.status, 'success')
  return r
}

const now = async () => (await client.getBlock({ blockTag: 'latest' })).timestamp

describe(`Phase 4 exit criteria (headless)`, () => {
  const passkey = SoftPasskey.generate()
  let account: Address
  const guardians = [addr('guardian1'), addr('guardian2'), addr('guardian3')]

  it('(a) passkey-only sign-up: counterfactual address, sponsored deployment, faucet funds', async () => {
    const pk = await passkey
    const params = buildInitParams({
      signers: [passkeySignerStruct(pk.publicKey)],
      guardians,
      guardianThreshold: 2,
      policy: recommendedPolicy({ nativeThreshold: parseEther('1'), dailyLimit: parseEther('3'), delay: 3_600n }),
    })
    const salt = saltFrom(BigInt(Date.now()))
    account = await counterfactualAddress(client, params, salt)
    assert.equal(await client.getCode({ address: account }), undefined)

    // First UserOperation deploys the account; gas paid by the XdevsPaymaster sponsor. No seed phrase anywhere.
    const sent = await sendUserOp({
      client, bundlerUrl, chainId: devnet.chainId, sender: account, callData: '0x', signer: pk.signer(),
      factory: { address: SMART_ACCOUNT_FACTORY, data: factoryData(params, salt) }, sponsor: devnet.sponsor,
    })
    assert.equal(sent.receipt.success, true)
    const st = await readAccountState(client, account)
    assert.equal(st.deployed, true)
    assert.equal(st.balance, 0n, 'deployment was fully sponsored')
    assert.equal(st.policy.enabled, true)
    assert.deepEqual(st.guardians, guardians)
    assert.equal(st.signers[0].kind, 2, 'passkey signer')

    // Faucet funds (the faucet app sends a plain tXDS transfer; the dev faucet key stands in for it).
    await txAndWait('faucet', account, '0x', parseEther('10'))
    assert.equal(await client.getBalance({ address: account }), parseEther('10'))

    // A small payment is instant (below the vault threshold); the account pays its own gas now.
    const call = { target: addr('recipient'), value: parseEther('0.5'), data: '0x' as Hex }
    const outcome = await checkVaultOutcome(client, await readAccountState(client, account), [call])
    assert.equal(outcome.kind, 'instant', outcome.reason)
    const before = await client.getBalance({ address: addr('recipient') })
    const pay = await sendUserOp({ client, bundlerUrl, chainId: devnet.chainId, sender: account, callData: executeCallData(call), signer: pk.signer() })
    assert.deepEqual(pay.queuedIds, [])
    assert.equal((await client.getBalance({ address: addr('recipient') })) - before, parseEther('0.5'))
  })

  it('(b) stolen key: the large transfer is queued, the owner sees it, a 2-of-3 guardian quorum cancels it', async () => {
    const thief = (await passkey).signer('stolen passkey')
    const attacker = addr('spender')
    const loot = { target: attacker, value: parseEther('8'), data: '0x' as Hex }
    const st = await readAccountState(client, account)
    const outcome = await checkVaultOutcome(client, st, [loot])
    assert.equal(outcome.kind, 'queued', outcome.reason)

    const attackerBefore = await client.getBalance({ address: attacker })
    const sent = await sendUserOp({ client, bundlerUrl, chainId: devnet.chainId, sender: account, callData: executeCallData(loot), signer: thief })
    assert.equal(sent.queuedIds.length, 1)
    const id = sent.queuedIds[0]
    assert.equal(await client.getBalance({ address: attacker }), attackerBefore, 'nothing left the account')

    // The thief also tries to switch the vault off: policy changes are queued too.
    const off = await sendUserOp({
      client, bundlerUrl, chainId: devnet.chainId, sender: account, signer: thief,
      callData: setVaultPolicyCallData({ ...st.policy, enabled: false }),
    })
    assert.equal(off.queuedIds.length, 1)
    assert.equal((await readAccountState(client, account)).policy.enabled, true)

    // Alert: the wallet's queue view shows the pending transfer with its countdown.
    let queue = await listVaultQueue(client, account, await now())
    const q = queue.find((a) => a.id === id)
    assert.ok(q)
    assert.equal(q.status, 'queued')
    assert.equal(q.totalValue, parseEther('8'))
    assert.ok(q.executeAfter > (await now()))

    // Guardians cancel: one vote is not enough, the second reaches the threshold.
    await txAndWait('guardian1', account, encodeFunctionData({ abi: accountAbi, functionName: 'cancel', args: [id] }))
    queue = await listVaultQueue(client, account, await now())
    assert.equal(queue.find((a) => a.id === id)?.status, 'queued')
    assert.equal(queue.find((a) => a.id === id)?.cancelVotes, 1)
    await txAndWait('guardian2', account, encodeFunctionData({ abi: accountAbi, functionName: 'cancel', args: [id] }))
    queue = await listVaultQueue(client, account, await now())
    assert.equal(queue.find((a) => a.id === id)?.status, 'cancelled')

    // A cancelled action can never execute.
    await assert.rejects(
      client.simulateContract({ account: attacker, address: account, abi: accountAbi, functionName: 'execute', args: [id] }),
      /ActionNotQueued/,
    )
    // Guardians also cancel the "vault off" change.
    for (const g of ['guardian1', 'guardian3'] as const) {
      await txAndWait(g, account, encodeFunctionData({ abi: accountAbi, functionName: 'cancel', args: [off.queuedIds[0]] }))
    }
    queue = await listVaultQueue(client, account, await now())
    assert.equal(queue.find((a) => a.id === off.queuedIds[0])?.status, 'cancelled')
    assert.equal(await client.getBalance({ address: attacker }), attackerBefore)
  })

  it('(c) the preview flags an unlimited approval (advice, not fact) and the vault would block it', async () => {
    const data = encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [addr('spender'), maxUint256] })
    const tx = { from: account, to: devnet.token, value: 0n, data }
    const p = await previewTransaction(client, tx)
    const w = p.warnings.find((x) => x.code === 'UNLIMITED_APPROVAL')
    assert.ok(w, JSON.stringify(p.warnings))
    assert.equal(w.severity, 'high')
    assert.match(w.message, /ALL your tUSD/)
    assert.match(p.disclaimer, /Advice, not fact/)
    assert.equal(p.approvals[0].amount, 'unlimited')
    const outcome = await checkVaultOutcome(client, await readAccountState(client, account), [{ target: devnet.token, value: 0n, data }])
    assert.equal(outcome.kind, 'rejected')
    assert.match(outcome.reason, /blocked by your allowance policy/)

    const nft = await previewTransaction(client, { from: account, to: devnet.token, value: 0n, data: encodeFunctionData({ abi: nftAbi, functionName: 'setApprovalForAll', args: [addr('spender'), true] }) })
    assert.ok(nft.warnings.some((x) => x.code === 'SET_APPROVAL_FOR_ALL' && x.severity === 'high'))
    const limited = await previewTransaction(client, { ...tx, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [addr('spender'), 5n * 10n ** 18n] }) })
    assert.ok(!limited.warnings.some((x) => x.code === 'UNLIMITED_APPROVAL'))
  })

  it('send checks: first-time and look-alike recipients, XNS names', async () => {
    const history = [addr('recipient')]
    const first = await checkRecipient(client, account, addr('guardian1'), history)
    assert.equal(first.firstTime, true)
    const r = addr('recipient')
    const poison = `${r.slice(0, 6)}${'0'.repeat(32)}${r.slice(-4)}` as Address
    const look = await checkRecipient(client, account, poison, history)
    assert.equal(look.lookAlikeOf.length, 1)
    const p = await previewTransaction(client, { from: account, to: poison, value: parseEther('0.1'), data: '0x' }, { recipient: look })
    assert.ok(p.warnings.some((w) => w.code === 'LOOKALIKE_RECIPIENT' && w.severity === 'high'))
    await assert.rejects(resolveRecipient(client, 'nobody-here-yet.xdevs'), /not registered/)
    await assert.rejects(resolveRecipient(client, 'PAYPAL.xdevs '), /./) // fine: normalizes; then not registered
    await assert.rejects(resolveRecipient(client, 'pаypal'), /./) // Cyrillic а: rejected locally
  })

  it('approvals manager: lists ERC-20 / XDRC-20 / Permit2 approvals and revokes them all', async () => {
    const owner = addr('faucet')
    await txAndWait('faucet', devnet.token, encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [addr('spender'), maxUint256] }))
    const exp = Number(await now()) + 86_400
    await txAndWait('faucet', PERMIT2_EXPIRING, encodeFunctionData({ abi: permit2Abi, functionName: 'approve', args: [devnet.token, addr('recipient'), 1000n, exp] }))
    let items = await listApprovals(client, owner, { knownTokens: [devnet.token] })
    assert.equal(items.length, 2, JSON.stringify(items, (_k, v) => (typeof v === 'bigint' ? String(v) : v)))
    const xdrc = items.find((i) => i.kind === 'xdrc20')
    assert.ok(xdrc?.expiry && xdrc.expiry > Number(await now()), 'XDRC-20 legacy approve still expires')
    assert.ok(items.some((i) => i.kind === 'permit2' && i.amount === 1000n))
    for (const c of revokeAllCalls(items, false)) await txAndWait('faucet', c.target, c.data)
    items = await listApprovals(client, owner, { knownTokens: [devnet.token] })
    assert.deepEqual(items, [])
  })

  it('EIP-7702 dev path: upgrade an EOA in place, enable the XIP-0001 EOA lock', async () => {
    const key = DEV_KEYS[ROLES.eoa7702]
    const eoa = privateKeyToAccount(key).address
    const pk = await SoftPasskey.generate()
    const params = buildInitParams({ signers: [passkeySignerStruct(pk.publicKey)], guardians, guardianThreshold: 2, policy: recommendedPolicy({ delay: 3_600n }) })
    await upgradeEoa(client, chain.rpcUrl, key, params)
    let st = await readAccountState(client, eoa)
    assert.equal(st.is7702, true)
    assert.equal(st.policy.enabled, true)
    assert.equal(st.eoaLock?.status, 'unlocked')
    await enableEoaLock(client, chain.rpcUrl, key)
    st = await readAccountState(client, eoa)
    assert.equal(st.eoaLock?.status, 'locked')
    const lock = await readEoaLock(client, eoa)
    if (chain.kind === 'xdevs') {
      assert.equal(lock.source, 'node')
      assert.equal(lock.protocolActive, true)
      // The node now refuses a plain transfer signed by the EOA key (XIP-0001, -32003).
      await assert.rejects(
        walletOf('eoa7702').sendTransaction({ chain: null, to: addr('spender'), value: parseEther('1') }),
        /XIP0001|xip|-32003|not allowed|locked/i,
      )
    }
    // The passkey still controls the account through the bundler (UserOps never come from the EOA).
    const pay = await sendUserOp({
      client, bundlerUrl, chainId: devnet.chainId, sender: eoa, signer: pk.signer(),
      callData: executeCallData({ target: addr('recipient'), value: parseEther('0.1'), data: '0x' }),
    })
    assert.deepEqual(pay.queuedIds, [])
  })
})

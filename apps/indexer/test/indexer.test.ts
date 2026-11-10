import { test } from 'node:test'
import assert from 'node:assert/strict'
import { keccak256, toHex as vHex } from 'viem'
import { xnsNamehash } from '@xdevs/sdk'
import { Indexer } from '../src/indexer.ts'
import { erc20Events, wxdsEvents } from '../src/decode.ts'
import { accountEvents, permit2Events, vaultEvents, xdrc20Events, xnsEvents } from '../src/abi.generated.ts'
import { RpcError } from '../src/rpc.ts'
import { addr, cfg, FakeChain, freshDb, log, silent } from './helpers.ts'
import type { Db } from '../src/db.ts'

const ALICE = addr('0xa11ce00000000000000000000000000000000001')
const BOB = addr('0xb0b0000000000000000000000000000000000002')
// Real counterparty and a look-alike (same first 4 / last 4 hex chars).
const REAL = addr('0x1234aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678')
const FAKE = addr('0x1234bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb5678')
const TOKEN = addr('0x70c0000000000000000000000000000000000001')
const XTOKEN = addr('0x70c0000000000000000000000000000000000002')
const SCAM = addr('0x5ca0000000000000000000000000000000000003')
const ACCOUNT = addr('0xacc0000000000000000000000000000000000004')
const PERMIT2 = addr('0x0000000000000000000000000000000000001008')
const XNS = addr('0x0000000000000000000000000000000000001004')
const WXDS = addr('0x0000000000000000000000000000000000001009')

const one = async (db: Db, sql: string, params: unknown[] = []) => (await db.query<Record<string, unknown>>(sql, params)).rows[0]!
const all = async (db: Db, sql: string, params: unknown[] = []) => (await db.query<Record<string, unknown>>(sql, params)).rows

async function syncAll(ix: Indexer) {
  while ((await ix.step()) > 0) {
    /* until caught up */
  }
}

function setup(chain = new FakeChain(), overrides = {}) {
  return freshDb().then((db) => ({ db, chain, ix: new Indexer(db, chain, cfg(overrides), silent) }))
}

test('indexes blocks, transactions, fees and addresses; idempotent and resumable', async () => {
  const { db, chain, ix } = await setup()
  chain.tokens.set(TOKEN, { name: 'Dollar X', symbol: 'USDX', decimals: 6 })
  chain.addBlock([{ from: ALICE, to: BOB, value: 10n ** 18n }])
  chain.addBlock([{ from: ALICE, to: null, contractAddress: TOKEN, input: '0x6080' }])
  chain.addBlock([{ from: ALICE, to: TOKEN, input: '0xa9059cbb', logs: [log(erc20Events, 'Transfer', { from: ALICE, to: BOB, value: 5_000_000n }, TOKEN)] }])
  for (let i = 0; i < 4; i++) chain.addBlock([])

  assert.equal(await ix.step(), 5, 'first batch: blocks 0..4')
  assert.equal((await ix.cursor())!.number, 4)
  // A new process resumes from the cursor.
  const ix2 = new Indexer(db, chain, cfg(), silent)
  await syncAll(ix2)
  assert.equal((await ix2.cursor())!.number, 7)
  assert.equal(await ix2.step(), 0)

  assert.equal(Number((await one(db, 'SELECT count(*) AS n FROM blocks')).n), 8)
  const tx = await one(db, 'SELECT * FROM transactions WHERE block_number = 1')
  // gasUsed 21000 × egp 2 gwei; base fee 1 gwei → 8000 bps of base burned on an xDEVS node.
  assert.equal(tx.fee, String(21_000n * 2_000_000_000n))
  assert.equal(tx.burned, String((21_000n * 1_000_000_000n * 8000n) / 10_000n))
  assert.equal(tx.treasury, String((21_000n * 1_000_000_000n * 2000n) / 10_000n))
  assert.equal(tx.priority_fee, String(21_000n * 1_000_000_000n))

  const alice = await one(db, 'SELECT * FROM addresses WHERE address = $1', [ALICE])
  assert.equal(Number(alice.tx_count), 3)
  assert.equal(Number(alice.first_seen_block), 1)
  assert.equal((await one(db, 'SELECT is_contract FROM addresses WHERE address = $1', [TOKEN])).is_contract, true)
  const tok = await one(db, 'SELECT * FROM tokens WHERE address = $1', [TOKEN])
  assert.equal(tok.symbol, 'USDX')
  assert.equal(Number(tok.decimals), 6)

  const transfers = await all(db, 'SELECT kind, token, value, spam FROM token_transfers ORDER BY block_number, seq')
  assert.deepEqual(transfers.map((t) => [t.kind, t.token, t.value, t.spam]), [
    ['native', null, String(10n ** 18n), false],
    ['erc20', TOKEN, '5000000', false],
  ])
  const stats = await one(db, 'SELECT * FROM chain_stats')
  assert.equal(Number(stats.tx_count), 3)
  assert.equal(Number(stats.transfer_count), 2)
  assert.equal(Number(stats.address_count), Number((await one(db, 'SELECT count(*) AS n FROM addresses')).n))

  // Re-running a batch that is already stored changes nothing (manual cursor rewind).
  await db.query(`UPDATE indexer_cursor SET block_number = 2, block_hash = (SELECT hash FROM blocks WHERE number = 2)`)
  await syncAll(ix2)
  assert.equal(Number((await one(db, 'SELECT tx_count FROM addresses WHERE address = $1', [ALICE])).tx_count), 3)
  assert.equal(Number((await one(db, 'SELECT count(*) AS n FROM token_transfers')).n), 2)
  await db.close()
})

test('address poisoning: zero-value transferFrom and dust from a look-alike are flagged, never deleted', async () => {
  const { db, chain, ix } = await setup(new FakeChain(), { knownTokens: [TOKEN] })
  chain.tokens.set(TOKEN, { name: 'Dollar X', symbol: 'USDX', decimals: 6 })
  chain.tokens.set(SCAM, { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 })
  // History: Alice pays REAL.
  chain.addBlock([{ from: ALICE, to: TOKEN, input: '0xa9059cbb', logs: [log(erc20Events, 'Transfer', { from: ALICE, to: REAL, value: 100_000_000n }, TOKEN)] }])
  chain.addBlock([{ from: ALICE, to: REAL, value: 2n * 10n ** 18n }])
  // Attacker (FAKE) calls transferFrom(alice, FAKE, 0): a Transfer event "from Alice" with value 0.
  chain.addBlock([{ from: FAKE, to: TOKEN, input: '0x23b872dd', logs: [log(erc20Events, 'Transfer', { from: ALICE, to: FAKE, value: 0n }, TOKEN)] }])
  // Attacker sends native dust to Alice.
  chain.addBlock([{ from: FAKE, to: ALICE, value: 1000n }])
  // A fake "tXDS" token airdrop.
  chain.addBlock([{ from: SCAM, to: SCAM, input: '0x01', logs: [log(erc20Events, 'Transfer', { from: SCAM, to: ALICE, value: 10n ** 21n }, SCAM)] }])
  await syncAll(ix)

  const rows = await all(db, 'SELECT block_number, kind, from_address, to_address, spam, spam_reasons, look_alike_of, tx_from FROM token_transfers ORDER BY block_number, seq')
  const byBlock = (n: number) => rows.find((r) => Number(r.block_number) === n)!
  assert.equal(byBlock(1).spam, false)
  assert.equal(byBlock(2).spam, false)
  assert.equal(byBlock(3).spam, true)
  assert.deepEqual(byBlock(3).spam_reasons, ['zero-value', 'look-alike'])
  assert.equal(byBlock(3).look_alike_of, REAL)
  assert.equal(byBlock(3).tx_from, FAKE, 'the poisoning tx was sent by the attacker')
  assert.equal(byBlock(4).spam, true)
  assert.deepEqual(byBlock(4).spam_reasons, ['dust', 'look-alike'])
  assert.deepEqual(byBlock(5).spam_reasons, ['unknown-token', 'impersonating-token'])
  const dustTx = await one(db, 'SELECT spam, spam_reasons FROM transactions WHERE block_number = 4')
  assert.equal(dustTx.spam, true)
  // Spam is never used as history: FAKE is not a counterparty of Alice.
  assert.equal((await all(db, 'SELECT 1 FROM counterparties WHERE owner = $1 AND counterparty = $2', [ALICE, FAKE])).length, 0)
  await db.close()
})

test('approvals: ERC-20 (never expires), XDRC-20 with expiry and revokeAll, Permit2Expiring', async () => {
  const { db, chain, ix } = await setup()
  chain.tokens.set(TOKEN, { name: 'Dollar X', symbol: 'USDX', decimals: 6 })
  chain.tokens.set(XTOKEN, { name: 'Safe X', symbol: 'SAFX', decimals: 18 })
  const max = 2n ** 256n - 1n
  chain.addBlock([{ from: ALICE, to: TOKEN, input: '0x095ea7b3', logs: [log(erc20Events, 'Approval', { owner: ALICE, spender: BOB, value: max }, TOKEN)] }])
  chain.addBlock([
    {
      from: ALICE,
      to: XTOKEN,
      input: '0x095ea7b3',
      logs: [
        log(erc20Events, 'Approval', { owner: ALICE, spender: BOB, value: 50n }, XTOKEN),
        log(xdrc20Events, 'ApprovalWithExpiry', { owner: ALICE, spender: BOB, value: 50n, expiry: 1_900_000_000n }, XTOKEN),
      ],
    },
  ])
  chain.addBlock([{ from: ALICE, to: PERMIT2, input: '0x01', logs: [log(permit2Events, 'Approval', { owner: ALICE, token: TOKEN, spender: REAL, amount: 2n ** 160n - 1n, expiration: 1_900_000_000 }, PERMIT2)] }])
  // A Permit2-shaped event from some other contract must be ignored.
  chain.addBlock([{ from: ALICE, to: SCAM, input: '0x01', logs: [log(permit2Events, 'Approval', { owner: ALICE, token: TOKEN, spender: FAKE, amount: 1n, expiration: 1_900_000_000 }, SCAM)] }])
  await syncAll(ix)

  let rows = await all(db, 'SELECT scope, kind, token, spender, amount, expiry, revoked FROM approvals ORDER BY block_number')
  assert.deepEqual(rows.map((r) => [r.scope, r.kind, r.token, r.amount, r.expiry, r.revoked]), [
    ['token', 'erc20', TOKEN, max.toString(), null, false],
    ['token', 'xdrc20', XTOKEN, '50', 1_900_000_000, false],
    ['permit2', 'permit2', TOKEN, (2n ** 160n - 1n).toString(), 1_900_000_000, false],
  ])
  assert.equal((await one(db, 'SELECT is_xdrc20 FROM tokens WHERE address = $1', [XTOKEN])).is_xdrc20, true)

  // Later XDRC-20 approvals: the plain Approval twin must not wipe the expiry once the token is known XDRC-20.
  chain.addBlock([
    {
      from: ALICE,
      to: XTOKEN,
      input: '0x095ea7b3',
      logs: [
        log(erc20Events, 'Approval', { owner: ALICE, spender: REAL, value: 7n }, XTOKEN),
        log(xdrc20Events, 'ApprovalWithExpiry', { owner: ALICE, spender: REAL, value: 7n, expiry: 1_950_000_000n }, XTOKEN),
      ],
    },
  ])
  chain.addBlock([{ from: ALICE, to: XTOKEN, input: '0x01', logs: [log(xdrc20Events, 'AllApprovalsRevoked', { owner: ALICE, newEpoch: 1 }, XTOKEN)] }])
  chain.addBlock([{ from: ALICE, to: PERMIT2, input: '0x01', logs: [log(permit2Events, 'AllRevoked', { owner: ALICE, newEpoch: 1 }, PERMIT2)] }])
  await syncAll(ix)
  rows = await all(db, 'SELECT scope, token, spender, expiry, revoked FROM approvals ORDER BY block_number, log_index')
  assert.deepEqual(rows.map((r) => [r.scope, r.token, r.spender, r.revoked]), [
    ['token', TOKEN, BOB, false],
    ['token', XTOKEN, BOB, true],
    ['permit2', TOKEN, REAL, true],
    ['token', XTOKEN, REAL, true],
  ])
  assert.equal(rows[3]!.expiry, 1_950_000_000)
  await db.close()
})

test('XNS names, reverse records and spoofed registry events', async () => {
  const { db, chain, ix } = await setup()
  const labelhash = keccak256(vHex('alice'))
  const node = xnsNamehash('alice.xdevs')
  const reverseNode = keccak256(vHex('reverse-node-of-alice'))
  chain.addBlock([
    {
      from: ALICE,
      to: XNS,
      input: '0x01',
      logs: [
        log(xnsEvents, 'NameRegistered', { label: 'alice', labelhash, owner: ALICE, expires: 1_900_000_000n, fee: 0n }, XNS),
        log(xnsEvents, 'ReverseClaimed', { addr: ALICE, node: reverseNode }, XNS),
        log(xnsEvents, 'NameChanged', { node: reverseNode, name: 'alice.xdevs' }, XNS),
      ],
    },
  ])
  chain.addBlock([{ from: SCAM, to: SCAM, input: '0x01', logs: [log(xnsEvents, 'NameRegistered', { label: 'paypal', labelhash: keccak256(vHex('paypal')), owner: SCAM, expires: 1n, fee: 0n }, SCAM)] }])
  chain.addBlock([{ from: ALICE, to: XNS, input: '0x01', logs: [log(xnsEvents, 'AddrChanged', { node, a: BOB }, XNS), log(xnsEvents, 'NameRenewed', { label: 'alice', labelhash, expires: 1_950_000_000n, fee: 0n }, XNS)] }])
  await syncAll(ix)
  const names = await all(db, 'SELECT * FROM xns_names')
  assert.equal(names.length, 1, 'events from other contracts are ignored')
  assert.equal(names[0]!.name, 'alice.xdevs')
  assert.equal(names[0]!.node, node)
  assert.equal(names[0]!.addr, BOB)
  assert.equal(Number(names[0]!.expires), 1_950_000_000)
  assert.equal((await one(db, 'SELECT name FROM xns_reverse WHERE address = $1', [ALICE])).name, 'alice.xdevs')
  await db.close()
})

test('Guardian Vault queue, account security state and alert outbox', async () => {
  const chain = new FakeChain()
  // ACCOUNT is an EIP-7702 account delegated to the xDEVS Smart Account.
  chain.codes.set(ACCOUNT, '0xef01000000000000000000000000000000000000001005')
  const { db, ix } = await setup(chain, {
    alerts: { ...cfg().alerts, webhookUrls: ['https://hooks.example/abc'], telegramBotToken: 't0ken', maxAgeSeconds: 3600 },
  })
  await db.query(`INSERT INTO alert_subscriptions (account, channel, target) VALUES ($1, 'telegram', '12345')`, [ACCOUNT])
  ix.now = () => chain.baseTimestamp + 100
  const ev = (name: string, args: Record<string, unknown>, abi: readonly unknown[] = accountEvents) => log(abi, name, args, ACCOUNT)
  chain.addBlock([
    {
      from: ACCOUNT,
      to: ACCOUNT,
      input: '0x01',
      logs: [
        ev('AccountInitialized', { signers: 1n, guardians: 3n, vaultEnabled: true }),
        ev('GuardiansUpdated', { guardians: [ALICE, BOB, REAL], threshold: 2 }),
        ev('VaultActionQueued', { id: 1n, target: BOB, value: 5n * 10n ** 21n, data: '0x', executeAfter: 1_800_086_400n }, vaultEvents),
        ev('VaultActionQueued', { id: 2n, target: FAKE, value: 1n, data: '0x', executeAfter: 1_800_086_400n }, vaultEvents),
      ],
    },
  ])
  chain.addBlock([
    {
      from: ALICE,
      to: ACCOUNT,
      input: '0x01',
      logs: [
        ev('VaultActionCancelled', { id: 2n, by: ALICE }, vaultEvents),
        ev('AccountFrozen', { frozenUntil: 1_802_000_000n }),
        ev('EoaLockUpdated', { state: 2n }),
        ev('RecoveryApproved', { id: keccak256(vHex('r1')), guardian: BOB, approvals: 1, newSigners: [] }),
      ],
    },
  ])
  // A random contract emitting a fake VaultActionQueued: stored, but no alert (unverified).
  chain.addBlock([{ from: SCAM, to: SCAM, input: '0x01', logs: [log(vaultEvents, 'VaultActionQueued', { id: 9n, target: SCAM, value: 1n, data: '0x', executeAfter: 1n }, SCAM)] }])
  await syncAll(ix)

  const actions = await all(db, 'SELECT account, action_id, status, cancelled_by FROM vault_actions ORDER BY queued_block, action_id')
  assert.deepEqual(actions.map((a) => [a.account, a.action_id, a.status]), [
    [ACCOUNT, '1', 'queued'],
    [ACCOUNT, '2', 'cancelled'],
    [SCAM, '9', 'queued'],
  ])
  const sec = await one(db, 'SELECT * FROM account_security WHERE account = $1', [ACCOUNT])
  assert.equal(sec.account_kind, 'xdevs-7702')
  assert.equal(sec.vault_enabled, true)
  assert.equal(Number(sec.frozen_until), 1_802_000_000)
  assert.equal(Number(sec.eoa_lock_state), 2)
  assert.equal(Number(sec.guardian_count), 3)
  assert.equal(Number(sec.guardian_threshold), 2)
  assert.equal(Number(sec.recovery_approvals), 1)
  assert.equal((await one(db, 'SELECT account_kind FROM account_security WHERE account = $1', [SCAM])).account_kind, 'unknown')

  const outbox = await all(db, 'SELECT event, account, destination, payload FROM alert_outbox ORDER BY id')
  const kinds = outbox.map((o) => `${o.event}@${o.destination}`)
  assert.deepEqual(kinds, [
    'vault.queued@webhook:0', 'vault.queued@telegram:12345',
    'vault.queued@webhook:0', 'vault.queued@telegram:12345',
    'account.frozen@webhook:0', 'account.frozen@telegram:12345',
    'eoaLock.changed@webhook:0', 'eoaLock.changed@telegram:12345',
    'recovery.approved@webhook:0', 'recovery.approved@telegram:12345',
  ])
  const p = outbox[0]!.payload as { details: { valueFormatted: string; executeAfter: number }; verified: boolean }
  assert.equal(p.details.valueFormatted, '5000 tXDS')
  assert.equal(p.verified, true)

  // Recovery executed: queued actions become invalid, freeze ends.
  chain.addBlock([{ from: BOB, to: ACCOUNT, input: '0x01', logs: [ev('RecoveryExecuted', { id: keccak256(vHex('r1')) })] }])
  await syncAll(ix)
  assert.equal((await one(db, `SELECT status FROM vault_actions WHERE account = $1 AND action_id = 1`, [ACCOUNT])).status, 'invalidated')
  assert.equal((await one(db, 'SELECT frozen_until FROM account_security WHERE account = $1', [ACCOUNT])).frozen_until, null)
  await db.close()
})

test('old blocks never alert (backfill)', async () => {
  const chain = new FakeChain()
  chain.codes.set(ACCOUNT, '0xef01000000000000000000000000000000000000001005')
  const { db, ix } = await setup(chain, { alerts: { ...cfg().alerts, webhookUrls: ['https://hooks.example/x'], maxAgeSeconds: 3600 } })
  ix.now = () => chain.baseTimestamp + 86_400
  chain.addBlock([{ from: ACCOUNT, to: ACCOUNT, input: '0x01', logs: [log(vaultEvents, 'VaultActionQueued', { id: 1n, target: BOB, value: 1n, data: '0x', executeAfter: 1n }, ACCOUNT)] }])
  await syncAll(ix)
  assert.equal(Number((await one(db, 'SELECT count(*) AS n FROM alert_outbox')).n), 0)
  await db.close()
})

test('WXDS deposit/withdrawal become mint/burn transfers', async () => {
  const { db, chain, ix } = await setup()
  chain.tokens.set(WXDS, { name: 'Wrapped XDS', symbol: 'WXDS', decimals: 18 })
  chain.addBlock([{ from: ALICE, to: WXDS, value: 10n ** 18n, input: '0xd0e30db0', logs: [log(wxdsEvents, 'Deposit', { dst: ALICE, wad: 10n ** 18n }, WXDS)] }])
  await syncAll(ix)
  const rows = await all(db, `SELECT kind, token, from_address, to_address FROM token_transfers ORDER BY seq`)
  assert.deepEqual(rows.map((r) => [r.kind, r.from_address, r.to_address]), [
    ['native', ALICE, WXDS],
    ['erc20', '0x0000000000000000000000000000000000000000', ALICE],
  ])
  await db.close()
})

test('parent-hash mismatch rolls back to the common ancestor and rebuilds derived tables', async () => {
  const { db, chain, ix } = await setup()
  chain.tokens.set(XTOKEN, { name: 'Safe X', symbol: 'SAFX', decimals: 18 })
  chain.addBlock([{ from: ALICE, to: BOB, value: 10n ** 18n }])
  chain.addBlock([{ from: ALICE, to: XTOKEN, input: '0x01', logs: [log(xdrc20Events, 'ApprovalWithExpiry', { owner: ALICE, spender: BOB, value: 5n, expiry: 1_900_000_000n }, XTOKEN)] }])
  chain.addBlock([{ from: ALICE, to: XTOKEN, input: '0x01', logs: [log(xdrc20Events, 'ApprovalWithExpiry', { owner: ALICE, spender: REAL, value: 6n, expiry: 1_900_000_000n }, XTOKEN)] }])
  chain.addBlock([{ from: BOB, to: ALICE, value: 3n * 10n ** 18n }])
  await syncAll(ix)
  assert.equal((await ix.cursor())!.number, 4)

  // Blocks 3.. are replaced (a restarted dev chain); block 3 now has a different approval.
  chain.blocks = chain.blocks.slice(0, 3)
  chain.salt = 'b'
  chain.addBlock([{ from: ALICE, to: XTOKEN, input: '0x01', logs: [log(xdrc20Events, 'ApprovalWithExpiry', { owner: ALICE, spender: FAKE, value: 9n, expiry: 1_900_000_000n }, XTOKEN)] }])
  chain.addBlock([])
  chain.addBlock([])
  assert.equal(await ix.step(), 0, 'divergence detected → rollback step')
  assert.equal((await ix.cursor())!.number, 2)
  await syncAll(ix)
  assert.equal((await ix.cursor())!.number, 5)
  const spenders = (await all(db, 'SELECT spender FROM approvals ORDER BY spender')).map((r) => r.spender)
  assert.deepEqual(spenders.sort(), [BOB, FAKE].sort())
  assert.equal(Number((await one(db, 'SELECT tx_count FROM addresses WHERE address = $1', [ALICE])).tx_count), 3)
  assert.equal(Number((await one(db, 'SELECT tx_count FROM chain_stats')).tx_count), 3)
  assert.equal((await one(db, 'SELECT hash FROM blocks WHERE number = 3')).hash, chain.blocks[3]!.block.hash)
  await db.close()
})

test('a divergence deeper than MAX_REORG_DEPTH is fatal', async () => {
  const { db, chain, ix } = await setup(new FakeChain(), { maxReorgDepth: 2 })
  for (let i = 0; i < 6; i++) chain.addBlock([])
  await syncAll(ix)
  chain.fork(1, 'z')
  chain.addBlock([])
  await assert.rejects(() => ix.step(), /diverged more than 2 blocks/)
  await db.close()
})

test('eth_getLogs cross-check and receipt fallback', async () => {
  const chain = new FakeChain()
  chain.addBlock([{ from: ALICE, to: TOKEN, input: '0x01', logs: [log(erc20Events, 'Transfer', { from: ALICE, to: BOB, value: 1n }, TOKEN)] }])
  const db = await freshDb()
  // A node that loses logs: the batch is refused (retryable) instead of indexing partial data.
  const lossy = Object.assign(Object.create(chain) as FakeChain, {
    request: async (m: string, p: unknown[]) => (m === 'eth_getLogs' ? [] : chain.request(m, p)),
    batch: (c: never) => chain.batch(c),
  })
  await assert.rejects(() => new Indexer(db, lossy, cfg(), silent).step(), /eth_getLogs returned 0 logs/)
  // A node without eth_getBlockReceipts: per-transaction receipts are used.
  const old = Object.assign(Object.create(chain) as FakeChain, {
    request: async (m: string, p: unknown[]) => {
      if (m === 'eth_getBlockReceipts') throw new RpcError(-32601, 'method not found')
      if (m === 'eth_getTransactionReceipt') {
        for (const b of chain.blocks) for (const r of b.receipts) if (r.transactionHash === p[0]) return r
        return null
      }
      return chain.request(m, p)
    },
  })
  old.batch = (calls) => Promise.all(calls.map((c) => old.request(c.method, c.params).catch((e: unknown) => e)))
  const ix = new Indexer(db, old, cfg(), silent)
  await syncAll(ix)
  assert.equal(Number((await one(db, 'SELECT count(*) AS n FROM token_transfers')).n), 1)
  await db.close()
})

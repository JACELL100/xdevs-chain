import { test, before, after } from 'node:test'
import assert from 'node:assert/strict'
import type { AddressInfo } from 'node:net'
import type { Server } from 'node:http'
import { getAddress, keccak256, toHex as vHex } from 'viem'
import { Indexer } from '../src/indexer.ts'
import { createApiServer, RateLimiter } from '../src/api/server.ts'
import { erc20Events } from '../src/decode.ts'
import { accountEvents, permit2Events, vaultEvents, xdrc20Events, xnsEvents } from '../src/abi.generated.ts'
import { addr, cfg, FakeChain, freshDb, log, silent } from './helpers.ts'
import type { Db } from '../src/db.ts'

const ALICE = addr('0xa11ce00000000000000000000000000000000001')
const BOB = addr('0xb0b0000000000000000000000000000000000002')
const REAL = addr('0x1234aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678')
const FAKE = addr('0x1234bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb5678')
const TOKEN = addr('0x70c0000000000000000000000000000000000001')
const XTOKEN = addr('0x70c0000000000000000000000000000000000002')
const ACCOUNT = addr('0xacc0000000000000000000000000000000000004')
const PERMIT2 = addr('0x0000000000000000000000000000000000001008')
const XNS = addr('0x0000000000000000000000000000000000001004')
const NOW = 1_800_001_000

let db: Db
let chain: FakeChain
let server: Server
let base: string
let poisonTx: string

const get = async (path: string, init?: RequestInit) => {
  const res = await fetch(`${base}${path}`, init)
  return { status: res.status, headers: res.headers, body: res.status === 204 || init?.method === 'HEAD' ? null : await res.json() }
}

before(async () => {
  db = await freshDb()
  chain = new FakeChain()
  chain.tokens.set(TOKEN, { name: 'Dollar X', symbol: 'USDX', decimals: 6 })
  chain.tokens.set(XTOKEN, { name: 'Safe X', symbol: 'SAFX', decimals: 18 })
  chain.codes.set(ACCOUNT, '0xef01000000000000000000000000000000000000001005')
  chain.allowances.set(`${XTOKEN}:${ALICE}:${BOB}`, 40n)
  const c = cfg({ knownTokens: [getAddress(TOKEN)] })
  chain.addBlock([{ from: ALICE, to: REAL, value: 10n ** 18n }]) // 1
  chain.addBlock([{ from: ALICE, to: TOKEN, input: '0xa9059cbb', logs: [log(erc20Events, 'Transfer', { from: ALICE, to: REAL, value: 5_000_000n }, TOKEN)] }]) // 2
  chain.addBlock([{ from: FAKE, to: TOKEN, input: '0x23b872dd', logs: [log(erc20Events, 'Transfer', { from: ALICE, to: FAKE, value: 0n }, TOKEN)] }]) // 3
  chain.addBlock([{ from: FAKE, to: ALICE, value: 0n }]) // 4: zero-value native from look-alike
  chain.addBlock([
    {
      from: ALICE,
      to: XTOKEN,
      input: '0x095ea7b3',
      logs: [
        log(erc20Events, 'Approval', { owner: ALICE, spender: BOB, value: 2n ** 256n - 1n }, XTOKEN),
        log(xdrc20Events, 'ApprovalWithExpiry', { owner: ALICE, spender: BOB, value: 2n ** 256n - 1n, expiry: 1_802_000_000n }, XTOKEN),
      ],
    },
  ]) // 5
  chain.addBlock([{ from: ALICE, to: PERMIT2, input: '0x01', logs: [log(permit2Events, 'Approval', { owner: ALICE, token: TOKEN, spender: REAL, amount: 5n, expiration: 1_800_000_500 }, PERMIT2)] }]) // 6: expired at NOW
  chain.addBlock([
    {
      from: ALICE,
      to: XNS,
      input: '0x01',
      logs: [
        log(xnsEvents, 'NameRegistered', { label: 'alice', labelhash: keccak256(vHex('alice')), owner: ALICE, expires: 1_900_000_000n, fee: 0n }, XNS),
        log(xnsEvents, 'ReverseClaimed', { addr: ALICE, node: keccak256(vHex('rev-alice')) }, XNS),
        log(xnsEvents, 'NameChanged', { node: keccak256(vHex('rev-alice')), name: 'alice.xdevs' }, XNS),
        log(xnsEvents, 'NameRegistered', { label: 'old', labelhash: keccak256(vHex('old')), owner: BOB, expires: 1_799_000_000n, fee: 0n }, XNS),
      ],
    },
  ]) // 7
  chain.addBlock([
    {
      from: ACCOUNT,
      to: ACCOUNT,
      input: '0x01',
      logs: [
        log(accountEvents, 'AccountInitialized', { signers: 1n, guardians: 2n, vaultEnabled: true }, ACCOUNT),
        log(vaultEvents, 'VaultActionQueued', { id: 1n, target: BOB, value: 10n ** 21n, data: '0x', executeAfter: 1_800_050_000n }, ACCOUNT),
        log(vaultEvents, 'VaultActionQueued', { id: 2n, target: BOB, value: 1n, data: '0x', executeAfter: 1_800_000_100n }, ACCOUNT),
        log(vaultEvents, 'VaultActionQueued', { id: 3n, target: BOB, value: 1n, data: '0x', executeAfter: 1_700_000_000n }, ACCOUNT),
        log(accountEvents, 'EoaLockUpdated', { state: 1n }, ACCOUNT),
      ],
    },
  ]) // 8
  const ix = new Indexer(db, chain, c, silent)
  while ((await ix.step()) > 0) {
    /* sync */
  }
  poisonTx = chain.blocks[3]!.block.transactions[0]!.hash
  server = createApiServer({ db, rpc: chain, cfg: { ...c, rateLimitRps: 1000, rateLimitBurst: 1000 }, now: () => NOW, log: silent })
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', () => r()))
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`
})

after(async () => {
  await new Promise((r) => server.close(r))
  await db.close()
})

test('health and stats', async () => {
  const h = await get('/health')
  assert.equal(h.status, 200)
  assert.deepEqual(h.body, { ok: true, chainId: 7117, indexedBlock: 8, headBlock: 8, lagBlocks: 0 })
  const s = await get('/api/v1/stats')
  assert.equal(s.body.totalTransactions, 8)
  assert.equal(s.body.avgBlockTimeMs, 2000)
  assert.equal(s.body.latestBaseFee, '0x3b9aca00')
  assert.match(s.body.burnedFees, /^0x[0-9a-f]+$/)
  assert.equal(s.headers.get('access-control-allow-origin'), '*')
  assert.equal(s.headers.get('cache-control'), 'public, max-age=5')
})

test('blocks and txs: cursor pagination, newest first, no overlap', async () => {
  const p1 = await get('/api/v1/blocks?limit=4')
  assert.deepEqual(p1.body.items.map((b: { number: number }) => b.number), [8, 7, 6, 5])
  const p2 = await get(`/api/v1/blocks?limit=4&cursor=${p1.body.nextCursor}`)
  assert.deepEqual(p2.body.items.map((b: { number: number }) => b.number), [4, 3, 2, 1])
  const p3 = await get(`/api/v1/blocks?limit=4&cursor=${p2.body.nextCursor}`)
  assert.deepEqual(p3.body.items.map((b: { number: number }) => b.number), [0])
  assert.equal(p3.body.nextCursor, null)

  const b = await get('/api/v1/blocks/1')
  assert.equal(b.body.transactions.length, 1)
  assert.equal(b.body.miner, getAddress(addr(0xfee)))
  assert.equal(b.body.commit, null, 'dev blocks have no commit certificate')
  assert.equal((await get(`/api/v1/blocks/${b.body.hash}`)).body.number, 1)
  assert.equal((await get('/api/v1/blocks/0x8')).body.number, 8)
  assert.equal((await get('/api/v1/blocks/latest')).body.number, 8)
  assert.equal((await get('/api/v1/blocks/99')).status, 404)
  assert.equal((await get('/api/v1/blocks/abc')).status, 400)

  const t1 = await get('/api/v1/txs?limit=3')
  const t2 = await get(`/api/v1/txs?limit=3&cursor=${t1.body.nextCursor}`)
  const hashes = [...t1.body.items, ...t2.body.items].map((t: { hash: string }) => t.hash)
  assert.equal(new Set(hashes).size, 6)
  assert.equal((await get('/api/v1/txs?cursor=garbage')).status, 400)
  assert.equal((await get('/api/v1/txs?limit=500')).status, 400)
})

test('tx detail: fee split, decoded logs, transfers with spam flags, approvals with unlimited/expiry', async () => {
  const poison = await get(`/api/v1/txs/${poisonTx}`)
  assert.equal(poison.status, 200)
  assert.equal(poison.body.logs[0].event, 'Transfer')
  assert.equal(poison.body.logs[0].standard, 'erc20')
  assert.deepEqual(poison.body.tokenTransfers[0].spamReasons, ['zero-value', 'look-alike'])
  assert.equal(poison.body.tokenTransfers[0].lookAlikeOf, getAddress(REAL))
  assert.equal(poison.body.tokenTransfers[0].token.symbol, 'USDX')
  assert.equal(poison.body.tokenTransfers[0].token.known, true)
  const fb = poison.body.feeBreakdown
  assert.equal(BigInt(fb.burned) + BigInt(fb.treasury), BigInt(fb.baseFee))
  assert.equal(BigInt(fb.baseFee) + BigInt(fb.priorityFee), BigInt(fb.total))
  assert.equal(fb.burnBps, 8000)

  const approveTx = chain.blocks[5]!.block.transactions[0]!.hash
  const ap = await get(`/api/v1/txs/${approveTx}`)
  assert.equal(ap.body.approvals.length, 1, 'the plain Approval twin of an XDRC-20 approval is folded')
  assert.equal(ap.body.approvals[0].kind, 'xdrc20')
  assert.equal(ap.body.approvals[0].unlimited, true)
  assert.equal(ap.body.approvals[0].expiry, 1_802_000_000)
  assert.equal(ap.body.approvals[0].token.symbol, 'SAFX')
  assert.equal(ap.body.logs[1].args.expiry, '0x6b685680')
  assert.equal((await get(`/api/v1/txs/0x${'0'.repeat(64)}`)).status, 404)
})

test('address summary: live balance, XNS primary name, labels, security state', async () => {
  const a = await get(`/api/v1/addresses/${ALICE.toUpperCase().replace('0X', '0x')}`)
  assert.equal(a.status, 200)
  assert.equal(a.body.address, getAddress(ALICE))
  assert.equal(a.body.live, true)
  assert.equal(a.body.balance, '0xde0b6b3a7640000')
  assert.equal(a.body.kind, 'eoa')
  assert.equal(a.body.xns.primaryName, 'alice.xdevs')
  assert.equal(a.body.security.accountKind, null)

  const acct = await get(`/api/v1/addresses/${ACCOUNT}`)
  assert.equal(acct.body.kind, 'delegated')
  assert.equal(acct.body.delegatedTo, '0x0000000000000000000000000000000000001005')
  assert.equal(acct.body.security.accountKind, 'xdevs-7702')
  assert.equal(acct.body.security.vaultEnabled, true)
  assert.deepEqual(acct.body.security.eoaLock, { status: 'locked', active: null, source: 'indexer' })
  assert.equal(acct.body.security.openVaultActions, 2, 'id 3 is past its execution window')

  chain.eoaLock = { active: true, status: 'sealed', word: `0x${'0'.repeat(63)}2`, delegatedTo: '0x0000000000000000000000000000000000001005' }
  const live = await get(`/api/v1/addresses/${ACCOUNT}`)
  assert.deepEqual(live.body.security.eoaLock, { status: 'sealed', active: true, source: 'rpc' })
  chain.eoaLock = null

  const p2 = await get('/api/v1/addresses/0x0000000000000000000000000000000000001008')
  assert.deepEqual(p2.body.labels, ['Permit2Expiring (predeploy)'])
  assert.equal((await get('/api/v1/addresses/0x1234')).status, 400)
})

test('address txs and token transfers hide spam not sent by the viewer', async () => {
  const txs = await get(`/api/v1/addresses/${ALICE}/txs`)
  assert.equal(txs.body.hiddenSpam, 1, 'the zero-value native tx from the look-alike')
  assert.ok(txs.body.items.every((t: { spam: boolean }) => !t.spam))
  const all = await get(`/api/v1/addresses/${ALICE}/txs?includeSpam=true`)
  assert.equal(all.body.items.length, txs.body.items.length + 1)
  assert.equal(all.body.hiddenSpam, null)

  const tr = await get(`/api/v1/addresses/${ALICE}/token-transfers`)
  assert.equal(tr.body.hiddenSpam, 2, 'zero-value transferFrom (sent by attacker) and the zero native transfer')
  const trAll = await get(`/api/v1/addresses/${ALICE}/token-transfers?includeSpam=true&limit=2`)
  assert.equal(trAll.body.items.length, 2)
  assert.ok(trAll.body.nextCursor)
  const filtered = await get(`/api/v1/addresses/${ALICE}/token-transfers?token=${TOKEN}`)
  assert.equal(filtered.status, 200)
  assert.equal(filtered.body.hiddenSpam, 1, 'only the token transfer is counted with a token filter')
  const only = await get(`/api/v1/addresses/${ALICE}/token-transfers?token=${TOKEN}&includeSpam=1`)
  assert.ok(only.body.items.every((t: { token: { address: string } }) => t.token.address === getAddress(TOKEN)))
  // The attacker's own view: their sends are never hidden from them.
  const attacker = await get(`/api/v1/addresses/${FAKE}/token-transfers`)
  assert.equal(attacker.body.items.length, 2)
})

test('live approvals with remaining allowance, expiry and a revoke hint', async () => {
  const r = await get(`/api/v1/addresses/${ALICE}/approvals`)
  assert.equal(r.body.items.length, 1, 'the Permit2 approval expired at NOW')
  const ap = r.body.items[0]
  assert.equal(ap.kind, 'xdrc20')
  assert.equal(ap.unlimited, true)
  assert.equal(ap.expired, false)
  assert.equal(ap.remaining, '0x28')
  assert.equal(ap.revoke.to, getAddress(XTOKEN))
  assert.match(ap.revoke.data, /^0x095ea7b3/)
  const withExpired = await get(`/api/v1/addresses/${ALICE}/approvals?includeExpired=true`)
  assert.equal(withExpired.body.items.length, 2)
  const p = withExpired.body.items.find((x: { kind: string }) => x.kind === 'permit2')
  assert.equal(p.expired, true)
  assert.equal(p.revoke.to, '0x0000000000000000000000000000000000001008')
})

test('vault queue statuses', async () => {
  const open = await get(`/api/v1/addresses/${ACCOUNT}/vault-queue`)
  assert.deepEqual(open.body.items.map((x: { id: string; status: string }) => [x.id, x.status]), [['0x2', 'ready'], ['0x1', 'queued']])
  const all = await get(`/api/v1/addresses/${ACCOUNT}/vault-queue?status=all&limit=2`)
  assert.equal(all.body.items.length, 2)
  const rest = await get(`/api/v1/addresses/${ACCOUNT}/vault-queue?status=all&limit=2&cursor=${all.body.nextCursor}`)
  assert.deepEqual(rest.body.items.map((x: { id: string; status: string }) => [x.id, x.status]), [['0x1', 'queued']])
  assert.equal(all.body.items[0].status, 'expired')
  assert.equal((await get(`/api/v1/addresses/${ACCOUNT}/vault-queue?status=x`)).status, 400)
})

test('names and search', async () => {
  const n = await get('/api/v1/names/ALICE')
  assert.equal(n.body.name, 'alice.xdevs')
  assert.equal(n.body.status, 'active')
  assert.equal(n.body.address, getAddress(ALICE))
  assert.equal(n.body.isPrimaryFor, getAddress(ALICE))
  const old = await get('/api/v1/names/old.xdevs')
  assert.equal(old.body.status, 'grace')
  assert.equal(old.body.address, null)
  assert.equal((await get('/api/v1/names/nobody')).status, 404)
  assert.equal((await get('/api/v1/names/pay%20pal')).status, 400)

  assert.deepEqual((await get('/api/v1/search?q=3')).body.results, [{ type: 'block', id: '3', label: 'Block 3' }])
  assert.equal((await get(`/api/v1/search?q=${poisonTx}`)).body.results[0].type, 'tx')
  assert.deepEqual((await get(`/api/v1/search?q=${ALICE}`)).body.results, [{ type: 'address', id: getAddress(ALICE), label: 'alice.xdevs' }])
  const name = (await get('/api/v1/search?q=alice.xdevs')).body.results
  assert.deepEqual(name[0], { type: 'name', id: 'alice.xdevs', label: getAddress(ALICE) })
  assert.deepEqual((await get('/api/v1/search?q=al')).body.results, [{ type: 'name', id: 'alice.xdevs', label: null }])
  assert.deepEqual((await get('/api/v1/search?q=zzz-nothing')).body.results, [])
})

test('HTTP behaviour: CORS preflight, HEAD, 405, 404, rate limit', async () => {
  const pre = await fetch(`${base}/api/v1/stats`, { method: 'OPTIONS' })
  assert.equal(pre.status, 204)
  assert.match(pre.headers.get('access-control-allow-methods') ?? '', /GET/)
  const head = await get('/api/v1/stats', { method: 'HEAD' })
  assert.equal(head.status, 200)
  assert.equal((await get('/api/v1/stats', { method: 'POST' })).status, 405)
  assert.equal((await get('/api/v1/nope')).status, 404)

  let t = 0
  const rl = new RateLimiter(2, 3, () => t)
  assert.deepEqual([rl.take('a').ok, rl.take('a').ok, rl.take('a').ok, rl.take('a').ok], [true, true, true, false])
  assert.equal(rl.take('b').ok, true, 'per client')
  t += 500
  assert.equal(rl.take('a').ok, true, 'refills at rps')
  assert.equal(rl.take('a').retryAfter, 1)

  const strict = createApiServer({ db, rpc: chain, cfg: { ...cfg(), rateLimitRps: 1, rateLimitBurst: 2, corsOrigins: ['https://explorer.xdevschain.tech'] }, now: () => NOW, log: silent })
  await new Promise<void>((r) => strict.listen(0, '127.0.0.1', () => r()))
  const url = `http://127.0.0.1:${(strict.address() as AddressInfo).port}/health`
  const codes = []
  for (let i = 0; i < 3; i++) codes.push((await fetch(url, { headers: { origin: 'https://evil.example' } })).status)
  assert.deepEqual(codes, [200, 200, 429])
  const ok = await fetch(url, { headers: { origin: 'https://explorer.xdevschain.tech' } })
  assert.equal(ok.headers.get('retry-after'), '1')
  const r = await fetch(url, { headers: { origin: 'https://evil.example' } })
  assert.equal(r.headers.get('access-control-allow-origin'), null)
  await new Promise((res) => strict.close(res))
})

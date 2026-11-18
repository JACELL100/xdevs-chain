// Unit tests (no chain): reputation, mempool rules and bundle selection, fee / preVerificationGas coverage,
// rate limiting and client IPs, configuration, keystore, metrics, state file.
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import type { IncomingMessage } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, it } from 'node:test'
import { type Address, type Hex, encodeFunctionData, keccak256, pad, parseEther, toHex } from 'viem'
import { entryPointAbi } from '../src/abi.ts'
import { DEFAULT_CONFIG, loadConfig, loadOperatorKey } from '../src/config.ts'
import { RpcCodes, RpcError } from '../src/errors.ts'
import { decryptKeystore } from '../src/keystore.ts'
import { type AdmissionContext, Mempool, type PoolEntry, nonceParts } from '../src/mempool.ts'
import { Metrics } from '../src/metrics.ts'
import { TokenBuckets, clientIp } from '../src/ratelimit.ts'
import { BAN_OPS_SEEN_PENALTY, type EntityStatus, Reputation } from '../src/reputation.ts'
import {
  PVG_FIXED,
  PVG_PER_OP,
  bundleFees,
  checkFees,
  checkPreVerificationGas,
  checkStatic,
  estimatePreVerificationGas,
  opMaxCost,
  parseValidationData,
  requiredPreVerificationGas,
  sponsorOf,
} from '../src/rules.ts'
import { JsonStore } from '../src/store.ts'
import { type UserOperation, calldataGas, packUserOp } from '../src/userop.ts'

const A = (n: number) => pad(toHex(n), { size: 20 }) as Address
const BENEFICIARY = A(0xbeef)
const PM = A(0x1007)
const FACTORY = A(0x1006)
const GWEI = 1_000_000_000n

function op(o: Partial<UserOperation> = {}): UserOperation {
  return {
    sender: A(0xa1),
    nonce: 0n,
    factory: null,
    factoryData: null,
    callData: '0xb61d27f6' + '00'.repeat(96),
    callGasLimit: 50_000n,
    verificationGasLimit: 150_000n,
    preVerificationGas: 60_000n,
    maxFeePerGas: 2n * GWEI,
    maxPriorityFeePerGas: GWEI / 1000n,
    paymaster: null,
    paymasterVerificationGasLimit: null,
    paymasterPostOpGasLimit: null,
    paymasterData: null,
    signature: toHex(new Uint8Array(65).fill(7)),
    ...o,
  } as UserOperation
}

let hashN = 0
function entry(o: Partial<UserOperation> = {}, e: Partial<PoolEntry> = {}): PoolEntry {
  const u = op(o)
  const { key, seq } = nonceParts(u.nonce)
  return {
    hash: keccak256(toHex(++hashN)),
    op: u,
    nonceKey: key,
    nonceSeq: seq,
    factory: u.factory ? u.factory.toLowerCase() : undefined,
    paymaster: u.paymaster ? u.paymaster.toLowerCase() : undefined,
    maxCost: opMaxCost(u),
    addedAt: Date.now(),
    addedBlock: 100n,
    validUntil: 0,
    validAfter: 0,
    status: 'pending',
    attempts: 0,
    sponsored: false,
    ...e,
  }
}

function ctx(o: Partial<AdmissionContext> & { statuses?: Record<string, EntityStatus>; staked?: string[] } = {}): AdmissionContext {
  return {
    onchainSeq: 0n,
    status: (a) => o.statuses?.[a.toLowerCase()] ?? 'ok',
    isStaked: (a) => (o.staked ?? []).includes(a.toLowerCase()),
    stakedAllowance: (_a, base) => base + 5,
    limits: DEFAULT_CONFIG,
    ...o,
  }
}

const code = (fn: () => unknown): number => {
  try {
    fn()
  } catch (e) {
    assert.ok(e instanceof RpcError, String(e))
    return e.code
  }
  return 0
}

// ------------------------------------------------------------------------------------------------ reputation
describe('reputation (ERC-7562)', () => {
  const params = { minInclusionRateDenominator: 10, throttlingSlack: 10, banSlack: 50 }

  it('OK -> THROTTLED -> BANNED as ops are seen but not included; inclusions restore it', () => {
    const r = new Reputation(params)
    const pm = A(1)
    for (let i = 0; i < 109; i++) r.seen(pm) // 109 / 10 = 10 expected inclusions <= 0 + THROTTLING_SLACK
    assert.equal(r.status(pm), 'ok')
    r.seen(pm) // 110 -> 11 > 10
    assert.equal(r.status(pm), 'throttled')
    for (let i = 0; i < 500; i++) r.seen(pm) // 620 -> 62 > 50
    assert.equal(r.status(pm), 'banned')
    for (let i = 0; i < 20; i++) r.included(pm) // 62 <= 20 + 50
    assert.equal(r.status(pm), 'throttled')
    for (let i = 0; i < 40; i++) r.included(pm) // 62 <= 60 + 10
    assert.equal(r.status(pm), 'ok')
  })

  it('an entity that made a bundle revert is banned; the allowlist is exempt', () => {
    const r = new Reputation({ ...params, allowlist: [A(9)] })
    r.crashed(A(2))
    assert.equal(r.get(A(2)).opsSeen, BAN_OPS_SEEN_PENALTY)
    assert.equal(r.status(A(2)), 'banned')
    r.crashed(A(9))
    assert.equal(r.status(A(9)), 'ok')
  })

  it('decays hourly by 23/24 and survives a snapshot round trip', () => {
    let now = 0
    const r = new Reputation(params, () => now)
    for (let i = 0; i < 240; i++) r.seen(A(3))
    r.included(A(3))
    now = 3_600_000
    r.decay()
    assert.deepEqual(r.get(A(3)), { opsSeen: 230, opsIncluded: 0 })
    const r2 = new Reputation(params, () => now)
    r2.restore(JSON.parse(JSON.stringify(r.snapshot())))
    assert.deepEqual(r2.get(A(3)), r.get(A(3)))
    assert.ok(r.stakedAllowance(A(3), 10) >= 10)
  })
})

// ------------------------------------------------------------------------------------------------ mempool
describe('mempool admission', () => {
  it('rejects used nonces and gaps, accepts the next sequence of a pending sender', () => {
    const m = new Mempool()
    assert.equal(code(() => m.checkAdmission(entry({ nonce: 3n }), ctx({ onchainSeq: 4n }))), RpcCodes.rejectedByEntryPointOrAccount)
    assert.equal(code(() => m.checkAdmission(entry({ nonce: 5n }), ctx({ onchainSeq: 4n }))), RpcCodes.invalidParams)
    m.put(entry({ nonce: 4n }))
    assert.equal(m.checkAdmission(entry({ nonce: 5n }), ctx({ onchainSeq: 4n })), undefined)
    // Another nonce key has its own sequence.
    const keyed = entry({ nonce: (7n << 64n) | 0n })
    assert.equal(m.checkAdmission(keyed, ctx({ onchainSeq: 0n })), undefined)
  })

  it('replacement needs +10 % on both fee fields and is impossible once submitted', () => {
    const m = new Mempool()
    const first = entry({ nonce: 0n })
    m.put(first)
    assert.equal(code(() => m.checkAdmission(entry({ maxFeePerGas: 2n * GWEI + 1n }), ctx())), RpcCodes.invalidParams)
    const ok = entry({ maxFeePerGas: (22n * GWEI) / 10n, maxPriorityFeePerGas: (11n * GWEI) / 10_000n })
    assert.equal(m.checkAdmission(ok, ctx()), first)
    first.status = 'submitted'
    assert.match(String((() => { try { m.checkAdmission(ok, ctx()) } catch (e) { return (e as Error).message } })()), /submitted bundle/)
  })

  it('limits ops per sender, per unstaked factory and per throttled paymaster; bans; STO-040; capacity', () => {
    const m = new Mempool()
    for (let i = 0; i < 4; i++) m.put(entry({ nonce: BigInt(i) }))
    assert.equal(code(() => m.checkAdmission(entry({ nonce: 4n }), ctx())), RpcCodes.bannedOrThrottled)

    const f = new Mempool()
    for (let i = 0; i < 10; i++) f.put(entry({ sender: A(0x100 + i), factory: FACTORY }))
    assert.equal(code(() => f.checkAdmission(entry({ sender: A(0x200), factory: FACTORY }), ctx())), RpcCodes.bannedOrThrottled)
    // Staked factories get a larger allowance.
    assert.equal(f.checkAdmission(entry({ sender: A(0x200), factory: FACTORY }), ctx({ staked: [FACTORY.toLowerCase()] })), undefined)

    const p = new Mempool()
    const pmOp = (s: number) => entry({ sender: A(s), paymaster: PM, paymasterVerificationGasLimit: 50_000n, paymasterPostOpGasLimit: 40_000n })
    for (let i = 0; i < 4; i++) p.put(pmOp(0x300 + i))
    const throttled = ctx({ statuses: { [PM.toLowerCase()]: 'throttled' }, staked: [PM.toLowerCase()] })
    assert.equal(code(() => p.checkAdmission(pmOp(0x400), throttled)), RpcCodes.bannedOrThrottled)
    assert.equal(code(() => new Mempool().checkAdmission(pmOp(0x400), ctx({ statuses: { [PM.toLowerCase()]: 'banned' } }))), RpcCodes.bannedOrThrottled)
    // Paymaster deposit must cover all its pending ops.
    const deposit = 4n * pmOp(1).maxCost + pmOp(1).maxCost - 1n
    assert.equal(code(() => p.checkAdmission(pmOp(0x401), ctx({ staked: [PM.toLowerCase()], paymasterDeposit: deposit }))), RpcCodes.rejectedByPaymaster)

    // STO-040: the paymaster address cannot also be a sender in the mempool.
    assert.equal(code(() => p.checkAdmission(entry({ sender: PM }), ctx())), RpcCodes.opcodeOrStorageViolation)

    const full = new Mempool()
    full.put(entry({ sender: A(0x500) }))
    assert.equal(code(() => full.checkAdmission(entry({ sender: A(0x501) }), ctx({ limits: { ...DEFAULT_CONFIG, maxMempoolOps: 1 } }))), RpcCodes.limitExceeded)
  })
})

describe('bundle selection', () => {
  const sel = (m: Mempool, o: Partial<Parameters<Mempool['select']>[0]> = {}) =>
    m.select({
      baseFee: GWEI / 10n,
      busy: new Set(),
      maxOps: 16,
      maxGas: 15_000_000n,
      nowSec: 1_000,
      minValidSeconds: 15,
      status: () => 'ok',
      throttledBundleCount: 4,
      ...o,
    })

  it('orders by effective tip, keeps nonce order per sender, and packs different senders together', () => {
    const m = new Mempool()
    const a0 = entry({ sender: A(1), nonce: 0n, maxPriorityFeePerGas: 1n })
    const a1 = entry({ sender: A(1), nonce: 1n, maxPriorityFeePerGas: GWEI })
    const b0 = entry({ sender: A(2), nonce: 0n, maxPriorityFeePerGas: GWEI / 2n })
    for (const e of [a1, b0, a0]) m.put(e)
    const picked = sel(m)
    assert.equal(picked.length, 3)
    assert.ok(picked.indexOf(a0) < picked.indexOf(a1), 'nonce 0 before nonce 1')
    assert.equal(picked[0], b0, 'highest tip whose predecessors are satisfied first')
  })

  it('skips ops of entities in flight, underpriced or expiring ops, and caps gas, size and throttled entities', () => {
    const m = new Mempool()
    const busy = entry({ sender: A(1) })
    const cheap = entry({ sender: A(2), maxFeePerGas: 1n, maxPriorityFeePerGas: 1n })
    const expiring = entry({ sender: A(3) }, { validUntil: 1_005 })
    const later = entry({ sender: A(4) }, { validAfter: 2_000 })
    const pmA = entry({ sender: A(5), paymaster: PM, paymasterVerificationGasLimit: 1n, paymasterPostOpGasLimit: 1n })
    const pmB = entry({ sender: A(6), paymaster: PM, paymasterVerificationGasLimit: 1n, paymasterPostOpGasLimit: 1n })
    for (const e of [busy, cheap, expiring, later, pmA, pmB]) m.put(e)
    const status = (a: string) => (a === PM.toLowerCase() ? 'throttled' : 'ok') as EntityStatus
    const picked = sel(m, { busy: new Set([A(1).toLowerCase()]), status, throttledBundleCount: 1 })
    assert.deepEqual(picked.length, 1)
    assert.ok(picked[0] === pmA || picked[0] === pmB)
    const two = new Mempool()
    two.put(entry({ sender: A(7) }))
    two.put(entry({ sender: A(8) }))
    assert.equal(sel(two, { maxOps: 1 }).length, 1)
    assert.equal(sel(two, { maxGas: 300_000n }).length, 1)
  })

  it('reports stale ops: TTL, validUntil passed, banned entities, slow throttled entities', () => {
    const m = new Mempool()
    const old = entry({ sender: A(1) }, { addedAt: 0 })
    const gone = entry({ sender: A(2) }, { validUntil: 10 })
    const banned = entry({ sender: A(3), factory: FACTORY })
    for (const e of [old, gone, banned]) m.put(e)
    const st = m.stale({
      nowMs: 1_000_000,
      nowSec: 20,
      block: 100n,
      ttlSeconds: 600,
      liveBlocks: 10,
      status: (a) => (a === FACTORY.toLowerCase() ? 'banned' : 'ok'),
    })
    assert.deepEqual(new Set(st.map((s) => s.entry)), new Set([old, gone, banned]))
  })
})

// ------------------------------------------------------------------------------------------------ fees
describe('fees and preVerificationGas (no operator loss)', () => {
  it('requires the preVerificationGas the estimator returns, and estimation leaves a margin', () => {
    const o = op()
    const need = requiredPreVerificationGas(o, BENEFICIARY)
    const est = estimatePreVerificationGas(o, BENEFICIARY)
    assert.ok(est >= (need * 110n) / 100n)
    assert.equal(code(() => checkPreVerificationGas({ ...o, preVerificationGas: need - 1n }, BENEFICIARY)), RpcCodes.invalidParams)
    assert.equal(code(() => checkPreVerificationGas({ ...o, preVerificationGas: est }, BENEFICIARY)), 0)
  })

  it('per-op preVerificationGas covers the whole unmetered bundle overhead for any bundle size', () => {
    for (const n of [1, 2, 5, 16]) {
      const ops = Array.from({ length: n }, (_, i) =>
        op({ sender: A(0x1000 + i), callData: toHex(new Uint8Array(36 + i * 50).fill(0xab)), signature: toHex(new Uint8Array(65 + i * 30).fill(9)) }),
      )
      const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'handleOps', args: [ops.map(packUserOp), BENEFICIARY] })
      const words = BigInt(Math.ceil((data.length / 2 - 1 - 4) / 32))
      const overhead = PVG_FIXED + BigInt(n) * PVG_PER_OP + calldataGas(data) + 4n * words
      const charged = ops.reduce((s, o) => s + requiredPreVerificationGas(o, BENEFICIARY), 0n)
      assert.ok(charged >= overhead, `n=${n}: charged ${charged} < overhead ${overhead}`)
    }
  })

  it('bundle fee caps never exceed what any op pays per gas', () => {
    const ops = [
      op({ maxFeePerGas: 3n * GWEI, maxPriorityFeePerGas: 2n * GWEI }),
      op({ maxFeePerGas: 2n * GWEI, maxPriorityFeePerGas: GWEI / 10n }),
      op({ maxFeePerGas: 5n * GWEI, maxPriorityFeePerGas: 1n }),
    ]
    const f = bundleFees(ops)
    for (const baseFee of [0n, GWEI / 10n, GWEI, 2n * GWEI]) {
      const txPrice = f.maxFeePerGas < f.maxPriorityFeePerGas + baseFee ? f.maxFeePerGas : f.maxPriorityFeePerGas + baseFee
      for (const o of ops) {
        const opPrice = o.maxFeePerGas < o.maxPriorityFeePerGas + baseFee ? o.maxFeePerGas : o.maxPriorityFeePerGas + baseFee
        assert.ok(opPrice >= txPrice)
      }
    }
  })

  it('enforces the minimum tip and the base-fee floor', () => {
    const cfg = { minPriorityFeePerGas: 1_000_000n, baseFeeMarginPct: 100n }
    assert.equal(code(() => checkFees(op({ maxPriorityFeePerGas: 999_999n }), GWEI / 10n, cfg)), RpcCodes.invalidParams)
    assert.equal(code(() => checkFees(op({ maxFeePerGas: GWEI / 10n }), GWEI / 10n, cfg)), RpcCodes.invalidParams)
    assert.equal(code(() => checkFees(op({ maxFeePerGas: GWEI / 10n + 1_000_000n, maxPriorityFeePerGas: 1_000_000n }), GWEI / 10n, cfg)), 0)
  })

  it('static limits: size, signature, gas caps', () => {
    const lim = DEFAULT_CONFIG
    assert.equal(code(() => checkStatic(op(), lim)), 0)
    assert.equal(code(() => checkStatic(op({ signature: '0x' }), lim)), RpcCodes.invalidParams)
    assert.equal(code(() => checkStatic(op({ callData: toHex(new Uint8Array(20_000)) }), lim)), RpcCodes.invalidParams)
    assert.equal(code(() => checkStatic(op({ verificationGasLimit: 5_000_000n }), lim)), RpcCodes.invalidParams)
    assert.equal(code(() => checkStatic(op({ callGasLimit: 100n }), lim)), RpcCodes.invalidParams)
    assert.equal(code(() => checkStatic(op({ preVerificationGas: 0n }), lim)), RpcCodes.invalidParams)
    assert.equal(code(() => checkStatic(op({ preVerificationGas: 0n, signature: '0x' }), lim, true)), 0, 'estimation accepts partial ops')
  })

  it('parses validationData and XdevsPaymaster sponsor modes', () => {
    const v = parseValidationData((123n << 208n) | (456n << 160n) | 1n)
    assert.deepEqual([v.sigFailed, v.validUntil, v.validAfter], [true, 456, 123])
    assert.equal(parseValidationData(0n).validUntil, 0)
    const sponsor = A(0x5)
    assert.deepEqual(sponsorOf(op({ paymaster: PM, paymasterData: sponsor }), PM), { sponsor, openMode: true })
    assert.equal(sponsorOf(op({ paymaster: PM, paymasterData: `${sponsor}${'00'.repeat(12)}` as Hex }), PM)?.openMode, false)
    assert.equal(sponsorOf(op({ paymaster: A(0x99), paymasterData: sponsor }), PM), null)
  })
})

// ------------------------------------------------------------------------------------------------ rate limits
describe('rate limits and client IP', () => {
  it('token buckets: burst, refill, bounded keys', () => {
    let now = 0
    const b = new TokenBuckets(2, 3, 2, () => now)
    assert.ok(b.take('x') && b.take('x') && b.take('x'))
    assert.equal(b.take('x'), false)
    assert.equal(b.retryAfter('x'), 1)
    now = 500
    assert.equal(b.take('x'), true)
    b.take('y')
    b.take('z')
    assert.equal(b.size, 2)
  })

  const req = (peer: string, headers: Record<string, string> = {}) =>
    ({ socket: { remoteAddress: peer }, headers }) as unknown as IncomingMessage
  const trusted = DEFAULT_CONFIG.trustedProxies

  it('honours CF-Connecting-IP only from a trusted peer (cloudflared on localhost)', () => {
    assert.equal(clientIp(req('127.0.0.1', { 'cf-connecting-ip': '203.0.113.7' }), 'cloudflare', trusted), '203.0.113.7')
    assert.equal(clientIp(req('::ffff:127.0.0.1', { 'cf-connecting-ip': '203.0.113.7' }), 'cloudflare', trusted), '203.0.113.7')
    assert.equal(clientIp(req('198.51.100.1', { 'cf-connecting-ip': '203.0.113.7' }), 'cloudflare', trusted), '198.51.100.1')
    assert.equal(clientIp(req('127.0.0.1', { 'cf-connecting-ip': '203.0.113.7' }), 'none', trusted), '127.0.0.1')
    assert.equal(clientIp(req('127.0.0.1', { 'cf-connecting-ip': 'not-an-ip' }), 'cloudflare', trusted), '127.0.0.1')
    assert.equal(clientIp(req('127.0.0.1', { 'x-forwarded-for': '192.0.2.4, 10.0.0.1' }), 'xff', trusted), '192.0.2.4')
    assert.equal(clientIp(req('127.0.0.1', { 'cf-connecting-ip': '2001:db8:1:2:3:4:5:6' }), 'cloudflare', trusted), '2001:db8:1:2::/64')
  })
})

// ------------------------------------------------------------------------------------------------ config / keys
describe('configuration and operator key', () => {
  const FIXTURE = join(import.meta.dirname, 'fixtures', 'keystore-scrypt.json')
  const FIXTURE_KEY = '0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'

  it('defaults are production-safe and env values are parsed and validated', () => {
    const c = loadConfig({})
    assert.equal(c.requirePaymasterStake, true)
    assert.equal(c.sponsorPolicy, 'activation')
    assert.equal(c.trustProxy, 'none')
    const p = loadConfig({
      CHAIN_ID: '717171',
      OPERATOR_MIN_BALANCE: '0.2',
      OPERATOR_LOW_BALANCE: '1.5',
      CORS_ORIGINS: 'https://wallet.xdevschain.tech, https://xdevschain.tech',
      TRUST_PROXY: 'cloudflare',
      BENEFICIARY: '0x00000000000000000000000000000000000000aa',
      SPONSOR_POLICY: 'off',
      DATA_DIR: '/var/lib/xdevs-bundler',
    })
    assert.equal(p.expectedChainId, 717171)
    assert.equal(p.operatorMinBalance, parseEther('0.2'))
    assert.deepEqual(p.corsOrigins, ['https://wallet.xdevschain.tech', 'https://xdevschain.tech'])
    assert.equal(p.beneficiary, '0x00000000000000000000000000000000000000AA')
    assert.throws(() => loadConfig({ TRUST_PROXY: 'all' }), /TRUST_PROXY/)
    assert.throws(() => loadConfig({ MAX_BATCH: '0' }), /MAX_BATCH/)
    assert.throws(() => loadConfig({ OPERATOR_MIN_BALANCE: '2', OPERATOR_LOW_BALANCE: '1' }), /OPERATOR_LOW_BALANCE/)
  })

  it('decrypts a Foundry (cast) scrypt keystore and refuses a wrong password', () => {
    const ks = JSON.parse(readFileSync(FIXTURE, 'utf8'))
    assert.equal(decryptKeystore(ks, 'correct horse'), FIXTURE_KEY)
    assert.throws(() => decryptKeystore(ks, 'wrong'), /wrong password/)
  })

  it('loads the key from a file, a keystore (password file / systemd credentials) or the env, never echoing it', () => {
    const dir = mkdtempSync(join(tmpdir(), 'bundler-key-'))
    try {
      const kf = join(dir, 'operator-key')
      writeFileSync(kf, `${FIXTURE_KEY}\n`)
      assert.equal(loadOperatorKey({ BUNDLER_PRIVATE_KEY_FILE: kf }), FIXTURE_KEY)
      assert.equal(loadOperatorKey({ CREDENTIALS_DIRECTORY: dir }), FIXTURE_KEY)
      const pw = join(dir, 'pw')
      writeFileSync(pw, 'correct horse\n')
      assert.equal(loadOperatorKey({ BUNDLER_KEYSTORE: FIXTURE, BUNDLER_KEYSTORE_PASSWORD_FILE: pw }), FIXTURE_KEY)
      assert.equal(loadOperatorKey({ BUNDLER_PRIVATE_KEY: FIXTURE_KEY }), FIXTURE_KEY)
      const bad = join(dir, 'bad')
      writeFileSync(bad, '0xdeadbeefsecret')
      assert.throws(
        () => loadOperatorKey({ BUNDLER_PRIVATE_KEY_FILE: bad }),
        (e: Error) => !e.message.includes('deadbeefsecret'),
      )
      assert.throws(() => loadOperatorKey({ BUNDLER_PRIVATE_KEY: '0x1234' }), (e: Error) => !e.message.includes('0x1234'))
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})

// ------------------------------------------------------------------------------------------------ metrics / store
describe('metrics and state file', () => {
  it('renders Prometheus text', () => {
    const m = new Metrics()
    m.inc('x_total', 'X', { code: '-32500' })
    m.inc('x_total', 'X', { code: '-32500' }, 2)
    m.set('bal_wei', 'B', 10n ** 20n)
    const t = m.render()
    assert.match(t, /# TYPE x_total counter\nx_total\{code="-32500"\} 3/)
    assert.match(t, /bal_wei 100000000000000000000/)
  })

  it('persists bigint-bearing state atomically and reads it back', () => {
    const dir = mkdtempSync(join(tmpdir(), 'bundler-state-'))
    try {
      const s = new JsonStore<{ a: bigint; b: string; n: number }>(join(dir, 'sub', 's.json'))
      s.saveNow({ a: 2n ** 70n, b: 'x', n: 1 })
      assert.deepEqual(s.load(), { a: 2n ** 70n, b: 'x', n: 1 })
      writeFileSync(join(dir, 'sub', 's.json'), '{broken')
      assert.equal(s.load(), undefined)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})

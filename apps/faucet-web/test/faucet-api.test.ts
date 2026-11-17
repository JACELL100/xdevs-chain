import assert from 'node:assert/strict'
import { afterEach, describe, it } from 'node:test'
import { claim, fetchStatus, formatAmount, formatDuration, interpretClaim, interpretStatus, parseRetryAfter } from '../lib/faucet-api.ts'

const HASH = `0x${'ab'.repeat(32)}`
const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

describe('interpretClaim (Worker POST /api/claim shapes)', () => {
  it('success', () => {
    const r = interpretClaim(
      200,
      {
        ok: true,
        txHash: HASH,
        to: '0x1',
        amount: '100',
        amountWei: '1',
        symbol: 'tXDS',
        nonce: 7,
        explorerUrl: `https://explorer.xdevschain.tech/tx/${HASH}`,
      },
      null,
    )
    assert.equal(r.ok, true)
    if (r.ok) {
      assert.equal(r.txHash, HASH)
      assert.equal(r.amount, '100')
      assert.equal(r.explorerUrl, `https://explorer.xdevschain.tech/tx/${HASH}`)
    }
  })

  it('drops non-http explorer URLs', () => {
    const r = interpretClaim(200, { ok: true, txHash: HASH, explorerUrl: 'javascript:alert(1)' }, null)
    assert.equal(r.ok && r.explorerUrl, null)
  })

  it('rate limited: uses Retry-After seconds', () => {
    const r = interpretClaim(429, { ok: false, error: 'rate_limited', message: 'This address already received tXDS.' }, '3600')
    assert.deepEqual(r, {
      ok: false,
      kind: 'rate_limited',
      code: 'rate_limited',
      message: 'This address already received tXDS.',
      retryAfter: 3600,
    })
  })

  it('rate limited without a header falls back to 24 h', () => {
    const r = interpretClaim(429, { ok: false, error: 'rate_limited', message: 'x' }, null)
    assert.equal(!r.ok && r.retryAfter, 86_400)
  })

  it('maps every Worker error code to a UI kind', () => {
    const cases: [number, string, string][] = [
      [503, 'faucet_empty', 'faucet_empty'],
      [503, 'rpc_unavailable', 'network_down'],
      [403, 'captcha_failed', 'captcha'],
      [403, 'captcha_required', 'captcha'],
      [503, 'captcha_unavailable', 'captcha'],
      [400, 'invalid_address', 'invalid_recipient'],
      [400, 'reserved_address', 'invalid_recipient'],
      [400, 'names_not_supported', 'names_not_supported'],
      [503, 'misconfigured', 'unavailable'],
      [418, 'teapot', 'unknown'],
    ]
    for (const [status, code, kind] of cases) {
      const r = interpretClaim(status, { ok: false, error: code, message: 'm' }, null)
      assert.equal(!r.ok && r.kind, kind, code)
    }
  })

  it('non-JSON (Worker not routed yet) is "unreachable"', () => {
    const r = interpretClaim(404, null, null)
    assert.equal(!r.ok && r.kind, 'unreachable')
  })
})

describe('interpretStatus (Worker GET /api/status shapes)', () => {
  it('healthy', () => {
    const s = interpretStatus(200, {
      ok: true,
      address: '0xf39f',
      chainId: 717171,
      symbol: 'tXDS',
      dripAmount: '100',
      minBalance: '1000',
      rateLimit: '1 claim per 24h per address and per IP',
      balance: '1000000',
      balanceWei: '1',
      healthy: true,
    })
    assert.equal(s.reachable, true)
    assert.equal(s.healthy, true)
    assert.equal(s.balance, '1000000')
    assert.equal(s.chainId, 717171)
  })
  it('RPC down (503, balance null)', () => {
    const s = interpretStatus(503, { ok: false, address: '0xf39f', chainId: 717171, balance: null, healthy: false, error: 'rpc_unavailable' })
    assert.equal(s.reachable, true)
    assert.equal(s.healthy, false)
    assert.equal(s.balance, null)
    assert.equal(s.error, 'rpc_unavailable')
  })
  it('not JSON -> unreachable', () => {
    assert.deepEqual(interpretStatus(404, null), { reachable: false, healthy: false })
  })
})

describe('fetch wrappers', () => {
  it('claim posts {address, turnstileToken} and reads Retry-After', async () => {
    let seen: { url: string; body: unknown } | null = null
    globalThis.fetch = (async (url: string, init: RequestInit) => {
      seen = { url, body: JSON.parse(String(init.body)) }
      return new Response(JSON.stringify({ ok: false, error: 'rate_limited', message: 'm' }), {
        status: 429,
        headers: { 'content-type': 'application/json; charset=utf-8', 'Retry-After': '120' },
      })
    }) as unknown as typeof fetch
    const r = await claim('/api', '0xabc', 'tok')
    assert.deepEqual(seen, { url: '/api/claim', body: { address: '0xabc', turnstileToken: 'tok' } })
    assert.equal(!r.ok && r.retryAfter, 120)
  })

  it('claim network failure -> unreachable', async () => {
    globalThis.fetch = (async () => {
      throw new TypeError('Failed to fetch')
    }) as typeof fetch
    const r = await claim('/api', '0xabc', 'tok')
    assert.equal(!r.ok && r.kind, 'unreachable')
  })

  it('status: HTML 404 from Pages -> unreachable', async () => {
    globalThis.fetch = (async () =>
      new Response('<html>404</html>', { status: 404, headers: { 'content-type': 'text/html' } })) as typeof fetch
    assert.deepEqual(await fetchStatus('/api'), { reachable: false, healthy: false })
  })
})

describe('formatting', () => {
  it('parseRetryAfter handles seconds, dates and junk', () => {
    assert.equal(parseRetryAfter('90'), 90)
    assert.equal(parseRetryAfter(new Date(1_000_000 + 30_000).toUTCString(), 1_000_000), 30)
    assert.equal(parseRetryAfter('soon'), undefined)
    assert.equal(parseRetryAfter(null), undefined)
  })
  it('formatDuration', () => {
    assert.equal(formatDuration(30), 'under a minute')
    assert.equal(formatDuration(42 * 60), '42 min')
    assert.equal(formatDuration(3600), '1 h')
    assert.equal(formatDuration(23 * 3600 + 59 * 60 + 59), '23 h 59 min')
  })
  it('formatAmount', () => {
    assert.equal(formatAmount('1000000'), '1,000,000')
    assert.equal(formatAmount('912.5'), '912.5')
    assert.equal(formatAmount(null), '—')
  })
})

import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { prune, remainingSeconds, withCooldown } from '../lib/cooldown.ts'
import { DEMO_ADDRESS, demoClaim, demoTxHash, parseScenario } from '../lib/demo.ts'

describe('cooldown memory', () => {
  const now = 1_700_000_000_000
  it('records and counts down per recipient, case-insensitively', () => {
    const m = withCooldown({}, '0xABC', now + 3_600_000, now)
    assert.equal(remainingSeconds(m, '0xabc', now), 3600)
    assert.equal(remainingSeconds(m, '0xabc', now + 3_599_001), 1)
    assert.equal(remainingSeconds(m, '0xabc', now + 3_600_000), 0)
    assert.equal(remainingSeconds(m, '0xdef', now), 0)
  })
  it('prunes expired and malformed entries', () => {
    assert.deepEqual(prune({ a: now - 1, b: now + 5, c: 'x', D: now + 9 }, now), { b: now + 5, d: now + 9 })
    assert.deepEqual(prune(null, now), {})
    assert.deepEqual(prune(['x'], now), {})
  })
})

describe('demo fixtures', () => {
  it('scenario parsing defaults to ready', () => {
    assert.equal(parseScenario('?scenario=limited'), 'limited')
    assert.equal(parseScenario('?scenario=nope'), 'ready')
    assert.equal(parseScenario(''), 'ready')
  })
  it('fake tx hashes are well-formed and stable', () => {
    assert.match(demoTxHash(DEMO_ADDRESS), /^0x[0-9a-f]{64}$/)
    assert.equal(demoTxHash('x'), demoTxHash('x'))
  })
  it('limited scenario returns a rate-limit with time left', () => {
    const r = demoClaim(DEMO_ADDRESS, 'limited')
    assert.equal(!r.ok && r.kind, 'rate_limited')
    assert.ok(!r.ok && (r.retryAfter ?? 0) > 0)
  })
})

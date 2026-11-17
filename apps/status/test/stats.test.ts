import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  blockIntervals,
  blockTimeStats,
  finality,
  formatAgo,
  headHealth,
  hexToNumber,
  mergeBlocks,
  parseCertificate,
  probeHealth,
  validatorStats,
  worst,
} from '../lib/stats.ts'

const V = (n: number) => `0x${String(n).repeat(40)}`

describe('hexToNumber', () => {
  it('parses quantities and rejects junk', () => {
    assert.equal(hexToNumber('0x2a'), 42)
    assert.equal(hexToNumber('0xAF173'), 717171)
    assert.equal(hexToNumber(7), 7)
    assert.equal(hexToNumber('42'), null)
    assert.equal(hexToNumber(null), null)
    assert.equal(hexToNumber('0x'), null)
  })
})

describe('parseCertificate (xdevs_getCommitCertificate result)', () => {
  it('normalizes signers to lowercase, dedupes and ignores malformed entries', () => {
    const c = parseCertificate({
      height: '0x2a',
      round: '0x1',
      blockHash: '0xabc',
      signatures: [
        { validator: V(1).toUpperCase().replace('0X', '0x'), signature: '0x00' },
        { validator: V(1), signature: '0x00' },
        { validator: 'nope' },
        null,
        { validator: V(2), signature: '0x00' },
      ],
      raw: '0x',
    })
    assert.deepEqual(c, { height: 42, round: 1, blockHash: '0xabc', signers: [V(1), V(2)] })
  })
  it('returns null for null (genesis / unknown block) and garbage', () => {
    assert.equal(parseCertificate(null), null)
    assert.equal(parseCertificate({ height: 'x' }), null)
  })
})

describe('block intervals & stats', () => {
  const blocks = [
    { number: 10, timestamp: 100 },
    { number: 11, timestamp: 102 },
    { number: 12, timestamp: 104 },
    { number: 14, timestamp: 110 }, // gap: 13 missing -> no interval for 14
    { number: 15, timestamp: 113 },
  ]
  it('only measures directly consecutive blocks', () => {
    assert.deepEqual(blockIntervals(blocks), [
      { number: 11, seconds: 2 },
      { number: 12, seconds: 2 },
      { number: 15, seconds: 3 },
    ])
  })
  it('computes mean / median / p95 / min / max', () => {
    const s = blockTimeStats([2, 2, 2, 2, 4].map((seconds, i) => ({ number: i, seconds })))!
    assert.equal(s.count, 5)
    assert.equal(s.mean, 2.4)
    assert.equal(s.median, 2)
    assert.equal(s.min, 2)
    assert.equal(s.max, 4)
    assert.ok(Math.abs(s.p95 - 3.6) < 1e-9)
    assert.equal(blockTimeStats([]), null)
  })
  it('mergeBlocks dedupes by number, sorts and caps', () => {
    const merged = mergeBlocks([{ number: 2, timestamp: 1 }, { number: 1, timestamp: 0 }], [{ number: 2, timestamp: 9 }, { number: 3, timestamp: 4 }], 2)
    assert.deepEqual(merged, [
      { number: 2, timestamp: 9 },
      { number: 3, timestamp: 4 },
    ])
  })
})

describe('finality', () => {
  const cert = (n: number) => ({ height: 1, round: 0, blockHash: '0x', signers: Array.from({ length: n }, (_, i) => V(i + 1)) })
  it('needs strictly more than 2/3', () => {
    assert.equal(finality(cert(3), 4).quorum, true) // 3/4
    assert.equal(finality(cert(2), 3).quorum, false) // exactly 2/3 is not enough
    assert.equal(finality(cert(3), 3).quorum, true)
    assert.equal(finality(cert(6), 9).quorum, false)
    assert.equal(finality(cert(7), 9).quorum, true)
  })
  it('never reports more signers than the set', () => {
    const f = finality(cert(5), 4)
    assert.equal(f.total, 5)
    assert.equal(f.ratio, 1)
  })
})

describe('validatorStats', () => {
  const certs = [
    { height: 100, round: 0, blockHash: '', signers: [V(1), V(2), V(3)] },
    { height: 101, round: 0, blockHash: '', signers: [V(1), V(2)] },
    { height: 102, round: 0, blockHash: '', signers: [V(1), V(3)] },
    { height: 103, round: 0, blockHash: '', signers: [V(1), V(2), V(3)] },
  ]
  it('counts misses and last signed height per validator', () => {
    const s = validatorStats(certs, [{ id: V(4), name: 'offline-node' }])
    const by = Object.fromEntries(s.map((v) => [v.id, v]))
    assert.deepEqual([by[V(1)]!.missed, by[V(1)]!.lastSigned, by[V(1)]!.uptime], [0, 103, 1])
    assert.deepEqual([by[V(2)]!.missed, by[V(2)]!.lastSigned], [1, 103])
    assert.deepEqual([by[V(3)]!.missed, by[V(3)]!.uptime], [1, 0.75])
    // A configured validator that never signed in the window.
    assert.deepEqual([by[V(4)]!.missed, by[V(4)]!.lastSigned, by[V(4)]!.name], [4, null, 'offline-node'])
    assert.ok(s.every((v) => v.window === 4))
  })
  it('handles an empty window', () => {
    assert.deepEqual(validatorStats([], [{ id: V(1) }]), [{ id: V(1), lastSigned: null, missed: 0, window: 0, uptime: 0 }])
  })
})

describe('health', () => {
  it('head freshness', () => {
    assert.equal(headHealth(null), 'unknown')
    assert.equal(headHealth(3), 'up')
    assert.equal(headHealth(25), 'degraded')
    assert.equal(headHealth(120), 'down')
  })
  it('probe + worst', () => {
    assert.equal(probeHealth(true, 100), 'up')
    assert.equal(probeHealth(true, 2000), 'degraded')
    assert.equal(probeHealth(false, null), 'down')
    assert.equal(worst('up', 'degraded', 'unknown'), 'degraded')
    assert.equal(worst('up', 'down', 'degraded'), 'down')
    assert.equal(worst(), 'up')
  })
  it('formatAgo', () => {
    assert.equal(formatAgo(4.9), '4 s')
    assert.equal(formatAgo(64), '1 min 4 s')
    assert.equal(formatAgo(120), '2 min')
    assert.equal(formatAgo(3 * 3600 + 5 * 60), '3 h 5 min')
  })
})

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { demoChain, prng } from '../lib/demo.ts'
import { arrangeIncidents, parseIncidents } from '../lib/incidents.ts'
import { type ComponentState, initialComponents, median, missingHeights, overallStatus, pushLatency } from '../lib/model.ts'
import { chunk, matchBatch } from '../lib/rpc.ts'
import { blockIntervals } from '../lib/stats.ts'

const urls = { rpc: 'r', ws: 'w', explorer: 'e', faucet: 'f', bundler: 'b', website: 's' }
const comps = (over: Partial<Record<keyof typeof urls, ComponentState['health']>> = {}) => {
  const c = initialComponents(urls)
  for (const id of Object.keys(c) as (keyof typeof urls)[]) c[id] = { ...c[id], health: over[id] ?? 'up' }
  return Object.values(c)
}

describe('overallStatus', () => {
  it('says "not launched yet" instead of an outage before the RPC was ever reached', () => {
    const o = overallStatus('unreachable', false, false, comps({ rpc: 'down', ws: 'down' }), 'unknown')
    assert.equal(o.level, 'prelaunch')
    assert.match(o.title, /not launched yet/)
  })
  it('after launch (or after it answered once) an unreachable RPC is an outage', () => {
    assert.equal(overallStatus('unreachable', false, true, comps(), 'unknown').level, 'down')
    assert.equal(overallStatus('unreachable', true, false, comps(), 'unknown').level, 'down')
  })
  it('stalled head is down, slow head or a degraded component is degraded', () => {
    assert.equal(overallStatus('live', true, true, comps(), 'down').level, 'down')
    assert.equal(overallStatus('live', true, true, comps(), 'degraded').level, 'degraded')
    const d = overallStatus('live', true, true, comps({ bundler: 'degraded' }), 'up')
    assert.equal(d.level, 'degraded')
    assert.match(d.title, /Bundler/)
    assert.equal(overallStatus('live', true, true, comps({ ws: 'down' }), 'up').level, 'down')
    assert.equal(overallStatus('live', true, true, comps(), 'up').title, 'All systems operational')
    assert.equal(overallStatus('connecting', false, false, comps(), 'unknown').level, 'connecting')
  })
})

describe('helpers', () => {
  it('missingHeights fetches only the window we lack', () => {
    assert.deepEqual(missingHeights([8, 9, 10], 12, 5), [11, 12])
    assert.deepEqual(missingHeights([], 2, 5), [0, 1, 2])
    assert.deepEqual(missingHeights([1, 2, 3], 3, 2), [])
  })
  it('pushLatency keeps the newest, median is exact', () => {
    assert.deepEqual(pushLatency([1, 2, 3], 4.4, 3), [2, 3, 4])
    assert.equal(median([5, 1, 3]), 3)
    assert.equal(median([4, 1, 3, 2]), 2.5)
    assert.equal(median([]), null)
  })
  it('matchBatch maps replies by id in any order, errors included', () => {
    const r = matchBatch([7, 8, 9], [
      { jsonrpc: '2.0', id: 9, error: { code: -32601, message: 'Method not found' } },
      { jsonrpc: '2.0', id: 7, result: '0x1' },
    ])
    assert.deepEqual(r[0], { ok: true, value: '0x1' })
    assert.equal(!r[1]!.ok && r[1]!.error.message, 'missing response')
    assert.equal(!r[2]!.ok && r[2]!.error.code, -32601)
  })
  it('chunk splits at the batch limit', () => {
    assert.deepEqual(chunk([1, 2, 3, 4, 5], 2), [[1, 2], [3, 4], [5]])
  })
})

describe('incidents', () => {
  const now = Date.parse('2026-12-10T12:00:00Z')
  it('the shipped content/incidents.json is valid (and empty for now)', () => {
    const raw = JSON.parse(readFileSync(new URL('../content/incidents.json', import.meta.url), 'utf8'))
    const { incidents, rejected } = parseIncidents(raw)
    assert.equal(rejected, 0)
    assert.deepEqual(incidents, [])
  })
  it('drops malformed entries and groups the rest', () => {
    const { incidents, rejected } = parseIncidents([
      { id: 'a', title: 'RPC 502s', kind: 'incident', severity: 'major', status: 'investigating', startsAt: '2026-12-10T11:00:00Z', updates: [] },
      { id: 'b', title: 'Upgrade', kind: 'maintenance', status: 'scheduled', startsAt: '2026-12-12T10:00:00Z', endsAt: '2026-12-12T11:00:00Z' },
      { id: 'c', title: 'Old', kind: 'incident', status: 'resolved', startsAt: '2026-12-01T10:00:00Z', endsAt: '2026-12-01T11:00:00Z' },
      { id: 'd', title: 'Ancient', kind: 'incident', status: 'resolved', startsAt: '2026-01-01T10:00:00Z' },
      { id: 'e', title: 'Bad', kind: 'oops', status: 'resolved', startsAt: '2026-12-01T10:00:00Z' },
      { id: 'f', title: 'No date', kind: 'incident', status: 'resolved', startsAt: 'yesterday' },
    ])
    assert.equal(rejected, 2)
    const g = arrangeIncidents(incidents, now)
    assert.deepEqual(g.active.map((i) => i.id), ['a'])
    assert.deepEqual(g.upcoming.map((i) => i.id), ['b'])
    assert.deepEqual(g.past.map((i) => i.id), ['c'])
  })
})

describe('demo simulation', () => {
  it('is reproducible and produces a consecutive ~2 s chain with certificates', () => {
    assert.equal(prng(1)(), prng(1)())
    const now = Date.parse('2026-12-10T12:00:00Z')
    const a = demoChain(now)
    const b = demoChain(now)
    assert.deepEqual(a.blocks, b.blocks)
    assert.equal(a.blocks.length, 60)
    assert.equal(blockIntervals(a.blocks).length, 59)
    assert.ok(blockIntervals(a.blocks).every((i) => i.seconds >= 2 && i.seconds <= 4))
    assert.equal(a.certs.length, 60)
    const { block, cert } = a.next(now + 2000)
    assert.equal(block.number, a.blocks[59]!.number + 1)
    assert.equal(cert.height, block.number)
  })
})

// NEXT_PUBLIC_DEMO=1: a seeded, in-browser simulation of a healthy 4-validator Enclave, so the page can be
// reviewed before launch. Everything rendered from it carries a "Demo data" label.

import type { BlockSample, CommitCertificate } from './stats.ts'
import type { ComponentId, ComponentState } from './model.ts'
import type { Incident } from './incidents.ts'

export const DEMO_VALIDATORS = [
  { id: '0x3f1c9a2b7d4e8f60a1b2c3d4e5f60718293a4b5c', name: 'xdevs-val-1' },
  { id: '0x7a2e4c6b8d0f1e3a5c7b9d1f2e4a6c8b0d2f4e6a', name: 'xdevs-val-2' },
  { id: '0xa94d1e7b3c5f9a2e8d4b6c0f1a3e5d7c9b2f4a6e', name: 'xdevs-val-3' },
  { id: '0xd05b8e2a4c6f1d3b5a7e9c0f2b4d6a8e1c3f5b7d', name: 'xdevs-val-4' },
]

/** Mulberry32: tiny seeded PRNG so screenshots are reproducible. */
export function prng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export interface DemoChain {
  blocks: BlockSample[]
  certs: CommitCertificate[]
  next(now: number): { block: BlockSample; cert: CommitCertificate }
}

const fakeHash = (n: number) => `0x${(n * 2654435761).toString(16).padStart(8, '0').repeat(8).slice(0, 64)}`

/** History of `count` blocks ending at `now` plus a generator for the next ones. */
export function demoChain(now: number, count = 60, seed = 7): DemoChain {
  const rand = prng(seed)
  const interval = () => {
    const r = rand()
    if (r > 0.95) return 4 // an extra consensus round
    if (r > 0.86) return 3
    return 2
  }
  const signersFor = (height: number) => {
    const out: string[] = []
    for (const [i, v] of DEMO_VALIDATORS.entries()) {
      // val-3 is flaky (misses ~1 in 9), val-4 missed a short streak recently.
      const miss = (i === 2 && rand() < 0.11) || (i === 3 && height % 97 > 92)
      if (!miss) out.push(v.id)
    }
    return out
  }
  const head0 = 1_284_000 + Math.floor((now / 1000) % 10_000)
  const blocks: BlockSample[] = []
  let ts = Math.floor(now / 1000)
  const times: number[] = []
  for (let i = 0; i < count; i++) {
    times.unshift(ts)
    ts -= interval()
  }
  times.forEach((t, i) => blocks.push({ number: head0 - count + 1 + i, timestamp: t, txCount: Math.floor(rand() * 40) }))
  const certs = blocks.map((b) => ({ height: b.number, round: 0, blockHash: fakeHash(b.number), signers: signersFor(b.number) }))
  let last = blocks[blocks.length - 1]!
  return {
    blocks,
    certs,
    next(nowMs: number) {
      const number = last.number + 1
      const timestamp = Math.max(last.timestamp + 1, Math.floor(nowMs / 1000))
      const block = { number, timestamp, txCount: Math.floor(rand() * 40) }
      last = block
      return { block, cert: { height: number, round: rand() > 0.95 ? 1 : 0, blockHash: fakeHash(number), signers: signersFor(number) } }
    },
  }
}

export function demoComponents(base: Record<ComponentId, ComponentState>, now: number): Record<ComponentId, ComponentState> {
  const set = (id: ComponentId, health: ComponentState['health'], latencyMs: number | null, detail: string) => ({
    ...base[id],
    health,
    latencyMs,
    detail,
    checkedAt: now,
  })
  return {
    rpc: set('rpc', 'up', 84, 'eth_chainId 0xaf173'),
    ws: set('ws', 'up', 61, 'newHeads streaming'),
    explorer: set('explorer', 'up', 142, 'Indexed to head (lag 1 block)'),
    faucet: set('faucet', 'up', 118, '18,450 tXDS available'),
    bundler: set('bundler', 'degraded', 1840, 'Slow responses (1.8 s)'),
    website: set('website', 'up', 96, 'Reachable'),
  }
}

export function demoIncidents(now: number): Incident[] {
  const iso = (ms: number) => new Date(ms).toISOString()
  return [
    {
      id: 'demo-maintenance',
      title: 'RPC node upgrade to v0.4.2',
      kind: 'maintenance',
      status: 'scheduled',
      components: ['rpc', 'ws'],
      startsAt: iso(now + 2 * 86_400_000),
      endsAt: iso(now + 2 * 86_400_000 + 30 * 60_000),
      updates: [{ at: iso(now - 3_600_000), text: 'Rolling restart of the public RPC nodes. Requests may fail for up to a minute each.' }],
    },
    {
      id: 'demo-bundler',
      title: 'Bundler responding slowly',
      kind: 'incident',
      severity: 'minor',
      status: 'monitoring',
      components: ['bundler'],
      startsAt: iso(now - 50 * 60_000),
      updates: [
        { at: iso(now - 10 * 60_000), text: 'A fix is deployed; we are watching response times.' },
        { at: iso(now - 50 * 60_000), text: 'eth_estimateUserOperationGas is taking 2 s or more.' },
      ],
    },
    {
      id: 'demo-past',
      title: 'Explorer indexer lagging behind head',
      kind: 'incident',
      severity: 'minor',
      status: 'resolved',
      components: ['explorer'],
      startsAt: iso(now - 4 * 86_400_000),
      endsAt: iso(now - 4 * 86_400_000 + 40 * 60_000),
      updates: [{ at: iso(now - 4 * 86_400_000 + 40 * 60_000), text: 'Resolved: the indexer caught up after a database restart.' }],
    },
  ]
}

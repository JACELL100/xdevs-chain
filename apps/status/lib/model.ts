// Status-page state model and the pure decisions on top of it (unit tested).

import type { BlockSample, CommitCertificate, Health } from './stats.ts'

export type ComponentId = 'rpc' | 'ws' | 'explorer' | 'faucet' | 'bundler' | 'website'

export interface ComponentState {
  id: ComponentId
  label: string
  description: string
  url: string
  health: Health
  latencyMs: number | null
  detail: string
  checkedAt: number | null
}

export type Phase = 'connecting' | 'live' | 'unreachable'

export interface NetworkState {
  phase: Phase
  /** True once the RPC answered at least once in this session. */
  everReached: boolean
  chainId: number | null
  /** Newest block we know (from polling or the WS newHeads stream). */
  head: BlockSample | null
  /** Recent blocks, ascending, consecutive where possible. */
  blocks: BlockSample[]
  /** Recent commit certificates, ascending by height. */
  certs: CommitCertificate[]
  /** Whether the node serves xdevs_getCommitCertificate. */
  certSupport: 'unknown' | 'yes' | 'no'
  validatorCount: number
  validatorSource: 'rpc' | 'config'
  peers: number | null
  /** Recent browser -> RPC round trips in ms (newest last). */
  latencies: number[]
  components: Record<ComponentId, ComponentState>
  updatedAt: number | null
}

export const COMPONENT_ORDER: ComponentId[] = ['rpc', 'ws', 'explorer', 'faucet', 'bundler', 'website']

export function initialComponents(urls: Record<ComponentId, string>): Record<ComponentId, ComponentState> {
  const meta: Record<ComponentId, [string, string]> = {
    rpc: ['JSON-RPC', 'Public HTTPS endpoint'],
    ws: ['WebSocket', 'Subscriptions (newHeads, logs)'],
    explorer: ['Explorer API', 'Indexer behind the explorer'],
    faucet: ['Faucet', 'tXDS drips'],
    bundler: ['Bundler', 'ERC-4337 smart accounts'],
    website: ['Website & docs', 'xdevschain.tech'],
  }
  const out = {} as Record<ComponentId, ComponentState>
  for (const id of COMPONENT_ORDER) {
    out[id] = { id, label: meta[id][0], description: meta[id][1], url: urls[id], health: 'unknown', latencyMs: null, detail: 'Checking…', checkedAt: null }
  }
  return out
}

/** Heights to fetch so we end up with the last `max` blocks up to `head` (skipping ones we have). */
export function missingHeights(have: number[], head: number, max: number): number[] {
  const known = new Set(have)
  const from = Math.max(0, head - max + 1)
  const out: number[] = []
  for (let n = from; n <= head; n++) if (!known.has(n)) out.push(n)
  return out
}

export type Overall =
  | { level: 'prelaunch'; title: string }
  | { level: 'connecting'; title: string }
  | { level: 'up' | 'degraded' | 'down'; title: string }

/**
 * Headline. Before launch (RPC never reached and not flagged as launched) the page says so
 * instead of painting everything red.
 */
export function overallStatus(
  phase: Phase,
  everReached: boolean,
  launched: boolean,
  components: ComponentState[],
  head: Health,
): Overall {
  if (phase === 'connecting') return { level: 'connecting', title: 'Checking the network…' }
  if (phase === 'unreachable' && !everReached && !launched) return { level: 'prelaunch', title: 'Enclave testnet: not launched yet' }
  if (phase === 'unreachable') return { level: 'down', title: 'Major outage: the Enclave RPC is unreachable' }
  if (head === 'down') return { level: 'down', title: 'Block production has stalled' }
  const core = components.filter((c) => c.id === 'rpc' || c.id === 'ws')
  const bad = components.filter((c) => c.health === 'down' || c.health === 'degraded')
  if (core.some((c) => c.health === 'down')) return { level: 'down', title: 'Major outage' }
  if (head === 'degraded' || bad.length > 0) {
    const names = bad.map((c) => c.label)
    return {
      level: 'degraded',
      title: head === 'degraded' ? 'Blocks are arriving slowly' : `Degraded: ${names.join(', ')}`,
    }
  }
  return { level: 'up', title: 'All systems operational' }
}

export function pushLatency(list: number[], ms: number, keep = 30): number[] {
  return [...list, Math.round(ms)].slice(-keep)
}

export function median(values: number[]): number | null {
  if (values.length === 0) return null
  const s = [...values].sort((a, b) => a - b)
  const mid = Math.floor(s.length / 2)
  return s.length % 2 ? s[mid]! : (s[mid - 1]! + s[mid]!) / 2
}

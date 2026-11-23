'use client'

/**
 * Live Enclave numbers, read in the browser straight from the public RPC (CORS: *) and,
 * when the API allows this origin, the explorer API. One shared poller for every widget;
 * it pauses while the tab is hidden. No number is ever invented: until data arrives the
 * widgets show a placeholder.
 */
import { useSyncExternalStore } from 'react'
import { ENCLAVE } from './site'

export type LiveBlock = { number: number; hash: string; txs: number; time: number; gasUsed: number }
export type ChainState = {
  status: 'connecting' | 'live' | 'offline'
  height: number | null
  blockTimeMs: number | null
  blocks: LiveBlock[]
  totalTransactions: number | null
  totalAddresses: number | null
}

let state: ChainState = {
  status: 'connecting',
  height: null,
  blockTimeMs: null,
  blocks: [],
  totalTransactions: null,
  totalAddresses: null,
}
const serverState = state
const listeners = new Set<() => void>()
const emit = (patch: Partial<ChainState>) => {
  state = { ...state, ...patch }
  listeners.forEach((l) => l())
}

type RawBlock = { number: string; hash: string; timestamp: string; gasUsed: string; transactions: unknown[] }

async function rpc<T>(body: object | object[]): Promise<T> {
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), 6000)
  try {
    const res = await fetch(ENCLAVE.rpcUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal: ctrl.signal,
      cache: 'no-store',
    })
    if (!res.ok) throw new Error(String(res.status))
    return (await res.json()) as T
  } finally {
    clearTimeout(t)
  }
}

const hex = (n: number) => '0x' + n.toString(16)
const toBlock = (b: RawBlock): LiveBlock => ({
  number: parseInt(b.number, 16),
  hash: b.hash,
  txs: b.transactions.length,
  time: parseInt(b.timestamp, 16),
  gasUsed: parseInt(b.gasUsed, 16),
})

let timer: ReturnType<typeof setTimeout> | undefined
let fails = 0
let lastAvgAt = 0
let lastStatsAt = 0

async function tick() {
  try {
    const r = await rpc<{ result?: RawBlock }>({ jsonrpc: '2.0', id: 1, method: 'eth_getBlockByNumber', params: ['latest', false] })
    if (!r.result) throw new Error('no block')
    const b = toBlock(r.result)
    fails = 0
    const known = state.blocks
    if (!known.length || b.number > known[0].number) {
      // Fill a small gap (a missed poll) so the stream stays continuous.
      const gap = known.length ? b.number - known[0].number - 1 : 0
      const missing: LiveBlock[] = []
      if (gap > 0 && gap <= 3) {
        const nums = Array.from({ length: gap }, (_, i) => known[0].number + 1 + i)
        const res = await rpc<{ result?: RawBlock }[]>(
          nums.map((n, i) => ({ jsonrpc: '2.0', id: i, method: 'eth_getBlockByNumber', params: [hex(n), false] })),
        ).catch(() => [])
        for (const x of res) if (x?.result) missing.push(toBlock(x.result))
        missing.sort((a, c) => c.number - a.number)
      }
      emit({ status: 'live', height: b.number, blocks: [b, ...missing, ...known].slice(0, 12) })
    } else if (state.status !== 'live') emit({ status: 'live' })

    const now = Date.now()
    if (now - lastAvgAt > 30000) {
      lastAvgAt = now
      const back = await rpc<{ result?: RawBlock }>({
        jsonrpc: '2.0',
        id: 2,
        method: 'eth_getBlockByNumber',
        params: [hex(Math.max(0, b.number - 60)), false],
      }).catch(() => null)
      if (back?.result) {
        const old = toBlock(back.result)
        if (b.number > old.number) emit({ blockTimeMs: Math.round(((b.time - old.time) * 1000) / (b.number - old.number)) })
      }
    }
    if (now - lastStatsAt > 60000) {
      lastStatsAt = now
      // The explorer API may not allow this origin yet; the RPC numbers show either way.
      fetch(`${ENCLAVE.explorerApiUrl}/api/v1/stats`, { cache: 'no-store' })
        .then((res) => (res.ok ? res.json() : null))
        .then((s: { totalTransactions?: number; totalAddresses?: number } | null) => {
          if (s && typeof s.totalTransactions === 'number')
            emit({ totalTransactions: s.totalTransactions, totalAddresses: s.totalAddresses ?? null })
        })
        .catch(() => {})
    }
  } catch {
    fails++
    if (fails >= 3) emit({ status: 'offline' })
  }
  schedule()
}

function schedule() {
  clearTimeout(timer)
  if (!listeners.size || document.hidden) return
  timer = setTimeout(tick, fails ? Math.min(15000, 2000 * 2 ** fails) : 2000)
}

function onVisibility() {
  if (!document.hidden && listeners.size) {
    clearTimeout(timer)
    tick()
  }
}

function subscribe(l: () => void) {
  listeners.add(l)
  if (listeners.size === 1) {
    document.addEventListener('visibilitychange', onVisibility)
    tick()
  }
  return () => {
    listeners.delete(l)
    if (!listeners.size) {
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }
}

export function useChain(): ChainState {
  return useSyncExternalStore(subscribe, () => state, () => serverState)
}

export const fmtInt = (n: number | null | undefined) => (n == null ? '—' : n.toLocaleString('en-US'))

// Reachability of the node (RPC) and the indexer (API), shared by every page. A failed request marks its service
// down; pages then switch to the RPC-only view or to the "network not reachable yet" state. Probes back off so an
// offline network costs one request per service every 30-120 s, never a stream of console errors.

import { useSyncExternalStore } from 'react'
import { config } from './config.ts'

export type Reach = 'unknown' | 'up' | 'down'
export interface NetState {
  rpc: Reach
  api: Reach
  chainId: number | null
  head: number | null
}

let state: NetState = { rpc: config.demo ? 'up' : 'unknown', api: config.demo ? 'up' : 'unknown', chainId: config.demo ? config.chainId : null, head: null }
const listeners = new Set<() => void>()
const emit = () => listeners.forEach((l) => l())

export function setNet(patch: Partial<NetState>) {
  const next = { ...state, ...patch }
  if (next.rpc === state.rpc && next.api === state.api && next.chainId === state.chainId && next.head === state.head) return
  state = next
  emit()
}

export const getNet = () => state
export function useNet(): NetState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => state,
    () => state,
  )
}

/** True for transport failures (DNS, refused, timeout, CORS, 5xx) as opposed to "not found" / bad input. */
export function isUnreachable(e: unknown): boolean {
  if (!e || typeof e !== 'object') return false
  const err = e as { name?: string; status?: number; code?: string; message?: string; details?: string }
  if (err.name === 'AbortError') return false
  if (err.code === 'NETWORK') return true
  if (typeof err.status === 'number' && (err.status === 0 || err.status >= 500)) return true
  if (err.name === 'HttpRequestError' || err.name === 'TimeoutError' || err.name === 'WebSocketRequestError') return true
  return /fetch failed|failed to fetch|networkerror|load failed|timed? ?out|ECONNREFUSED|ENOTFOUND/i.test(`${err.message ?? ''} ${err.details ?? ''}`)
}

/** Both services down (and not in demo mode): the whole explorer shows the "launching soon" state. */
export const networkDown = (s: NetState) => !config.demo && s.rpc === 'down' && s.api !== 'up'

// --------------------------------------------------------------------------------------------------------------
// Probing (browser only)
// --------------------------------------------------------------------------------------------------------------

const RETRY = [15_000, 30_000, 60_000, 120_000]
let probing = false
let retryTimer: ReturnType<typeof setTimeout> | null = null
let retries = 0

async function probeRpcOnce(): Promise<void> {
  try {
    const res = await fetch(config.rpcUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify([
        { jsonrpc: '2.0', id: 1, method: 'eth_chainId', params: [] },
        { jsonrpc: '2.0', id: 2, method: 'eth_blockNumber', params: [] },
      ]),
      signal: AbortSignal.timeout(8000),
    })
    const body = (await res.json()) as { id?: number; result?: string }[] | { id?: number; result?: string }
    const list = Array.isArray(body) ? body : [body]
    const chain = list.find((r) => r.id === 1)?.result
    const head = list.find((r) => r.id === 2)?.result
    setNet({ rpc: res.ok ? 'up' : 'down', chainId: chain ? Number(BigInt(chain)) : state.chainId, head: head ? Number(BigInt(head)) : state.head })
  } catch {
    setNet({ rpc: 'down' })
  }
}

async function probeApiOnce(): Promise<void> {
  try {
    const res = await fetch(`${config.apiUrl}/health`, { signal: AbortSignal.timeout(8000) })
    setNet({ api: res.ok ? 'up' : 'down' })
  } catch {
    setNet({ api: 'down' })
  }
}

/** Probes both services now, then again with back-off while either is down. Returns a stop function. */
export function startNetProbe(): () => void {
  if (config.demo || typeof window === 'undefined' || probing) return () => {}
  probing = true
  const run = async () => {
    retryTimer = null
    await Promise.all([state.rpc !== 'up' ? probeRpcOnce() : null, state.api !== 'up' ? probeApiOnce() : null])
    if (!probing) return
    if (state.rpc === 'down' || state.api === 'down') retryTimer = setTimeout(run, RETRY[Math.min(retries++, RETRY.length - 1)])
    else retries = 0
  }
  void run()
  const unsub = (() => {
    // A page that finds a service down restarts the slow re-probe.
    const l = () => {
      if (probing && !retryTimer && (state.rpc === 'down' || state.api === 'down')) retryTimer = setTimeout(run, RETRY[Math.min(retries++, RETRY.length - 1)])
    }
    listeners.add(l)
    return () => listeners.delete(l)
  })()
  return () => {
    probing = false
    unsub()
    if (retryTimer) clearTimeout(retryTimer)
    retryTimer = null
  }
}

/** Manual "try again": probe immediately. */
export function reprobe(): void {
  retries = 0
  void Promise.all([probeRpcOnce(), probeApiOnce()])
}

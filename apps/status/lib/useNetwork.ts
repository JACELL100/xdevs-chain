'use client'

// Live network state for the dashboard: polls the RPC (batched), follows newHeads over WebSocket,
// pulls commit certificates, and probes the public components. All requests come from the browser,
// so the latency shown is the visitor's real round trip.

import { useEffect, useRef, useState } from 'react'
import { config } from './config'
import { demoChain, demoComponents } from './demo'
import { type ComponentId, type ComponentState, type NetworkState, initialComponents, missingHeights, pushLatency } from './model'
import { METHOD_NOT_FOUND, type RpcCall, chunk, rpcBatch, toHex } from './rpc'
import { type BlockSample, type CommitCertificate, hexToNumber, mergeBlocks, parseCertificate, probeHealth } from './stats'

const POLL_MS = 4_000
const PROBE_MS = 30_000
const BLOCK_WINDOW = 60
const CERT_WINDOW = 50

const URLS: Record<ComponentId, string> = {
  rpc: config.rpcUrl,
  ws: config.wsUrl,
  explorer: config.explorerApiUrl,
  faucet: config.faucetUrl,
  bundler: config.bundlerUrl,
  website: config.siteUrl,
}

function initialState(): NetworkState {
  return {
    phase: 'connecting',
    everReached: false,
    chainId: null,
    head: null,
    blocks: [],
    certs: [],
    certSupport: 'unknown',
    validatorCount: config.validatorCount,
    validatorSource: 'config',
    peers: null,
    latencies: [],
    components: initialComponents(URLS),
    updatedAt: null,
  }
}

function toBlock(raw: unknown): BlockSample | null {
  const r = raw as Record<string, unknown> | null
  if (!r) return null
  const number = hexToNumber(r.number)
  const timestamp = hexToNumber(r.timestamp)
  if (number === null || timestamp === null) return null
  return { number, timestamp, txCount: Array.isArray(r.transactions) ? r.transactions.length : undefined }
}

function mergeCerts(a: CommitCertificate[], b: CommitCertificate[]): CommitCertificate[] {
  const m = new Map<number, CommitCertificate>()
  for (const c of a) m.set(c.height, c)
  for (const c of b) m.set(c.height, c)
  return [...m.values()].sort((x, y) => x.height - y.height).slice(-CERT_WINDOW)
}

async function timed<T>(fn: () => Promise<T>): Promise<{ value: T; ms: number }> {
  const t0 = performance.now()
  const value = await fn()
  return { value, ms: performance.now() - t0 }
}

async function fetchJson(url: string, init?: RequestInit, timeoutMs = 8000): Promise<{ status: number; body: unknown; ms: number }> {
  const ctrl = new AbortController()
  const timer = setTimeout(() => ctrl.abort(), timeoutMs)
  try {
    const { value, ms } = await timed(async () => {
      const res = await fetch(url, { ...init, signal: ctrl.signal, cache: 'no-store' })
      let body: unknown = null
      if ((res.headers.get('content-type') ?? '').includes('json')) body = await res.json().catch(() => null)
      return { status: res.status, body }
    })
    return { ...value, ms }
  } finally {
    clearTimeout(timer)
  }
}

type Probe = Pick<ComponentState, 'health' | 'latencyMs' | 'detail'>

async function probeComponent(id: ComponentId): Promise<Probe> {
  try {
    switch (id) {
      case 'explorer': {
        const r = await fetchJson(`${config.explorerApiUrl}/health`)
        const b = r.body as { ok?: boolean; lagBlocks?: number | null } | null
        if (!b) return { health: 'down', latencyMs: null, detail: `HTTP ${r.status}` }
        if (b.ok) return { health: probeHealth(true, r.ms), latencyMs: r.ms, detail: `Indexed to head (lag ${b.lagBlocks ?? 0})` }
        return { health: 'degraded', latencyMs: r.ms, detail: `Indexer lagging (${b.lagBlocks ?? '?'} blocks)` }
      }
      case 'faucet': {
        const r = await fetchJson(`${config.faucetUrl}/api/status`)
        const b = r.body as { healthy?: boolean; balance?: string | null; symbol?: string; error?: string } | null
        if (!b) return { health: 'down', latencyMs: null, detail: `HTTP ${r.status}` }
        if (b.healthy) {
          const bal = b.balance ? `${Number(b.balance).toLocaleString('en-US', { maximumFractionDigits: 0 })} ${b.symbol ?? 'tXDS'} available` : 'Healthy'
          return { health: probeHealth(true, r.ms), latencyMs: r.ms, detail: bal }
        }
        return { health: 'degraded', latencyMs: r.ms, detail: b.error === 'rpc_unavailable' ? 'Cannot reach the RPC' : 'Paused for refill' }
      }
      case 'bundler': {
        const r = await fetchJson(config.bundlerUrl, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_supportedEntryPoints', params: [] }),
        })
        const b = r.body as { result?: unknown } | null
        if (b && Array.isArray(b.result)) {
          return { health: probeHealth(true, r.ms), latencyMs: r.ms, detail: `${b.result.length} entry point${b.result.length === 1 ? '' : 's'}` }
        }
        return { health: b ? 'degraded' : 'down', latencyMs: b ? r.ms : null, detail: b ? 'Unexpected answer' : `HTTP ${r.status}` }
      }
      case 'website': {
        // Cross-origin without CORS: an opaque response still proves the site answers.
        const { ms } = await timed(() => fetch(config.siteUrl, { mode: 'no-cors', cache: 'no-store' }))
        return { health: probeHealth(true, ms, 3000), latencyMs: ms, detail: 'Reachable' }
      }
      default:
        return { health: 'unknown', latencyMs: null, detail: '' }
    }
  } catch {
    return { health: 'down', latencyMs: null, detail: 'Unreachable' }
  }
}

export function useNetwork(): NetworkState {
  const [state, setState] = useState<NetworkState>(initialState)
  const ref = useRef(state)
  ref.current = state

  useEffect(() => {
    let alive = true
    const timers: ReturnType<typeof setInterval>[] = []
    const update = (fn: (s: NetworkState) => NetworkState) => {
      if (alive) setState((s) => fn(s))
    }
    const visible = () => document.visibilityState === 'visible'

    // ---------- demo ----------
    if (config.demo) {
      const now = Date.now()
      const chain = demoChain(now)
      let lat = [64, 71, 58, 92, 77, 69, 83, 61, 74, 66]
      setState((s) => ({
        ...s,
        phase: 'live',
        everReached: true,
        chainId: 717171,
        blocks: chain.blocks,
        head: chain.blocks[chain.blocks.length - 1]!,
        certs: chain.certs.slice(-CERT_WINDOW),
        certSupport: 'yes',
        peers: 7,
        latencies: lat,
        components: demoComponents(s.components, now),
        updatedAt: now,
      }))
      timers.push(
        setInterval(() => {
          const { block, cert } = chain.next(Date.now())
          lat = pushLatency(lat, 55 + Math.random() * 45)
          update((s) => ({
            ...s,
            head: block,
            blocks: mergeBlocks(s.blocks, [block], BLOCK_WINDOW),
            certs: mergeCerts(s.certs, [cert]),
            latencies: lat,
            updatedAt: Date.now(),
          }))
        }, 2000),
      )
      return () => {
        alive = false
        timers.forEach(clearInterval)
      }
    }

    // ---------- live RPC polling ----------
    let inFlight = false
    let validatorsTried = false

    const setComponent = (id: ComponentId, p: Probe) =>
      update((s) => ({ ...s, components: { ...s.components, [id]: { ...s.components[id], ...p, checkedAt: Date.now() } } }))

    const poll = async () => {
      if (inFlight) return
      inFlight = true
      try {
        const { results, latencyMs } = await rpcBatch(config.rpcUrl, [
          { method: 'eth_chainId' },
          { method: 'eth_blockNumber' },
          { method: 'net_peerCount' },
        ])
        const chainId = results[0]?.ok ? hexToNumber(results[0].value) : null
        const headNum = results[1]?.ok ? hexToNumber(results[1].value) : null
        const peers = results[2]?.ok ? hexToNumber(results[2].value) : null
        if (headNum === null) throw new Error('no head')
        update((s) => ({
          ...s,
          phase: 'live',
          everReached: true,
          chainId,
          peers,
          latencies: pushLatency(s.latencies, latencyMs),
          updatedAt: Date.now(),
        }))
        setComponent('rpc', {
          health: chainId === null ? 'degraded' : probeHealth(true, latencyMs),
          latencyMs,
          detail: chainId === null ? 'eth_chainId failed' : `chain ${chainId}${chainId !== 717171 ? ' (unexpected)' : ''}`,
        })

        // New blocks + certificates for heights we do not have yet.
        const cur = ref.current
        const want = missingHeights(cur.blocks.map((b) => b.number), headNum, BLOCK_WINDOW)
        const certsOn = cur.certSupport !== 'no'
        const calls: { call: RpcCall; kind: 'block' | 'cert' }[] = []
        for (const n of want) {
          calls.push({ call: { method: 'eth_getBlockByNumber', params: [toHex(n), false] }, kind: 'block' })
          if (certsOn && n > headNum - CERT_WINDOW) calls.push({ call: { method: 'xdevs_getCommitCertificate', params: [toHex(n)] }, kind: 'cert' })
        }
        if (!validatorsTried) calls.push({ call: { method: 'xdevs_validators', params: [] }, kind: 'cert' })
        const blocks: BlockSample[] = []
        const certs: CommitCertificate[] = []
        let certSupport = cur.certSupport
        let validatorCount: number | null = null
        for (const part of chunk(calls, 50)) {
          const { results: rs } = await rpcBatch(config.rpcUrl, part.map((p) => p.call))
          rs.forEach((r, i) => {
            const { call, kind } = part[i]!
            if (call.method === 'xdevs_validators') {
              validatorsTried = true
              if (r.ok && Array.isArray(r.value) && r.value.length > 0) validatorCount = r.value.length
              return
            }
            if (kind === 'block') {
              const b = r.ok ? toBlock(r.value) : null
              if (b) blocks.push(b)
            } else if (!r.ok && r.error.code === METHOD_NOT_FOUND) {
              certSupport = 'no'
            } else if (r.ok) {
              const c = parseCertificate(r.value)
              if (c) {
                certs.push(c)
                certSupport = 'yes'
              }
            }
          })
        }
        update((s) => {
          const merged = mergeBlocks(s.blocks, blocks, BLOCK_WINDOW)
          const newest = merged[merged.length - 1] ?? null
          return {
            ...s,
            blocks: merged,
            head: newest && (!s.head || newest.number >= s.head.number) ? newest : s.head,
            certs: mergeCerts(s.certs, certs),
            certSupport,
            ...(validatorCount ? { validatorCount, validatorSource: 'rpc' as const } : {}),
          }
        })
      } catch {
        update((s) => ({ ...s, phase: 'unreachable', updatedAt: Date.now() }))
        setComponent('rpc', { health: 'down', latencyMs: null, detail: 'Unreachable' })
      } finally {
        inFlight = false
      }
    }

    const probeAll = () => {
      for (const id of ['explorer', 'faucet', 'bundler', 'website'] as ComponentId[]) {
        void probeComponent(id).then((p) => setComponent(id, p))
      }
    }

    // ---------- WebSocket newHeads ----------
    let ws: WebSocket | null = null
    let wsRetry = 0
    let wsTimer: ReturnType<typeof setTimeout> | null = null
    let lastHeadAt = 0
    const connectWs = () => {
      if (!alive) return
      let opened = 0
      try {
        ws = new WebSocket(config.wsUrl)
      } catch {
        setComponent('ws', { health: 'down', latencyMs: null, detail: 'Unreachable' })
        return
      }
      const t0 = performance.now()
      ws.onopen = () => {
        opened = performance.now() - t0
        ws?.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_subscribe', params: ['newHeads'] }))
      }
      ws.onmessage = (ev) => {
        let msg: { id?: number; result?: unknown; error?: unknown; method?: string; params?: { result?: unknown } }
        try {
          msg = JSON.parse(String(ev.data))
        } catch {
          return
        }
        if (msg.id === 1) {
          if (typeof msg.result === 'string') {
            wsRetry = 0
            setComponent('ws', { health: probeHealth(true, opened), latencyMs: opened, detail: 'Subscribed to newHeads' })
          } else setComponent('ws', { health: 'degraded', latencyMs: opened, detail: 'Subscription refused' })
          return
        }
        if (msg.method === 'eth_subscription') {
          const b = toBlock(msg.params?.result)
          if (!b) return
          lastHeadAt = Date.now()
          update((s) => ({
            ...s,
            head: !s.head || b.number > s.head.number ? b : s.head,
            blocks: mergeBlocks(s.blocks, [b], BLOCK_WINDOW),
          }))
          // Fetch the certificate (and anything missed) right away rather than waiting for the poll.
          void poll()
        }
      }
      ws.onclose = () => {
        ws = null
        if (!alive) return
        setComponent('ws', { health: 'down', latencyMs: null, detail: 'Disconnected, retrying' })
        const delay = Math.min(60_000, 5_000 * 2 ** wsRetry++)
        wsTimer = setTimeout(connectWs, delay)
      }
      ws.onerror = () => ws?.close()
    }

    void poll()
    probeAll()
    connectWs()
    timers.push(setInterval(() => visible() && void poll(), POLL_MS))
    timers.push(setInterval(() => visible() && probeAll(), PROBE_MS))
    // A connected socket that stops delivering heads is degraded.
    timers.push(
      setInterval(() => {
        if (ws && ws.readyState === WebSocket.OPEN && lastHeadAt && Date.now() - lastHeadAt > 30_000) {
          setComponent('ws', { health: 'degraded', latencyMs: null, detail: 'Connected, no new heads for 30 s' })
        }
      }, 10_000),
    )

    return () => {
      alive = false
      timers.forEach(clearInterval)
      if (wsTimer) clearTimeout(wsTimer)
      if (ws) {
        ws.onclose = null
        ws.close()
      }
    }
  }, [])

  return state
}

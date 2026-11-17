'use client'
// Live chain head for the home page: WebSocket `newHeads` when the node offers it, `eth_blockNumber` polling
// otherwise, fixture ticks in demo mode. Keeps a rolling window of recent blocks for the computed statistics.

import { useEffect, useRef, useState } from 'react'
import { config } from './config.ts'
import { computeStats, type ChainStats } from './chainstats.ts'
import * as demo from './fixtures.ts'
import { isUnreachable, setNet } from './net.ts'
import * as rpc from './rpc.ts'
import type { BlockSummary, TxSummary } from './types.ts'

export type LiveMode = 'connecting' | 'ws' | 'poll' | 'demo' | 'offline'

export interface LiveChain {
  mode: LiveMode
  blocks: BlockSummary[]
  /** Transactions from the window's blocks, newest first. Status is unknown (no receipts). */
  txs: TxSummary[]
  stats: ChainStats
  /** Wall-clock ms when the newest head arrived (drives the ticker animation). */
  headAt: number
}

const POLL_MS = 2000
const MAX_CATCHUP = 12
const BACKOFF = [10_000, 20_000, 40_000, 80_000, 120_000]

export function mergeBlocks(prev: BlockSummary[], incoming: BlockSummary[], keep: number): BlockSummary[] {
  const map = new Map(prev.map((b) => [b.number, b]))
  for (const b of incoming) map.set(b.number, b)
  return [...map.values()].sort((a, b) => b.number - a.number).slice(0, keep)
}

export function mergeTxs(prev: TxSummary[], incoming: TxSummary[], keep: number): TxSummary[] {
  const seen = new Set<string>()
  return [...incoming, ...prev]
    .sort((a, b) => b.blockNumber - a.blockNumber || b.txIndex - a.txIndex)
    .filter((t) => (seen.has(t.hash) ? false : (seen.add(t.hash), true)))
    .slice(0, keep)
}

export function useLiveChain(windowSize = 30): LiveChain {
  const [state, setState] = useState<LiveChain>({ mode: config.demo ? 'demo' : 'connecting', blocks: [], txs: [], stats: computeStats([]), headAt: 0 })
  const headRef = useRef(-1)

  useEffect(() => {
    let disposed = false
    const timers = new Set<ReturnType<typeof setTimeout>>()
    const later = (fn: () => void, ms: number) => {
      const t = setTimeout(() => (timers.delete(t), !disposed && fn()), ms)
      timers.add(t)
    }
    let ws: WebSocket | null = null
    let pollTimer: ReturnType<typeof setInterval> | null = null
    let attempt = 0
    let wsFailures = 0

    const apply = (blocks: BlockSummary[], txs: TxSummary[], mode?: LiveMode) => {
      if (disposed) return
      setState((s) => {
        const merged = mergeBlocks(s.blocks, blocks, windowSize)
        return {
          mode: mode ?? s.mode,
          blocks: merged,
          txs: mergeTxs(s.txs, txs, 25),
          stats: computeStats(merged),
          headAt: blocks.length ? Date.now() : s.headAt,
        }
      })
    }

    if (config.demo) {
      const tick = () => {
        const head = demo.demoHead()
        if (head === headRef.current) return
        const from = headRef.current < 0 ? head - windowSize + 1 : Math.max(headRef.current + 1, head - MAX_CATCHUP)
        const blocks: BlockSummary[] = []
        const txs: TxSummary[] = []
        for (let n = from; n <= head; n++) {
          const b = demo.demoBlock(n)
          const { transactions, commit: _c, ...summary } = b
          blocks.push(summary)
          txs.push(...transactions)
        }
        headRef.current = head
        setNet({ head })
        apply(blocks, txs, 'demo')
      }
      tick()
      const id = setInterval(tick, 1000)
      return () => {
        disposed = true
        clearInterval(id)
      }
    }

    const fail = (e: unknown) => {
      if (disposed) return
      if (isUnreachable(e)) setNet({ rpc: 'down' })
      stopStreams()
      setState((s) => ({ ...s, mode: 'offline' }))
      later(start, BACKOFF[Math.min(attempt++, BACKOFF.length - 1)]!)
    }

    const catchUp = async (target: number) => {
      if (target <= headRef.current) return
      const from = Math.max(headRef.current + 1, target - MAX_CATCHUP + 1)
      headRef.current = target
      setNet({ head: target })
      const got = await Promise.all(Array.from({ length: target - from + 1 }, (_, i) => rpc.rpcBlockSummary(from + i)))
      apply(got.map((g) => g.block), got.flatMap((g) => g.txs))
    }

    const stopStreams = () => {
      if (pollTimer) clearInterval(pollTimer)
      pollTimer = null
      if (ws) {
        ws.onclose = ws.onerror = ws.onmessage = ws.onopen = null
        try {
          ws.close()
        } catch {
          /* ignore */
        }
      }
      ws = null
    }

    const startPolling = () => {
      if (pollTimer || disposed) return
      setState((s) => ({ ...s, mode: 'poll' }))
      pollTimer = setInterval(() => {
        if (document.visibilityState !== 'visible') return
        rpc.rpcHead().then(catchUp).catch(fail)
      }, POLL_MS)
    }

    const startWs = () => {
      if (!config.wsUrl || typeof WebSocket === 'undefined' || wsFailures >= 3) return startPolling()
      let sock: WebSocket
      try {
        sock = new WebSocket(config.wsUrl)
      } catch {
        wsFailures++
        return startPolling()
      }
      ws = sock
      let opened = false
      sock.onopen = () => {
        opened = true
        sock.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_subscribe', params: ['newHeads'] }))
      }
      sock.onmessage = (ev) => {
        try {
          const msg = JSON.parse(String(ev.data)) as { id?: number; error?: unknown; params?: { result?: { number?: string } } }
          if (msg.id === 1 && msg.error) throw new Error('subscribe rejected')
          if (msg.id === 1) {
            // Subscribed: the socket drives the page, polling stops.
            if (pollTimer) clearInterval(pollTimer)
            pollTimer = null
            setState((s) => ({ ...s, mode: 'ws' }))
          }
          const n = msg.params?.result?.number
          if (n) catchUp(Number(BigInt(n))).catch(fail)
        } catch {
          sock.close()
        }
      }
      const lost = () => {
        // onerror and onclose both fire for one failure: handle it once.
        sock.onerror = sock.onclose = sock.onmessage = sock.onopen = null
        if (ws === sock) ws = null
        wsFailures++
        if (disposed) return
        // Polling keeps the page live; retry the socket later (a socket that never opened gets 3 tries in total).
        startPolling()
        if (opened || wsFailures < 3)
          later(() => {
            if (pollTimer) clearInterval(pollTimer)
            pollTimer = null
            startWs()
          }, 30_000 * wsFailures)
      }
      sock.onerror = lost
      sock.onclose = lost
      // Poll until the subscription is confirmed.
      startPolling()
    }

    const start = async () => {
      try {
        const head = await rpc.rpcHead()
        setNet({ rpc: 'up' })
        attempt = 0
        const { blocks, txs } = await rpc.rpcRecentBlocks(head, windowSize)
        headRef.current = head
        setNet({ head })
        apply(blocks, txs)
        startWs()
      } catch (e) {
        fail(e)
      }
    }
    start()

    return () => {
      disposed = true
      stopStreams()
      timers.forEach(clearTimeout)
    }
  }, [windowSize])

  return state
}

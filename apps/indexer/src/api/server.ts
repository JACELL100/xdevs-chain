// node:http server for the read-only explorer API: routing, CORS, per-IP token-bucket rate limit, JSON errors.

import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'
import type { Config } from '../config.ts'
import type { Queryable } from '../db.ts'
import type { Rpc } from '../rpc.ts'
import * as h from './handlers.ts'

type Handler = (ctx: h.ApiContext, params: string[], qs: URLSearchParams) => Promise<unknown>

const routes: [RegExp, Handler, number][] = [
  [/^\/health$/, (c) => h.health(c), 0],
  [/^\/api\/v1\/stats$/, (c) => h.stats(c), 5],
  [/^\/api\/v1\/blocks$/, (c, _p, qs) => h.listBlocks(c, qs), 5],
  [/^\/api\/v1\/blocks\/([^/]+)$/, (c, p) => h.getBlock(c, p[0]!), 60],
  [/^\/api\/v1\/txs$/, (c, _p, qs) => h.listTxs(c, qs), 5],
  [/^\/api\/v1\/txs\/([^/]+)$/, (c, p) => h.getTx(c, p[0]!), 60],
  [/^\/api\/v1\/addresses\/([^/]+)$/, (c, p) => h.getAddressSummary(c, p[0]!), 5],
  [/^\/api\/v1\/addresses\/([^/]+)\/txs$/, (c, p, qs) => h.addressTxs(c, p[0]!, qs), 5],
  [/^\/api\/v1\/addresses\/([^/]+)\/token-transfers$/, (c, p, qs) => h.addressTransfers(c, p[0]!, qs), 5],
  [/^\/api\/v1\/addresses\/([^/]+)\/approvals$/, (c, p, qs) => h.addressApprovals(c, p[0]!, qs), 5],
  [/^\/api\/v1\/addresses\/([^/]+)\/vault-queue$/, (c, p, qs) => h.vaultQueue(c, p[0]!, qs), 5],
  [/^\/api\/v1\/names\/([^/]+)$/, (c, p) => h.getName(c, p[0]!), 5],
  [/^\/api\/v1\/search$/, (c, _p, qs) => h.search(c, qs), 5],
]

/** Token bucket per key. */
export class RateLimiter {
  private readonly buckets = new Map<string, { tokens: number; at: number }>()
  readonly rps: number
  readonly burst: number
  private readonly clock: () => number
  constructor(rps: number, burst: number, clock: () => number = () => Date.now()) {
    this.rps = rps
    this.burst = burst
    this.clock = clock
  }
  take(key: string): { ok: boolean; remaining: number; retryAfter: number } {
    const now = this.clock()
    const b = this.buckets.get(key) ?? { tokens: this.burst, at: now }
    b.tokens = Math.min(this.burst, b.tokens + ((now - b.at) / 1000) * this.rps)
    b.at = now
    this.buckets.set(key, b)
    if (this.buckets.size > 50_000) this.sweep(now)
    if (b.tokens >= 1) {
      b.tokens -= 1
      return { ok: true, remaining: Math.floor(b.tokens), retryAfter: 0 }
    }
    return { ok: false, remaining: 0, retryAfter: Math.ceil((1 - b.tokens) / this.rps) }
  }
  private sweep(now: number) {
    for (const [k, b] of this.buckets) if (now - b.at > (this.burst / this.rps) * 1000) this.buckets.delete(k)
  }
}

export function clientIp(req: IncomingMessage, trust: Config['trustProxy']): string {
  if (trust === 'cloudflare') {
    const v = req.headers['cf-connecting-ip']
    if (typeof v === 'string' && v) return v
  }
  if (trust === 'xff') {
    const v = req.headers['x-forwarded-for']
    if (typeof v === 'string' && v) return v.split(',')[0]!.trim()
  }
  return req.socket.remoteAddress ?? 'unknown'
}

export interface ApiOptions {
  db: Queryable
  rpc: Rpc
  cfg: Config
  now?: () => number
  log?: (m: string) => void
}

export function createApiServer(opts: ApiOptions): Server {
  const ctx: h.ApiContext = { db: opts.db, rpc: opts.rpc, cfg: opts.cfg, now: opts.now ?? (() => Math.floor(Date.now() / 1000)) }
  const limiter = new RateLimiter(opts.cfg.rateLimitRps, opts.cfg.rateLimitBurst)
  const log = opts.log ?? ((m: string) => console.log(`[api] ${m}`))
  const origins = opts.cfg.corsOrigins

  return createServer((req, res) => {
    void handle(req, res).catch((err) => {
      log(`unhandled: ${(err as Error).message}`)
      if (!res.headersSent) send(res, 500, { error: { code: 'INTERNAL', message: 'internal error' } }, req.method === 'HEAD')
    })
  })

  async function handle(req: IncomingMessage, res: ServerResponse) {
    const origin = req.headers.origin
    if (origins.includes('*')) res.setHeader('access-control-allow-origin', '*')
    else if (origin && origins.includes(origin)) {
      res.setHeader('access-control-allow-origin', origin)
      res.setHeader('vary', 'Origin')
    }
    res.setHeader('access-control-allow-methods', 'GET, HEAD, OPTIONS')
    res.setHeader('access-control-allow-headers', 'content-type')
    res.setHeader('access-control-max-age', '86400')
    res.setHeader('x-content-type-options', 'nosniff')

    if (req.method === 'OPTIONS') {
      res.writeHead(204)
      return res.end()
    }
    const head = req.method === 'HEAD'
    if (req.method !== 'GET' && !head) {
      res.setHeader('allow', 'GET, HEAD, OPTIONS')
      return send(res, 405, { error: { code: 'METHOD_NOT_ALLOWED', message: 'read-only API: GET only' } }, head)
    }

    const ip = clientIp(req, opts.cfg.trustProxy)
    const rl = limiter.take(ip)
    res.setHeader('ratelimit-limit', String(opts.cfg.rateLimitRps))
    res.setHeader('ratelimit-remaining', String(rl.remaining))
    if (!rl.ok) {
      res.setHeader('retry-after', String(rl.retryAfter))
      return send(res, 429, { error: { code: 'RATE_LIMITED', message: 'too many requests' } }, head)
    }

    const url = new URL(req.url ?? '/', 'http://localhost')
    const path = url.pathname.replace(/\/+$/, '') || '/'
    for (const [re, fn, maxAge] of routes) {
      const m = re.exec(path)
      if (!m) continue
      try {
        const body = await fn(ctx, m.slice(1).map((s) => decodeURIComponent(s)), url.searchParams)
        res.setHeader('cache-control', maxAge ? `public, max-age=${maxAge}` : 'no-store')
        return send(res, 200, body, head)
      } catch (err) {
        if (err instanceof h.HttpError) return send(res, err.status, { error: { code: err.code, message: err.message } }, head)
        const msg = (err as Error).message ?? String(err)
        const dbDown = /ECONNREFUSED|ETIMEDOUT|terminat|Connection|connect/i.test(msg)
        log(`${path}: ${msg}`)
        return send(res, dbDown ? 503 : 500, { error: { code: dbDown ? 'UNAVAILABLE' : 'INTERNAL', message: dbDown ? 'database unavailable' : 'internal error' } }, head)
      }
    }
    return send(res, 404, { error: { code: 'NOT_FOUND', message: 'no such endpoint' } }, head)
  }
}

function send(res: ServerResponse, status: number, body: unknown, head: boolean) {
  const text = JSON.stringify(body)
  res.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'content-length': Buffer.byteLength(text) })
  res.end(head ? undefined : text)
}

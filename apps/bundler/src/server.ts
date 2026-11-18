// JSON-RPC 2.0 HTTP server: per-IP rate limits (Cloudflare-aware), body / batch limits, CORS allow-list,
// `/health` and Prometheus `/metrics`. Internal errors are logged, never echoed (they can contain RPC URLs).
import { randomUUID } from 'node:crypto'
import { type IncomingMessage, type Server, type ServerResponse, createServer } from 'node:http'
import type { Bundler } from './bundler.ts'
import { type BundlerConfig, DEFAULT_CONFIG } from './config.ts'
import { RpcCodes, RpcError } from './errors.ts'
import { type Logger, silentLogger } from './log.ts'
import { TokenBuckets, clientIp } from './ratelimit.ts'

type Req = { jsonrpc?: string; id?: unknown; method?: unknown; params?: unknown }

export type ServerOptions = Partial<
  Pick<
    BundlerConfig,
    | 'trustProxy'
    | 'trustedProxies'
    | 'corsOrigins'
    | 'maxBodyBytes'
    | 'maxBatch'
    | 'rateLimitRps'
    | 'rateLimitBurst'
    | 'sendPerMinute'
    | 'estimatePerMinute'
  >
> & { logger?: Logger }

/** Bundler surface used by the server (lets tests use a stub). */
export type BundlerLike = Pick<Bundler, 'handle' | 'entryPoint' | 'chainId' | 'health' | 'metricsText' | 'metrics'>

export function createBundlerServer(bundler: BundlerLike, o: ServerOptions = {}): Server {
  const cfg = { ...DEFAULT_CONFIG, ...('cfg' in bundler ? (bundler as Bundler).cfg : {}), ...stripUndefined(o) }
  const log = o.logger ?? silentLogger
  const general = new TokenBuckets(cfg.rateLimitRps, cfg.rateLimitBurst)
  const sends = new TokenBuckets(cfg.sendPerMinute / 60, Math.max(1, Math.ceil(cfg.sendPerMinute / 4)))
  const estimates = new TokenBuckets(cfg.estimatePerMinute / 60, Math.max(1, Math.ceil(cfg.estimatePerMinute / 4)))
  const anyOrigin = cfg.corsOrigins.includes('*')
  const m = bundler.metrics
  m.declare('bundler_rate_limited_total', 'Requests refused by the per-IP rate limits', 'counter', { bucket: 'general' })

  const limited = (bucket: string) => m.inc('bundler_rate_limited_total', 'Requests refused by the per-IP rate limits', { bucket })

  async function handleOne(req: Req, ip: string): Promise<unknown> {
    const id = req && typeof req === 'object' && 'id' in req ? (req.id ?? null) : null
    try {
      if (!req || typeof req !== 'object' || typeof req.method !== 'string') {
        throw new RpcError(RpcCodes.invalidRequest, 'invalid request')
      }
      const params = req.params === undefined ? [] : req.params
      if (!Array.isArray(params)) throw new RpcError(RpcCodes.invalidParams, 'params must be an array')
      if (req.method === 'eth_sendUserOperation' && !sends.take(ip)) {
        limited('send')
        throw new RpcError(RpcCodes.limitExceeded, 'rate limit: too many eth_sendUserOperation calls, retry later')
      }
      if (req.method === 'eth_estimateUserOperationGas' && !estimates.take(ip)) {
        limited('estimate')
        throw new RpcError(RpcCodes.limitExceeded, 'rate limit: too many eth_estimateUserOperationGas calls, retry later')
      }
      m.inc('bundler_rpc_requests_total', 'JSON-RPC requests by method', {
        method: KNOWN_METHODS.has(req.method) ? req.method : 'other',
      })
      const result = await bundler.handle(req.method, params, { ip })
      return { jsonrpc: '2.0', id, result }
    } catch (e) {
      let err: RpcError
      if (e instanceof RpcError) err = e
      else {
        const ref = randomUUID().slice(0, 8)
        log.error('internal error', { ref, method: typeof req?.method === 'string' ? req.method : undefined, error: (e as Error)?.message })
        err = new RpcError(RpcCodes.internal, `internal error (ref ${ref})`)
      }
      const error: Record<string, unknown> = { code: err.code, message: err.message }
      if (err.data !== undefined) error.data = err.data
      return { jsonrpc: '2.0', id, error }
    }
  }

  function cors(req: IncomingMessage, res: ServerResponse) {
    const origin = req.headers.origin
    if (anyOrigin) res.setHeader('access-control-allow-origin', '*')
    else if (typeof origin === 'string' && cfg.corsOrigins.includes(origin)) {
      res.setHeader('access-control-allow-origin', origin)
      res.setHeader('vary', 'origin')
    }
    res.setHeader('access-control-allow-methods', 'POST, GET, OPTIONS')
    res.setHeader('access-control-allow-headers', 'content-type')
    res.setHeader('access-control-max-age', '600')
  }

  const json = (res: ServerResponse, status: number, obj: unknown, extra: Record<string, string> = {}) => {
    res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store', 'x-content-type-options': 'nosniff', ...extra })
    res.end(JSON.stringify(obj))
  }

  const server = createServer((req, res) => {
    cors(req, res)
    const ip = clientIp(req, cfg.trustProxy, cfg.trustedProxies)
    const path = (req.url ?? '/').split('?')[0]
    if (req.method === 'OPTIONS') {
      res.writeHead(204).end()
      return
    }
    if (!general.take(ip)) {
      limited('general')
      json(res, 429, { jsonrpc: '2.0', id: null, error: { code: RpcCodes.limitExceeded, message: 'rate limit exceeded' } }, {
        'retry-after': String(general.retryAfter(ip)),
      })
      req.resume()
      return
    }
    if (req.method === 'GET') {
      if (path === '/health' || path === '/healthz') {
        const h = bundler.health()
        return json(res, h.ok ? 200 : 503, h.body)
      }
      if (path === '/metrics') {
        res.writeHead(200, { 'content-type': 'text/plain; version=0.0.4', 'cache-control': 'no-store' })
        res.end(bundler.metricsText())
        return
      }
      if (path === '/') {
        return json(res, 200, {
          service: 'xdevs-bundler',
          entryPoint: bundler.entryPoint,
          chainId: bundler.chainId,
          methods: [...KNOWN_METHODS],
          docs: 'https://github.com/JACELL100/xdevs-chain/tree/main/apps/bundler',
        })
      }
      return json(res, 404, { error: 'not found' })
    }
    if (req.method !== 'POST') {
      res.writeHead(405, { allow: 'GET, POST, OPTIONS' }).end()
      return
    }
    const declared = Number(req.headers['content-length'] ?? 0)
    if (declared > cfg.maxBodyBytes) {
      json(res, 413, { jsonrpc: '2.0', id: null, error: { code: RpcCodes.invalidRequest, message: 'request too large' } }, { connection: 'close' })
      req.resume()
      return
    }
    const chunks: Buffer[] = []
    let size = 0
    let aborted = false
    req.on('data', (chunk: Buffer) => {
      if (aborted) return
      size += chunk.length
      if (size > cfg.maxBodyBytes) {
        aborted = true
        json(res, 413, { jsonrpc: '2.0', id: null, error: { code: RpcCodes.invalidRequest, message: 'request too large' } }, { connection: 'close' })
        req.resume()
        return
      }
      chunks.push(chunk)
    })
    req.on('error', () => {
      aborted = true
    })
    req.on('end', async () => {
      if (aborted) return
      let parsed: unknown
      try {
        parsed = JSON.parse(Buffer.concat(chunks).toString('utf8'))
      } catch {
        return json(res, 200, { jsonrpc: '2.0', id: null, error: { code: RpcCodes.parseError, message: 'parse error' } })
      }
      try {
        if (Array.isArray(parsed)) {
          if (parsed.length === 0 || parsed.length > cfg.maxBatch) {
            return json(res, 200, {
              jsonrpc: '2.0',
              id: null,
              error: { code: RpcCodes.invalidRequest, message: `batch must contain 1 to ${cfg.maxBatch} requests` },
            })
          }
          // Each batch entry beyond the first costs one more general token.
          if (parsed.length > 1 && !general.take(ip, parsed.length - 1)) {
            limited('general')
            return json(res, 429, { jsonrpc: '2.0', id: null, error: { code: RpcCodes.limitExceeded, message: 'rate limit exceeded' } })
          }
          const out: unknown[] = []
          for (const r of parsed) out.push(await handleOne(r as Req, ip))
          return json(res, 200, out)
        }
        return json(res, 200, await handleOne(parsed as Req, ip))
      } catch (e) {
        log.error('request failed', { error: (e as Error).message })
        if (!res.headersSent) json(res, 500, { jsonrpc: '2.0', id: null, error: { code: RpcCodes.internal, message: 'internal error' } })
      }
    })
  })
  server.requestTimeout = 30_000
  server.headersTimeout = 10_000
  server.keepAliveTimeout = 5_000
  server.maxHeadersCount = 64
  return server
}

const KNOWN_METHODS = new Set([
  'eth_chainId',
  'eth_supportedEntryPoints',
  'eth_sendUserOperation',
  'eth_estimateUserOperationGas',
  'eth_getUserOperationReceipt',
  'eth_getUserOperationByHash',
])

function stripUndefined<T extends object>(o: T): Partial<T> {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as Partial<T>
}

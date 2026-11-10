// Minimal JSON-RPC 2.0 HTTP server for the dev bundler (node:http, CORS open for local wallets).
import { type Server, createServer } from 'node:http'
import type { Bundler } from './bundler.ts'
import { RpcCodes, RpcError } from './errors.ts'

type Req = { jsonrpc?: string; id?: unknown; method?: unknown; params?: unknown }

const MAX_BODY = 1_000_000
const MAX_BATCH = 20

async function handleOne(bundler: Bundler, req: Req): Promise<unknown> {
  const id = req?.id ?? null
  try {
    if (!req || typeof req !== 'object' || typeof req.method !== 'string') {
      throw new RpcError(RpcCodes.invalidRequest, 'invalid request')
    }
    const params = req.params === undefined ? [] : req.params
    if (!Array.isArray(params)) throw new RpcError(RpcCodes.invalidParams, 'params must be an array')
    const result = await bundler.handle(req.method, params)
    return { jsonrpc: '2.0', id, result }
  } catch (e) {
    const err = e instanceof RpcError ? e : new RpcError(RpcCodes.internal, (e as Error)?.message ?? String(e))
    const error: Record<string, unknown> = { code: err.code, message: err.message }
    if (err.data !== undefined) error.data = err.data
    return { jsonrpc: '2.0', id, error }
  }
}

export function createBundlerServer(bundler: Bundler): Server {
  return createServer((req, res) => {
    res.setHeader('access-control-allow-origin', '*')
    res.setHeader('access-control-allow-methods', 'POST, GET, OPTIONS')
    res.setHeader('access-control-allow-headers', 'content-type')
    if (req.method === 'OPTIONS') {
      res.writeHead(204).end()
      return
    }
    if (req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ service: 'xdevs-dev-bundler', devOnly: true, entryPoint: bundler.entryPoint, chainId: bundler.chainId }))
      return
    }
    if (req.method !== 'POST') {
      res.writeHead(405).end()
      return
    }
    let body = ''
    let tooBig = false
    req.setEncoding('utf8')
    req.on('data', (chunk: string) => {
      body += chunk
      if (body.length > MAX_BODY) tooBig = true
    })
    req.on('end', async () => {
      const send = (obj: unknown) => {
        res.writeHead(200, { 'content-type': 'application/json' })
        res.end(JSON.stringify(obj))
      }
      if (tooBig) return send({ jsonrpc: '2.0', id: null, error: { code: RpcCodes.invalidRequest, message: 'request too large' } })
      let parsed: unknown
      try {
        parsed = JSON.parse(body)
      } catch {
        return send({ jsonrpc: '2.0', id: null, error: { code: RpcCodes.parseError, message: 'parse error' } })
      }
      if (Array.isArray(parsed)) {
        if (parsed.length === 0 || parsed.length > MAX_BATCH) {
          return send({ jsonrpc: '2.0', id: null, error: { code: RpcCodes.invalidRequest, message: 'bad batch size' } })
        }
        const out: unknown[] = []
        for (const r of parsed) out.push(await handleOne(bundler, r as Req))
        return send(out)
      }
      return send(await handleOne(bundler, parsed as Req))
    })
  })
}

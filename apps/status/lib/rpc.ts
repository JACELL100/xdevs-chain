// Minimal browser JSON-RPC client (fetch + batch) with latency measurement. No viem needed for
// the handful of read calls the status page makes, which keeps the bundle small.

export class RpcError extends Error {
  readonly code: number
  constructor(code: number, message: string) {
    super(message)
    this.code = code
  }
}

export interface RpcCall {
  method: string
  params?: unknown[]
}

export type RpcResult<T = unknown> = { ok: true; value: T } | { ok: false; error: RpcError }

export interface BatchResponse {
  results: RpcResult[]
  /** Round trip of the whole HTTP request in ms. */
  latencyMs: number
}

let nextId = 1

/** Map a batch reply (any order, by id) back onto the calls; missing entries become errors. */
export function matchBatch(ids: number[], body: unknown): RpcResult[] {
  const byId = new Map<number, Record<string, unknown>>()
  const list = Array.isArray(body) ? body : [body]
  for (const item of list) {
    if (item && typeof item === 'object' && typeof (item as { id?: unknown }).id === 'number') {
      byId.set((item as { id: number }).id, item as Record<string, unknown>)
    }
  }
  return ids.map((id) => {
    const r = byId.get(id)
    if (!r) return { ok: false, error: new RpcError(-32603, 'missing response') }
    if (r.error && typeof r.error === 'object') {
      const e = r.error as { code?: unknown; message?: unknown }
      return { ok: false, error: new RpcError(typeof e.code === 'number' ? e.code : -32603, String(e.message ?? 'error')) }
    }
    return { ok: true, value: r.result }
  })
}

export const METHOD_NOT_FOUND = -32601

export async function rpcBatch(url: string, calls: RpcCall[], timeoutMs = 8000): Promise<BatchResponse> {
  const ids = calls.map(() => nextId++)
  const payload = calls.map((c, i) => ({ jsonrpc: '2.0', id: ids[i], method: c.method, params: c.params ?? [] }))
  const ctrl = new AbortController()
  const timer = setTimeout(() => ctrl.abort(), timeoutMs)
  const t0 = performance.now()
  try {
    const res = await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(payload),
      signal: ctrl.signal,
      cache: 'no-store',
    })
    if (!res.ok) throw new RpcError(res.status, `HTTP ${res.status}`)
    const body: unknown = await res.json()
    return { results: matchBatch(ids, body), latencyMs: performance.now() - t0 }
  } finally {
    clearTimeout(timer)
  }
}

/** Split into chunks of at most `size` (the public RPC caps batches at 50). */
export function chunk<T>(items: T[], size: number): T[][] {
  const out: T[][] = []
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size))
  return out
}

export const toHex = (n: number) => `0x${n.toString(16)}`

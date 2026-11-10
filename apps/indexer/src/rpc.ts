// Minimal JSON-RPC client (fetch) with batching, timeouts and retries, plus a newHeads WebSocket follower.
// The node limits batches to 50 requests (plan §19.3), so batches are chunked.

export class RpcError extends Error {
  readonly code: number
  readonly data: unknown
  constructor(code: number, message: string, data?: unknown) {
    super(message)
    this.name = 'RpcError'
    this.code = code
    this.data = data
  }
}

export interface RpcCall {
  method: string
  params: unknown[]
}

export interface Rpc {
  request<T = unknown>(method: string, params?: unknown[]): Promise<T>
  /** Results in call order; a failed call yields an `RpcError` instance in its slot (never throws for those). */
  batch(calls: RpcCall[]): Promise<unknown[]>
}

export const MAX_BATCH = 50

export function isMethodNotFound(err: unknown): boolean {
  return (
    err instanceof RpcError &&
    (err.code === -32601 || err.code === -32004 || /method .*not (found|supported)|not supported/i.test(err.message))
  )
}

export interface HttpRpcOptions {
  timeoutMs?: number
  retries?: number
  useBatch?: boolean
  fetch?: typeof fetch
}

export class HttpRpc implements Rpc {
  private id = 0
  private readonly url: string
  private readonly timeoutMs: number
  private readonly retries: number
  private useBatch: boolean
  private readonly fetchFn: typeof fetch

  constructor(url: string, opts: HttpRpcOptions = {}) {
    this.url = url
    this.timeoutMs = opts.timeoutMs ?? 15_000
    this.retries = opts.retries ?? 3
    this.useBatch = opts.useBatch ?? true
    this.fetchFn = opts.fetch ?? fetch
  }

  private async post(body: unknown): Promise<unknown> {
    let lastErr: unknown
    for (let attempt = 0; attempt <= this.retries; attempt++) {
      try {
        const res = await this.fetchFn(this.url, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(body),
          signal: AbortSignal.timeout(this.timeoutMs),
        })
        if (res.status === 429 || res.status >= 500) throw new Error(`RPC HTTP ${res.status}`)
        const text = await res.text()
        try {
          return JSON.parse(text)
        } catch {
          throw new Error(`RPC HTTP ${res.status}: non-JSON response`)
        }
      } catch (err) {
        lastErr = err
        if (attempt < this.retries) await sleep(250 * 2 ** attempt)
      }
    }
    throw lastErr
  }

  async request<T>(method: string, params: unknown[] = []): Promise<T> {
    const res = (await this.post({ jsonrpc: '2.0', id: ++this.id, method, params })) as {
      result?: T
      error?: { code: number; message: string; data?: unknown }
    }
    if (res.error) throw new RpcError(res.error.code, res.error.message, res.error.data)
    return res.result as T
  }

  async batch(calls: RpcCall[]): Promise<unknown[]> {
    if (!calls.length) return []
    if (!this.useBatch) return Promise.all(calls.map((c) => this.request(c.method, c.params).catch(toRpcError)))
    const out: unknown[] = []
    for (let i = 0; i < calls.length; i += MAX_BATCH) {
      const chunk = calls.slice(i, i + MAX_BATCH)
      const first = this.id + 1
      const body = chunk.map((c) => ({ jsonrpc: '2.0', id: ++this.id, method: c.method, params: c.params }))
      const res = await this.post(body)
      if (!Array.isArray(res)) {
        // Server without batch support: fall back to single requests from now on.
        this.useBatch = false
        out.push(...(await Promise.all(chunk.map((c) => this.request(c.method, c.params).catch(toRpcError)))))
        continue
      }
      const byId = new Map<number, { result?: unknown; error?: { code: number; message: string; data?: unknown } }>()
      for (const r of res as { id: number; result?: unknown; error?: { code: number; message: string; data?: unknown } }[]) byId.set(r.id, r)
      chunk.forEach((_c, j) => {
        const r = byId.get(first + j)
        if (!r) out.push(new RpcError(-32603, 'missing response in batch'))
        else if (r.error) out.push(new RpcError(r.error.code, r.error.message, r.error.data))
        else out.push(r.result)
      })
    }
    return out
  }
}

function toRpcError(err: unknown): RpcError {
  return err instanceof RpcError ? err : new RpcError(-32603, err instanceof Error ? err.message : String(err))
}

/** Unwraps batch results, throwing the first error. */
export function unwrapAll<T>(results: unknown[]): T[] {
  for (const r of results) if (r instanceof RpcError) throw r
  return results as T[]
}

export const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

/**
 * Follows `eth_subscribe("newHeads")` over WebSocket and calls `onHead` for each new block number. Reconnects with
 * backoff. Purely a latency optimisation: the indexer keeps polling, so a broken socket only costs latency.
 */
export function followNewHeads(
  wsUrl: string,
  onHead: (n: number) => void,
  log: (msg: string) => void = () => {},
): { close(): void } {
  let closed = false
  let ws: WebSocket | null = null
  let delay = 1000
  const connect = () => {
    if (closed) return
    try {
      ws = new WebSocket(wsUrl)
    } catch (err) {
      log(`newHeads: cannot open WebSocket: ${(err as Error).message}`)
      return retry()
    }
    ws.onopen = () => {
      delay = 1000
      ws!.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_subscribe', params: ['newHeads'] }))
    }
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(String(ev.data)) as {
          id?: number
          error?: { message: string }
          params?: { result?: { number?: string } }
        }
        if (msg.id === 1 && msg.error) {
          log(`newHeads: subscription refused (${msg.error.message}); polling only`)
          closed = true
          ws?.close()
          return
        }
        const n = msg.params?.result?.number
        if (n) onHead(Number(BigInt(n)))
      } catch {
        /* ignore malformed frames */
      }
    }
    ws.onerror = () => {}
    ws.onclose = () => retry()
  }
  const retry = () => {
    if (closed) return
    setTimeout(connect, delay).unref?.()
    delay = Math.min(delay * 2, 30_000)
  }
  connect()
  return {
    close() {
      closed = true
      ws?.close()
    },
  }
}

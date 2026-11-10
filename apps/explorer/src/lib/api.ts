// Typed client for the explorer API and a tiny JSON-RPC helper for direct node reads.

import { config } from './config.ts'
import type {
  AddressSummary,
  Approval,
  BlockDetail,
  BlockSummary,
  NameInfo,
  Page,
  SearchResult,
  Stats,
  TokenTransfer,
  TxDetail,
  TxSummary,
  VaultAction,
} from './types.ts'

export class ApiError extends Error {
  readonly status: number
  readonly code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}

export async function apiGet<T>(path: string, signal?: AbortSignal, base = config.apiUrl): Promise<T> {
  let res: Response
  try {
    res = await fetch(`${base}/api/v1${path}`, { signal, headers: { accept: 'application/json' } })
  } catch (err) {
    if ((err as Error).name === 'AbortError') throw err
    throw new ApiError(0, 'NETWORK', 'The explorer API is unreachable. Check your connection or try again later.')
  }
  let body: unknown = null
  try {
    body = await res.json()
  } catch {
    /* non-JSON */
  }
  if (!res.ok) {
    const e = (body as { error?: { code?: string; message?: string } } | null)?.error
    throw new ApiError(res.status, e?.code ?? 'HTTP', e?.message ?? `HTTP ${res.status}`)
  }
  return body as T
}

const qs = (params: Record<string, string | number | boolean | null | undefined>) => {
  const p = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== null && v !== '') p.set(k, String(v))
  const s = p.toString()
  return s ? `?${s}` : ''
}
const enc = encodeURIComponent

export const api = {
  stats: (s?: AbortSignal) => apiGet<Stats>('/stats', s),
  blocks: (cursor?: string | null, limit = 25, s?: AbortSignal) => apiGet<Page<BlockSummary>>(`/blocks${qs({ cursor, limit })}`, s),
  block: (id: string, s?: AbortSignal) => apiGet<BlockDetail>(`/blocks/${enc(id)}`, s),
  txs: (cursor?: string | null, limit = 25, s?: AbortSignal) => apiGet<Page<TxSummary>>(`/txs${qs({ cursor, limit })}`, s),
  tx: (hash: string, s?: AbortSignal) => apiGet<TxDetail>(`/txs/${enc(hash)}`, s),
  address: (a: string, s?: AbortSignal) => apiGet<AddressSummary>(`/addresses/${enc(a)}`, s),
  addressTxs: (a: string, o: { cursor?: string | null; includeSpam?: boolean }, s?: AbortSignal) =>
    apiGet<Page<TxSummary>>(`/addresses/${enc(a)}/txs${qs({ cursor: o.cursor, includeSpam: o.includeSpam })}`, s),
  addressTransfers: (a: string, o: { cursor?: string | null; includeSpam?: boolean }, s?: AbortSignal) =>
    apiGet<Page<TokenTransfer>>(`/addresses/${enc(a)}/token-transfers${qs({ cursor: o.cursor, includeSpam: o.includeSpam })}`, s),
  approvals: (a: string, includeExpired = false, s?: AbortSignal) =>
    apiGet<{ items: Approval[]; live: boolean }>(`/addresses/${enc(a)}/approvals${qs({ includeExpired })}`, s),
  vaultQueue: (a: string, o: { cursor?: string | null; status?: 'open' | 'all' }, s?: AbortSignal) =>
    apiGet<Page<VaultAction>>(`/addresses/${enc(a)}/vault-queue${qs({ cursor: o.cursor, status: o.status })}`, s),
  name: (n: string, s?: AbortSignal) => apiGet<NameInfo>(`/names/${enc(n)}`, s),
  search: (q: string, s?: AbortSignal) => apiGet<{ query: string; results: SearchResult[] }>(`/search${qs({ q })}`, s),
}

/** Direct JSON-RPC read from the node (VITE_RPC_URL), used for the network check in the footer. */
export async function rpc<T>(method: string, params: unknown[] = [], signal?: AbortSignal): Promise<T> {
  const res = await fetch(config.rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
    signal,
  })
  const body = (await res.json()) as { result?: T; error?: { message: string } }
  if (body.error) throw new Error(body.error.message)
  return body.result as T
}

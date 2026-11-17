// Client for the faucet Worker (apps/faucet). Contract:
//   POST {api}/claim  {address, turnstileToken}
//     200 {ok:true, txHash, to, amount, amountWei, symbol, nonce, explorerUrl|null}
//     4xx/5xx {ok:false, error, message}  (+ Retry-After seconds on 429)
//   GET  {api}/status
//     200 {ok:true, address, chainId, symbol, dripAmount, minBalance, rateLimit, balance, balanceWei, healthy}
//     503 {ok:false, ...same base, balance:null, healthy:false, error:"rpc_unavailable"}
//     503 {ok:false, error:"misconfigured", message}

export interface ClaimSuccess {
  ok: true
  txHash: `0x${string}`
  to: string
  amount: string
  symbol: string
  explorerUrl: string | null
}

export type ClaimErrorKind =
  | 'rate_limited'
  | 'faucet_empty'
  | 'network_down'
  | 'unreachable'
  | 'captcha'
  | 'invalid_recipient'
  | 'names_not_supported'
  | 'unavailable'
  | 'unknown'

export interface ClaimFailure {
  ok: false
  kind: ClaimErrorKind
  code: string
  message: string
  /** Seconds until the next claim is allowed (rate_limited only). */
  retryAfter?: number
}

export type ClaimOutcome = ClaimSuccess | ClaimFailure

export interface FaucetStatus {
  reachable: boolean
  healthy: boolean
  address?: string
  chainId?: number
  symbol?: string
  dripAmount?: string
  balance?: string | null
  minBalance?: string
  error?: string
}

const KIND_BY_CODE: Record<string, ClaimErrorKind> = {
  rate_limited: 'rate_limited',
  faucet_empty: 'faucet_empty',
  rpc_unavailable: 'network_down',
  captcha_required: 'captcha',
  captcha_failed: 'captcha',
  captcha_unavailable: 'captcha',
  invalid_address: 'invalid_recipient',
  invalid_name: 'invalid_recipient',
  reserved_address: 'invalid_recipient',
  names_not_supported: 'names_not_supported',
  misconfigured: 'unavailable',
  internal: 'unavailable',
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

/** Parse a Retry-After header (delta-seconds or HTTP date). */
export function parseRetryAfter(header: string | null, now = Date.now()): number | undefined {
  if (!header) return undefined
  const trimmed = header.trim()
  if (/^\d+$/.test(trimmed)) return Number(trimmed)
  const at = Date.parse(trimmed)
  if (Number.isNaN(at)) return undefined
  return Math.max(0, Math.ceil((at - now) / 1000))
}

/** Turn a Worker response (status, headers, parsed JSON or null) into a ClaimOutcome. */
export function interpretClaim(status: number, body: unknown, retryAfterHeader: string | null): ClaimOutcome {
  if (isRecord(body) && body.ok === true && typeof body.txHash === 'string' && /^0x[0-9a-fA-F]{64}$/.test(body.txHash)) {
    return {
      ok: true,
      txHash: body.txHash as `0x${string}`,
      to: String(body.to ?? ''),
      amount: String(body.amount ?? ''),
      symbol: String(body.symbol ?? 'tXDS'),
      explorerUrl: typeof body.explorerUrl === 'string' && /^https?:\/\//.test(body.explorerUrl) ? body.explorerUrl : null,
    }
  }
  if (!isRecord(body)) {
    // Non-JSON: the Worker is not deployed/routed yet (Pages 404 page) or a proxy error.
    return {
      ok: false,
      kind: 'unreachable',
      code: `http_${status}`,
      message: 'The faucet service is not reachable right now.',
    }
  }
  const code = typeof body.error === 'string' ? body.error : `http_${status}`
  const message = typeof body.message === 'string' && body.message ? body.message : 'Something went wrong. Please try again.'
  const kind = KIND_BY_CODE[code] ?? (status === 429 ? 'rate_limited' : 'unknown')
  const out: ClaimFailure = { ok: false, kind, code, message }
  if (kind === 'rate_limited') out.retryAfter = parseRetryAfter(retryAfterHeader) ?? 24 * 3600
  return out
}

export function interpretStatus(status: number, body: unknown): FaucetStatus {
  if (!isRecord(body)) return { reachable: false, healthy: false }
  const s: FaucetStatus = {
    reachable: true,
    healthy: body.healthy === true && status === 200,
    error: typeof body.error === 'string' ? body.error : undefined,
  }
  if (typeof body.address === 'string') s.address = body.address
  if (typeof body.chainId === 'number') s.chainId = body.chainId
  if (typeof body.symbol === 'string') s.symbol = body.symbol
  if (typeof body.dripAmount === 'string') s.dripAmount = body.dripAmount
  if (typeof body.minBalance === 'string') s.minBalance = body.minBalance
  if (typeof body.balance === 'string' || body.balance === null) s.balance = body.balance as string | null
  return s
}

async function readJson(res: Response): Promise<unknown> {
  const type = res.headers.get('content-type') ?? ''
  if (!type.includes('json')) return null
  try {
    return await res.json()
  } catch {
    return null
  }
}

export async function claim(apiBase: string, address: string, turnstileToken: string, signal?: AbortSignal): Promise<ClaimOutcome> {
  let res: Response
  try {
    res = await fetch(`${apiBase}/claim`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ address, turnstileToken }),
      signal,
    })
  } catch {
    return { ok: false, kind: 'unreachable', code: 'network_error', message: 'Could not reach the faucet. Check your connection and try again.' }
  }
  return interpretClaim(res.status, await readJson(res), res.headers.get('Retry-After'))
}

export async function fetchStatus(apiBase: string, signal?: AbortSignal): Promise<FaucetStatus> {
  try {
    const res = await fetch(`${apiBase}/status`, { signal, cache: 'no-store' })
    return interpretStatus(res.status, await readJson(res))
  } catch {
    return { reachable: false, healthy: false }
  }
}

/** "23 h 59 min", "42 min", "under a minute" */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  if (s < 60) return 'under a minute'
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (h === 0) return `${m} min`
  return m === 0 ? `${h} h` : `${h} h ${m} min`
}

/** 1234567.5 -> "1,234,567.5" (keeps up to 4 decimals). */
export function formatAmount(value: string | number | null | undefined, maxDecimals = 2): string {
  if (value === null || value === undefined || value === '') return '—'
  const n = typeof value === 'number' ? value : Number(value)
  if (!Number.isFinite(n)) return String(value)
  return n.toLocaleString('en-US', { maximumFractionDigits: maxDecimals })
}

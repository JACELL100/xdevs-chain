// Pure formatting helpers (unit-tested with node --test).

export type Tone = 'ok' | 'warn' | 'danger' | 'info' | 'neutral'

export const toBig = (v: string | bigint | number | null | undefined): bigint => {
  if (v === null || v === undefined || v === '') return 0n
  return typeof v === 'bigint' ? v : BigInt(v)
}

const group = (int: string) => int.replace(/\B(?=(\d{3})+(?!\d))/g, ',')

/** `formatUnits(1234567890000000000000n, 18)` → `"1,234.56789"`. Trims trailing zeros, at most `maxFraction` digits (rounded down). */
export function formatUnits(value: string | bigint | number, decimals: number, maxFraction = 6): string {
  const v = toBig(value)
  const neg = v < 0n
  const abs = neg ? -v : v
  const base = 10n ** BigInt(Math.max(0, decimals))
  const int = abs / base
  let frac = decimals > 0 ? (abs % base).toString().padStart(decimals, '0').slice(0, maxFraction).replace(/0+$/, '') : ''
  if (!frac && int === 0n && abs > 0n) {
    // Tiny non-zero amount: never print "0" for dust (it matters for spotting poisoning).
    return `${neg ? '-' : ''}<0.${'0'.repeat(Math.max(0, maxFraction - 1))}1`
  }
  frac = frac ? `.${frac}` : ''
  return `${neg ? '-' : ''}${group(int.toString())}${frac}`
}

export function formatNative(value: string | bigint, symbol = 'tXDS', maxFraction = 6): string {
  return `${formatUnits(value, 18, maxFraction)} ${symbol}`
}

export function formatGwei(value: string | bigint | null | undefined): string {
  if (value === null || value === undefined) return '—'
  return `${formatUnits(value, 9, 4)} gwei`
}

export function formatToken(value: string | bigint, token: { decimals: number | null; symbol: string | null }, unlimited = false): string {
  if (unlimited) return `Unlimited ${token.symbol ?? ''}`.trim()
  const d = token.decimals ?? 18
  return `${formatUnits(value, d)} ${token.symbol ?? (token.decimals === null ? 'units (raw)' : '')}`.trim()
}

/** `0x1234ab…cd5678`: keeps 6 + 6 hex chars so 4/4 look-alikes still differ visibly. */
export function shortAddress(a: string | null | undefined, n = 6): string {
  if (!a) return '—'
  if (a.length <= 2 + n * 2 + 1) return a
  return `${a.slice(0, 2 + n)}…${a.slice(-n)}`
}

export function shortHash(h: string | null | undefined, n = 8): string {
  if (!h) return '—'
  if (h.length <= 2 + n * 2) return h
  return `${h.slice(0, 2 + n)}…${h.slice(-n)}`
}

export function formatNumber(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '—'
  return group(Math.trunc(n).toString())
}

export function formatDuration(seconds: number): string {
  const s = Math.abs(Math.round(seconds))
  if (s < 60) return `${s} s`
  if (s < 3600) return `${Math.floor(s / 60)} min`
  if (s < 86400) {
    const h = Math.floor(s / 3600)
    const m = Math.floor((s % 3600) / 60)
    return m ? `${h} h ${m} min` : `${h} h`
  }
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  return h ? `${d} d ${h} h` : `${d} d`
}

export function timeAgo(ts: number, now: number): string {
  const diff = now - ts
  if (diff < 0) return `in ${formatDuration(-diff)}`
  if (diff < 2) return 'just now'
  return `${formatDuration(diff)} ago`
}

export function formatDate(ts: number): string {
  return new Date(ts * 1000).toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC')
}

/** Expiry badge for approvals (plan §16.5): "never expires" is the dangerous case. */
export function expiryBadge(expiry: number | null, now: number): { text: string; tone: Tone } {
  if (expiry === null) return { text: 'Never expires', tone: 'danger' }
  if (expiry <= now) return { text: `Expired ${formatDuration(now - expiry)} ago`, tone: 'neutral' }
  const left = expiry - now
  return { text: `Expires in ${formatDuration(left)}`, tone: left < 86400 ? 'warn' : 'ok' }
}

const SPAM_LABELS: Record<string, string> = {
  'zero-value': 'Zero value',
  dust: 'Dust amount',
  'look-alike': 'Look-alike address',
  'unknown-token': 'Unknown token',
  'impersonating-token': 'Fake token (copies a known symbol)',
}
export const spamReasonLabel = (r: string): string => SPAM_LABELS[r] ?? r

export function vaultStatusTone(s: string): Tone {
  switch (s) {
    case 'queued':
      return 'warn'
    case 'ready':
      return 'danger'
    case 'executed':
      return 'ok'
    case 'cancelled':
    case 'invalidated':
    case 'expired':
      return 'neutral'
    default:
      return 'info'
  }
}

export const TX_TYPES: Record<number, string> = { 0: 'Legacy', 1: 'EIP-2930', 2: 'EIP-1559', 3: 'Blob', 4: 'EIP-7702 (set code)' }

/** Classifies a search box input so the UI can route without a round trip when the answer is obvious. */
export type SearchTarget =
  | { type: 'block'; id: string }
  | { type: 'address'; id: string }
  | { type: 'hash'; id: string }
  | { type: 'name'; id: string }
  | { type: 'text'; id: string }
  | { type: 'empty' }

export function classifySearch(raw: string): SearchTarget {
  const q = raw.trim()
  if (!q) return { type: 'empty' }
  if (/^\d{1,15}$/.test(q)) return { type: 'block', id: String(Number(q)) }
  if (/^0x[0-9a-fA-F]{40}$/.test(q)) return { type: 'address', id: q }
  if (/^0x[0-9a-fA-F]{64}$/.test(q)) return { type: 'hash', id: q.toLowerCase() }
  if (/^[a-z0-9-]{3,63}\.xdevs$/i.test(q)) return { type: 'name', id: q.toLowerCase() }
  return { type: 'text', id: q }
}

export function percent(part: bigint, whole: bigint): string {
  if (whole === 0n) return '0%'
  const bp = (part * 10000n) / whole
  return `${(Number(bp) / 100).toFixed(bp % 100n === 0n ? 0 : 2)}%`
}

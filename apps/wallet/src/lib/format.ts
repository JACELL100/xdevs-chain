// Human-readable formatting for amounts, addresses and times. Pure (shared by the UI and the tests).
import { formatUnits, parseUnits } from 'viem'

/** `0x1234…abcd` (4 + 4 hex chars: exactly what address-poisoning scams imitate, so never show less). */
export const shortAddr = (a: string, n = 4) => (a.length > 2 + 2 * n ? `${a.slice(0, 2 + n)}…${a.slice(-n)}` : a)

/** Splits an address into groups of 4 hex chars for careful visual comparison. */
export function groupAddr(a: string): string[] {
  const hex = a.startsWith('0x') ? a.slice(2) : a
  const out: string[] = []
  for (let i = 0; i < hex.length; i += 4) out.push(hex.slice(i, i + 4))
  return out
}

/**
 * Formats a token amount with thousands separators and at most `maxDigits` fraction digits (truncated, never
 * rounded up, so the wallet never shows more than you have). Tiny non-zero amounts show as "< 0.0001".
 */
export function fmtAmount(v: bigint, decimals = 18, maxDigits = 4): string {
  const neg = v < 0n
  const s = formatUnits(neg ? -v : v, decimals)
  const [i, f = ''] = s.split('.')
  const frac = f.slice(0, maxDigits).replace(/0+$/, '')
  if (v !== 0n && i === '0' && !frac) return `< 0.${'0'.repeat(Math.max(0, maxDigits - 1))}1`
  const int = i.replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return `${neg ? '-' : ''}${int}${frac ? `.${frac}` : ''}`
}

/** Exact amount (every digit), with separators: for confirmations. */
export function fmtExact(v: bigint, decimals = 18): string {
  return fmtAmount(v, decimals, decimals)
}

/**
 * Strict amount parsing for user input: digits with one optional dot, no exponent, no sign, at most `decimals`
 * fraction digits. Commas and spaces are rejected rather than guessed (1,5 could mean 1.5 or 15).
 */
export function parseAmount(input: string, decimals = 18): bigint {
  const s = input.trim()
  if (!s) throw new Error('Enter an amount')
  if (!/^\d*\.?\d*$/.test(s) || s === '.') throw new Error('Use digits and at most one dot, e.g. 12.5')
  const frac = s.split('.')[1] ?? ''
  if (frac.length > decimals) throw new Error(`At most ${decimals} decimal places`)
  return parseUnits(s.startsWith('.') ? `0${s}` : s, decimals)
}

export function fmtCountdown(seconds: number): string {
  if (seconds <= 0) return 'now'
  const d = Math.floor(seconds / 86_400)
  const h = Math.floor((seconds % 86_400) / 3_600)
  const m = Math.floor((seconds % 3_600) / 60)
  const s = Math.floor(seconds % 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m`
  return `${m}m ${String(s).padStart(2, '0')}s`
}

/** "24 hours", "1 hour", "3 days", "90 minutes" — for policy explanations. */
export function fmtDuration(seconds: number): string {
  if (seconds % 86_400 === 0 && seconds >= 2 * 86_400) return `${seconds / 86_400} days`
  if (seconds % 3_600 === 0) return seconds === 3_600 ? '1 hour' : `${seconds / 3_600} hours`
  return `${Math.round(seconds / 60)} minutes`
}

export function fmtRelative(unixSeconds: number, now = Date.now() / 1000): string {
  const d = Math.round(now - unixSeconds)
  if (d < 45) return 'just now'
  if (d < 3_600) return `${Math.round(d / 60)} min ago`
  if (d < 86_400) return `${Math.round(d / 3_600)} h ago`
  if (d < 7 * 86_400) return `${Math.round(d / 86_400)} d ago`
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

export function fmtDate(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' })
}

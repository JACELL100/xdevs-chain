// Spam and address-poisoning heuristics (plan §16.3, explorer-api.md §3). Pure functions; the indexer supplies history.

import { isLookAlikeAddress } from '@xdevs/sdk'
import type { Address } from 'viem'

export type SpamReason = 'zero-value' | 'dust' | 'look-alike' | 'unknown-token' | 'impersonating-token'

export interface TokenInfo {
  known: boolean
  symbol: string | null
  decimals: number | null
}

export interface SpamInput {
  kind: 'native' | 'erc20'
  value: bigint
  token: TokenInfo | null
  /** Set when the counterparty is a look-alike of an earlier counterparty (only checked for zero/dust). */
  lookAlikeOf: string | null
  /** Upper-cased symbols of known tokens (plus the native symbols). */
  protectedSymbols: ReadonlySet<string>
  dustNativeWei: bigint
}

export const NATIVE_SYMBOLS = ['TXDS', 'XDS'] as const

export function isDust(kind: 'native' | 'erc20', value: bigint, decimals: number | null, dustNativeWei: bigint): boolean {
  if (value === 0n) return false
  if (kind === 'native') return value < dustNativeWei
  const d = decimals ?? 18
  const threshold = d >= 3 ? 10n ** BigInt(d - 3) : 1n
  return value < threshold
}

/** Zero or dust: the transfers that are cheap enough to be used for poisoning, so the look-alike check applies. */
export function isPoisonCandidate(input: Pick<SpamInput, 'kind' | 'value' | 'token' | 'dustNativeWei'>): boolean {
  return input.value === 0n || isDust(input.kind, input.value, input.token?.decimals ?? null, input.dustNativeWei)
}

export function classifyTransfer(input: SpamInput): { spam: boolean; reasons: SpamReason[] } {
  const reasons: SpamReason[] = []
  const zero = input.value === 0n
  const dust = isDust(input.kind, input.value, input.token?.decimals ?? null, input.dustNativeWei)
  const unknown = input.kind === 'erc20' && !(input.token?.known ?? false)
  const symbol = input.token?.symbol?.trim().toUpperCase() ?? null
  const impersonating = unknown && symbol !== null && input.protectedSymbols.has(symbol)
  if (zero) reasons.push('zero-value')
  if (dust) reasons.push('dust')
  if (input.lookAlikeOf && (zero || dust)) reasons.push('look-alike')
  if (unknown) reasons.push('unknown-token')
  if (impersonating) reasons.push('impersonating-token')
  const spam = zero || reasons.includes('look-alike') || impersonating || (dust && unknown)
  return { spam, reasons }
}

/** First entry of `history` (other than `candidate`) that `candidate` imitates (same first 4 / last 4 hex chars). */
export function findLookAlike(candidate: string, history: Iterable<string>): string | null {
  for (const h of history) {
    if (h.toLowerCase() === candidate.toLowerCase()) continue
    if (isLookAlikeAddress(candidate as Address, h as Address, 4)) return h
  }
  return null
}

/** SQL-friendly fingerprint: first 4 and last 4 hex chars (lowercase), used to narrow the history query. */
export function fingerprint(address: string): { prefix: string; suffix: string } {
  const a = address.toLowerCase()
  return { prefix: a.slice(0, 6), suffix: a.slice(-4) }
}

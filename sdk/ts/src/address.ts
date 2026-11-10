import { type Address, isAddress } from 'viem'

/**
 * True if `a` and `b` are *different* addresses that share the first `n` and last `n` hex characters
 * (case-insensitive) — the pattern used by address-poisoning scams (plan §16.3). Wallets typically
 * shorten addresses to `0x1234…abcd`, so such pairs look identical at a glance.
 *
 * @param n number of leading and trailing hex chars to compare (1–20, default 4; 4–6 is typical).
 */
export function isLookAlikeAddress(a: Address, b: Address, n = 4): boolean {
  if (!Number.isInteger(n) || n < 1 || n > 20) throw new RangeError(`n must be an integer in 1..20, got ${n}`)
  if (!isAddress(a, { strict: false }) || !isAddress(b, { strict: false }))
    throw new TypeError(`invalid address: ${isAddress(a, { strict: false }) ? b : a}`)
  const x = a.slice(2).toLowerCase()
  const y = b.slice(2).toLowerCase()
  if (x === y) return false
  return x.slice(0, n) === y.slice(0, n) && x.slice(-n) === y.slice(-n)
}

/** Returns the entries of `known` (e.g. past recipients) that are look-alikes of `target`. */
export function findLookAlikeAddresses(target: Address, known: readonly Address[], n = 4): Address[] {
  return known.filter((k) => isLookAlikeAddress(target, k, n))
}

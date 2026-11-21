// Operator names of the Enclave genesis validators (no XNS reverse records yet), and uptime from recent commit
// certificates.

import { consensusAddress } from './chainstats.ts'
import type { CommitDetail, ValidatorRow } from './types.ts'

/** Enclave chain id. */
const ENCLAVE = 717171

/** Enclave's four validators by operator address (lowercase). */
const ENCLAVE_NAMES: Readonly<Record<string, string>> = {
  '0x4e0a69305925f213d717e002ef7e4bae4f5b0402': 'val-1 · Hyderabad',
  '0x4f3cd5fb70f67d8c4851a48c9317833612cf8393': 'val-2 · Hyderabad',
  '0x187f5eea6730c7c7ac7ba1cc8994420ecbb6d6f2': 'val-3 · Mumbai',
  '0x42073689894ba795ff54d4aa2b528f5b48280782': 'val-4 · Mumbai',
}

/** Static name of an operator on `chainId`, if known. */
export function knownValidatorName(chainId: number, operator: string): string | null {
  return chainId === ENCLAVE ? (ENCLAVE_NAMES[operator.toLowerCase()] ?? null) : null
}

/** Commit certificates read for the uptime column. */
export const UPTIME_WINDOW = 100

/**
 * Uptime in basis points: the share of `certs` that carry a signature of each signing-set validator (matched by
 * consensus address, or by operator). `null` for validators outside the signing set or when there are no certs.
 */
export function uptimeFromCerts(validators: readonly ValidatorRow[], certs: readonly CommitDetail[]): (number | null)[] {
  return validators.map((v) => {
    if (!v.inSigningSet || certs.length === 0) return null
    const ids = new Set([consensusAddress(v.consensusKey), v.operator.toLowerCase()])
    const signed = certs.filter((c) => c.signatures.some((s) => ids.has(s.validator.toLowerCase()))).length
    return Math.round((signed * 10_000) / certs.length)
  })
}

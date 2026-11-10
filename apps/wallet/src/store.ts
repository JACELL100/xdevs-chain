// Local persistence (browser localStorage). Only PUBLIC data is stored for passkey accounts (credential id, public
// key, init params). The dev-only virtual passkey / raw keys are the exception and are labelled as such in the UI.
import type { Address, Hex } from 'viem'
import type { InitParams } from './lib/account.ts'
import type { StoredPasskey } from './lib/signers.ts'

export type AccountRecord = {
  address: Address
  label: string
  kind: 'passkey' | 'virtual' | 'dev-key' | 'watch'
  passkey?: StoredPasskey
  /** DEV ONLY: private JWK of the virtual passkey. */
  virtualJwk?: JsonWebKey
  /** DEV ONLY: raw secp256k1 key (EIP-7702 EOA). */
  devKey?: Hex
  /** Counterfactual deployment data (factory accounts). */
  params?: InitParams
  salt?: Hex
  createdAt: number
}

type Persisted = { accounts: AccountRecord[]; active: number; history: Record<string, Address[]> }

const KEY = 'xdevs-wallet:v1'

const replacer = (_k: string, v: unknown) => (typeof v === 'bigint' ? { $bigint: v.toString() } : v)
const reviver = (_k: string, v: unknown) =>
  v && typeof v === 'object' && '$bigint' in (v as Record<string, unknown>) ? BigInt((v as { $bigint: string }).$bigint) : v

export function load(): Persisted {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return JSON.parse(raw, reviver) as Persisted
  } catch {
    /* storage blocked or corrupt */
  }
  return { accounts: [], active: -1, history: {} }
}

export function save(p: Persisted): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(p, replacer))
  } catch {
    /* storage blocked: the session still works in memory */
  }
}

export function addAccount(rec: AccountRecord): Persisted {
  const p = load()
  const i = p.accounts.findIndex((a) => a.address.toLowerCase() === rec.address.toLowerCase())
  if (i >= 0) p.accounts[i] = { ...p.accounts[i], ...rec }
  else p.accounts.push(rec)
  p.active = i >= 0 ? i : p.accounts.length - 1
  save(p)
  return p
}

export function rememberRecipient(account: Address, to: Address): void {
  const p = load()
  const h = p.history[account.toLowerCase()] ?? []
  if (!h.some((x) => x.toLowerCase() === to.toLowerCase())) h.unshift(to)
  p.history[account.toLowerCase()] = h.slice(0, 200)
  save(p)
}

export function recipientHistory(account: Address): Address[] {
  return load().history[account.toLowerCase()] ?? []
}

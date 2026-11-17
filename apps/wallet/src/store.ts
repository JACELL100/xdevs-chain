// Local persistence (browser localStorage). Only PUBLIC data is stored for passkey accounts (credential id, public
// key, init params). The dev-only virtual passkey / raw keys are the exception and are labelled as such in the UI.
import type { Address, Hex } from 'viem'
import type { ActivityItem } from './data/types.ts'
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

export type Persisted = { accounts: AccountRecord[]; active: number; history: Record<string, Address[]> }

const KEY = 'xdevs-wallet:v1'
const ACTIVITY_KEY = 'xdevs-wallet:v1:activity'

const replacer = (_k: string, v: unknown) => (typeof v === 'bigint' ? { $bigint: v.toString() } : v)
const reviver = (_k: string, v: unknown) =>
  v && typeof v === 'object' && '$bigint' in (v as Record<string, unknown>) ? BigInt((v as { $bigint: string }).$bigint) : v

const storage = (): Storage | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

export function load(): Persisted {
  try {
    const raw = storage()?.getItem(KEY)
    if (raw) {
      const p = JSON.parse(raw, reviver) as Persisted
      if (Array.isArray(p.accounts)) return { accounts: p.accounts, active: p.active ?? -1, history: p.history ?? {} }
    }
  } catch {
    /* storage blocked or corrupt */
  }
  return { accounts: [], active: -1, history: {} }
}

export function save(p: Persisted): void {
  try {
    storage()?.setItem(KEY, JSON.stringify(p, replacer))
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

export function setActive(i: number): Persisted {
  const p = { ...load(), active: i }
  save(p)
  return p
}

/** Removes an account from THIS browser only (the on-chain account is untouched). */
export function forgetAccount(address: Address): Persisted {
  const p = load()
  p.accounts = p.accounts.filter((a) => a.address.toLowerCase() !== address.toLowerCase())
  p.active = p.accounts.length ? 0 : -1
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

// ------------------------------------------------------------------------------------------- local activity
// Operations sent from this browser (used when no explorer API is configured, and for queued vault actions).
export function localActivity(account: Address): ActivityItem[] {
  try {
    const raw = storage()?.getItem(ACTIVITY_KEY)
    const all = raw ? (JSON.parse(raw, reviver) as Record<string, ActivityItem[]>) : {}
    return all[account.toLowerCase()] ?? []
  } catch {
    return []
  }
}

export function recordActivity(account: Address, item: ActivityItem): void {
  try {
    const s = storage()
    if (!s) return
    const raw = s.getItem(ACTIVITY_KEY)
    const all = raw ? (JSON.parse(raw, reviver) as Record<string, ActivityItem[]>) : {}
    const k = account.toLowerCase()
    all[k] = [item, ...(all[k] ?? []).filter((x) => x.id !== item.id)].slice(0, 100)
    s.setItem(ACTIVITY_KEY, JSON.stringify(all, replacer))
  } catch {
    /* ignore */
  }
}

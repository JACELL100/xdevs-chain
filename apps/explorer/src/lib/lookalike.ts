// Look-alike warning (plan §16.3): remembers the addresses this browser recently viewed or searched, and warns when a
// new address shares the first 4 and last 4 hex chars with one of them but is a different address. Data stays in
// localStorage and never leaves the device.

import { isLookAlikeAddress } from '@xdevs/sdk'

export interface HistoryEntry {
  address: string
  at: number
  source: 'view' | 'search'
}

export interface KeyValueStore {
  get(): string | null
  set(value: string): void
}

export const HISTORY_KEY = 'xdevs-explorer:recent-addresses'
export const HISTORY_MAX = 50

const isAddr = (a: unknown): a is string => typeof a === 'string' && /^0x[0-9a-fA-F]{40}$/.test(a)

export function loadHistory(store: KeyValueStore): HistoryEntry[] {
  try {
    const raw = JSON.parse(store.get() ?? '[]') as unknown
    if (!Array.isArray(raw)) return []
    return raw
      .filter((e): e is HistoryEntry => !!e && isAddr((e as HistoryEntry).address) && typeof (e as HistoryEntry).at === 'number')
      .map((e): HistoryEntry => ({ address: e.address, at: e.at, source: e.source === 'search' ? 'search' : 'view' }))
      .slice(0, HISTORY_MAX)
  } catch {
    return []
  }
}

/** Adds `address` to the front of the history (deduplicated, newest first, capped). Returns the new list. */
export function recordAddress(store: KeyValueStore, address: string, source: HistoryEntry['source'], now: number): HistoryEntry[] {
  if (!isAddr(address)) return loadHistory(store)
  const list = loadHistory(store).filter((e) => e.address.toLowerCase() !== address.toLowerCase())
  list.unshift({ address, at: now, source })
  const out = list.slice(0, HISTORY_MAX)
  try {
    store.set(JSON.stringify(out))
  } catch {
    /* storage full or disabled */
  }
  return out
}

/** History entries that `address` imitates (same first 4 / last 4 hex chars, different address). */
export function findLookAlikes(address: string, history: readonly HistoryEntry[], n = 4): HistoryEntry[] {
  if (!isAddr(address)) return []
  return history.filter((e) => isLookAlikeAddress(address as `0x${string}`, e.address as `0x${string}`, n))
}

/**
 * Splits an address for display so the parts that poisoners copy (start and end) and the middle they cannot copy are
 * both visible: `{ head: '0x1234', middle: 'abcd…', tail: '5678' }`.
 */
export function addressParts(address: string, n = 4): { head: string; middle: string; tail: string } {
  return { head: address.slice(0, 2 + n), middle: address.slice(2 + n, address.length - n), tail: address.slice(-n) }
}

export function browserStore(key = HISTORY_KEY): KeyValueStore {
  return {
    get() {
      try {
        return globalThis.localStorage?.getItem(key) ?? null
      } catch {
        return null
      }
    },
    set(v) {
      try {
        globalThis.localStorage?.setItem(key, v)
      } catch {
        /* ignore */
      }
    },
  }
}

export function memoryStore(initial: string | null = null): KeyValueStore {
  let v = initial
  return { get: () => v, set: (x) => void (v = x) }
}

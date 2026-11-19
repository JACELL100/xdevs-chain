// ERC-7562 entity reputation: opsSeen / opsIncluded counters with hourly decay, OK / THROTTLED / BANNED status.
import { type Address, getAddress } from 'viem'

export type ReputationParams = {
  /** ERC-7562 MIN_INCLUSION_RATE_DENOMINATOR (10 for bundlers). */
  minInclusionRateDenominator: number
  /** ERC-7562 THROTTLING_SLACK (10). */
  throttlingSlack: number
  /** ERC-7562 BAN_SLACK (50). */
  banSlack: number
  /** Never throttled / banned. */
  allowlist?: Address[]
}

export type EntityStatus = 'ok' | 'throttled' | 'banned'
export type ReputationEntry = { opsSeen: number; opsIncluded: number }
export type ReputationSnapshot = { lastDecay: number; entries: Record<string, ReputationEntry> }

/** ERC-7562 BAN_OPS_SEEN_PENALTY: an entity that made a bundle revert on chain. */
export const BAN_OPS_SEEN_PENALTY = 10_000
const HOUR = 3_600_000

export class Reputation {
  private readonly entries = new Map<string, ReputationEntry>()
  private readonly allow: Set<string>
  private lastDecay: number
  private readonly p: ReputationParams
  private readonly now: () => number

  constructor(p: ReputationParams, now: () => number = Date.now) {
    this.p = p
    this.now = now
    this.allow = new Set((p.allowlist ?? []).map((a) => a.toLowerCase()))
    this.lastDecay = now()
  }

  private entry(a: Address): ReputationEntry {
    const k = a.toLowerCase()
    let e = this.entries.get(k)
    if (!e) {
      e = { opsSeen: 0, opsIncluded: 0 }
      this.entries.set(k, e)
    }
    return e
  }

  get(a: Address): ReputationEntry {
    return { ...(this.entries.get(a.toLowerCase()) ?? { opsSeen: 0, opsIncluded: 0 }) }
  }

  /** An op using this entity entered the mempool. */
  seen(a: Address): void {
    this.decay()
    this.entry(a).opsSeen++
  }

  /** An op using this entity was included on chain. */
  included(a: Address): void {
    this.decay()
    this.entry(a).opsIncluded++
  }

  /** The entity caused a bundle to revert on chain (or failed validation inside a submitted bundle): ban it. */
  crashed(a: Address): void {
    if (this.allow.has(a.toLowerCase())) return
    const e = this.entry(a)
    e.opsSeen = BAN_OPS_SEEN_PENALTY
    e.opsIncluded = 0
  }

  status(a: Address): EntityStatus {
    if (this.allow.has(a.toLowerCase())) return 'ok'
    this.decay()
    const e = this.entries.get(a.toLowerCase())
    if (!e) return 'ok'
    const minExpectedIncluded = Math.floor(e.opsSeen / this.p.minInclusionRateDenominator)
    if (minExpectedIncluded <= e.opsIncluded + this.p.throttlingSlack) return 'ok'
    if (minExpectedIncluded <= e.opsIncluded + this.p.banSlack) return 'throttled'
    return 'banned'
  }

  /**
   * Mempool allowance of a STAKED entity (ERC-7562 "SAME_UNSTAKED_ENTITY_MEMPOOL_COUNT + inclusionRate *
   * INCLUDED_RATE_FACTOR + min(opsIncluded, 10000)"). Unstaked entities get `base` only.
   */
  stakedAllowance(a: Address, base: number): number {
    const e = this.get(a)
    const rate = e.opsSeen === 0 ? 0 : e.opsIncluded / e.opsSeen
    return base + Math.floor(rate * 10) + Math.min(e.opsIncluded, 10_000)
  }

  /** Hourly decay: `x = x * 23 / 24` for both counters (ERC-7562), rounded down; empty entries are forgotten. */
  decay(): void {
    const t = this.now()
    let hours = Math.floor((t - this.lastDecay) / HOUR)
    if (hours <= 0) return
    this.lastDecay += hours * HOUR
    hours = Math.min(hours, 24 * 30)
    for (const [k, e] of this.entries) {
      for (let i = 0; i < hours && (e.opsSeen > 0 || e.opsIncluded > 0); i++) {
        e.opsSeen = Math.floor((e.opsSeen * 23) / 24)
        e.opsIncluded = Math.floor((e.opsIncluded * 23) / 24)
      }
      if (e.opsSeen <= 0 && e.opsIncluded <= 0) this.entries.delete(k)
    }
  }

  counts(): Record<EntityStatus, number> {
    const c = { ok: 0, throttled: 0, banned: 0 }
    for (const k of this.entries.keys()) c[this.status(getAddress(k))]++
    return c
  }

  snapshot(): ReputationSnapshot {
    return { lastDecay: this.lastDecay, entries: Object.fromEntries(this.entries) }
  }

  restore(s: ReputationSnapshot | undefined): void {
    if (!s) return
    this.lastDecay = typeof s.lastDecay === 'number' ? s.lastDecay : this.now()
    for (const [k, e] of Object.entries(s.entries ?? {})) {
      if (/^0x[0-9a-f]{40}$/.test(k) && Number.isFinite(e.opsSeen) && Number.isFinite(e.opsIncluded)) {
        this.entries.set(k, { opsSeen: e.opsSeen, opsIncluded: e.opsIncluded })
      }
    }
  }
}

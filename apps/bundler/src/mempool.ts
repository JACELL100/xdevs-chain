// In-memory UserOperation mempool with the ERC-7562 admission limits and bundle selection (pure: no RPC).
import type { Address, Hex } from 'viem'
import type { BundlerConfig } from './config.ts'
import { RpcCodes, RpcError } from './errors.ts'
import type { EntityStatus } from './reputation.ts'
import { effectiveTip, opGasLimit } from './rules.ts'
import type { UserOperation } from './userop.ts'

export type PoolEntry = {
  hash: Hex
  op: UserOperation
  nonceKey: bigint
  nonceSeq: bigint
  /** Real factory (not the 7702 marker), lower-case. */
  factory?: string
  /** Paymaster, lower-case. */
  paymaster?: string
  maxCost: bigint
  /** ms since epoch. */
  addedAt: number
  addedBlock: bigint
  /** Unix seconds; 0 = no expiry. */
  validUntil: number
  validAfter: number
  status: 'pending' | 'submitted'
  txHash?: Hex
  /** Times it went into a bundle that did not land. */
  attempts: number
  /** Counted by the sponsorship policy. */
  sponsored: boolean
}

const lc = (a: string) => a.toLowerCase()
const U64 = (1n << 64n) - 1n

export const nonceParts = (nonce: bigint) => ({ key: nonce >> 64n, seq: nonce & U64 })

export type AdmissionContext = {
  /** On-chain sequence of the op's nonce key (`EntryPoint.getNonce(sender, key) & (2^64-1)`). */
  onchainSeq: bigint
  status: (a: string) => EntityStatus
  isStaked: (a: string) => boolean
  stakedAllowance: (a: string, base: number) => number
  /** EntryPoint deposit of the op's paymaster (undefined: no paymaster). */
  paymasterDeposit?: bigint
  limits: Pick<
    BundlerConfig,
    'maxMempoolOps' | 'sameSenderMempoolCount' | 'sameUnstakedEntityMempoolCount' | 'throttledEntityMempoolCount'
  >
}

export type SelectOptions = {
  baseFee: bigint
  /** Senders / factories / paymasters (lower-case) used by in-flight bundles: their ops wait. */
  busy: Set<string>
  maxOps: number
  maxGas: bigint
  nowSec: number
  /** Ops expiring sooner than this (seconds) are not bundled. */
  minValidSeconds: number
  status: (a: string) => EntityStatus
  throttledBundleCount: number
}

export class Mempool {
  private readonly byHash = new Map<Hex, PoolEntry>()

  get size(): number {
    return this.byHash.size
  }
  get(hash: Hex): PoolEntry | undefined {
    return this.byHash.get(hash)
  }
  all(): PoolEntry[] {
    return [...this.byHash.values()]
  }
  remove(hash: Hex): PoolEntry | undefined {
    const e = this.byHash.get(hash)
    this.byHash.delete(hash)
    return e
  }
  put(e: PoolEntry): void {
    this.byHash.set(e.hash, e)
  }

  bySender(sender: string, key?: bigint): PoolEntry[] {
    const s = lc(sender)
    return this.all().filter((e) => lc(e.op.sender) === s && (key === undefined || e.nonceKey === key))
  }

  /** Ops using `a` as factory or paymaster. */
  entityOps(a: string): PoolEntry[] {
    const x = lc(a)
    return this.all().filter((e) => e.factory === x || e.paymaster === x)
  }

  isEntity(a: string): boolean {
    return this.entityOps(a).length > 0
  }
  isSender(a: string): boolean {
    return this.bySender(a).length > 0
  }

  /**
   * ERC-7562 mempool rules for a validated candidate. Returns the entry it replaces (same sender + nonce), if any.
   * Throws an ERC-7769 RpcError otherwise.
   */
  checkAdmission(c: PoolEntry, ctx: AdmissionContext): PoolEntry | undefined {
    if (this.byHash.has(c.hash)) throw new RpcError(RpcCodes.invalidParams, `UserOperation ${c.hash} is already in the mempool`)
    const sender = lc(c.op.sender)
    const own = this.bySender(sender)
    const same = own.find((e) => e.op.nonce === c.op.nonce)
    if (same) {
      if (same.status === 'submitted') {
        throw new RpcError(RpcCodes.invalidParams, 'an op with this sender and nonce is already in a submitted bundle')
      }
      const bump = (v: bigint) => (v * 110n + 99n) / 100n
      if (c.op.maxFeePerGas < bump(same.op.maxFeePerGas) || c.op.maxPriorityFeePerGas < bump(same.op.maxPriorityFeePerGas)) {
        throw new RpcError(
          RpcCodes.invalidParams,
          'replacement underpriced: raise both maxFeePerGas and maxPriorityFeePerGas by at least 10 %',
        )
      }
    }
    // Nonce: not used yet, and no gap after the on-chain nonce (lower sequences of the same key must be pending here).
    if (c.nonceSeq < ctx.onchainSeq) {
      throw new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `AA25 invalid account nonce: ${c.nonceSeq} already used`)
    }
    const seqs = new Set(own.filter((e) => e.nonceKey === c.nonceKey).map((e) => e.nonceSeq))
    for (let s = ctx.onchainSeq; s < c.nonceSeq; s++) {
      if (!seqs.has(s)) throw new RpcError(RpcCodes.invalidParams, `nonce gap: sequence ${s} of key ${c.nonceKey} is not pending`)
    }
    const others = (list: PoolEntry[]) => list.filter((e) => e !== same)
    if (!same && this.size >= ctx.limits.maxMempoolOps) throw new RpcError(RpcCodes.limitExceeded, 'mempool full, retry later')

    // Sender limits.
    if (ctx.status(sender) === 'banned') throw new RpcError(RpcCodes.bannedOrThrottled, `sender ${c.op.sender} is banned`)
    const senderLimit = ctx.isStaked(sender)
      ? ctx.stakedAllowance(sender, ctx.limits.sameSenderMempoolCount)
      : ctx.limits.sameSenderMempoolCount
    if (others(own).length >= senderLimit) {
      throw new RpcError(RpcCodes.bannedOrThrottled, `sender has ${senderLimit} ops in the mempool already: wait for inclusion`)
    }
    // STO-040: an address is either a sender or a factory / paymaster in the mempool, not both.
    if (this.isEntity(sender)) {
      throw new RpcError(RpcCodes.opcodeOrStorageViolation, `sender ${c.op.sender} is used as a factory or paymaster by other ops`)
    }
    for (const [role, ent] of [['factory', c.factory], ['paymaster', c.paymaster]] as const) {
      if (!ent) continue
      if (this.isSender(ent)) {
        throw new RpcError(RpcCodes.opcodeOrStorageViolation, `${role} ${ent} is the sender of other ops in the mempool`)
      }
      const st = ctx.status(ent)
      if (st === 'banned') throw new RpcError(RpcCodes.bannedOrThrottled, `${role} ${ent} is banned (reputation)`)
      const count = others(this.entityOps(ent)).length
      if (st === 'throttled' && count >= ctx.limits.throttledEntityMempoolCount) {
        throw new RpcError(RpcCodes.bannedOrThrottled, `${role} ${ent} is throttled (reputation): retry later`)
      }
      const staked = ctx.isStaked(ent)
      const limit = staked
        ? ctx.stakedAllowance(ent, ctx.limits.sameUnstakedEntityMempoolCount)
        : ctx.limits.sameUnstakedEntityMempoolCount
      if (count >= limit) {
        throw new RpcError(
          RpcCodes.bannedOrThrottled,
          `${role} ${ent} has ${count} ops in the mempool (limit ${limit}${staked ? '' : ' for unstaked entities'}): retry later`,
        )
      }
    }
    // The paymaster's EntryPoint deposit must cover all its pending ops.
    if (c.paymaster && ctx.paymasterDeposit !== undefined) {
      const reserved = others(this.all())
        .filter((e) => e.paymaster === c.paymaster)
        .reduce((s, e) => s + e.maxCost, 0n)
      if (reserved + c.maxCost > ctx.paymasterDeposit) {
        throw new RpcError(RpcCodes.rejectedByPaymaster, 'paymaster deposit too low for its pending ops: retry later')
      }
    }
    return same
  }

  /**
   * Picks the next bundle: pending ops by effective tip, lowest nonce first per sender, entity-disjoint from in-flight
   * bundles, within the op-count and gas caps; throttled entities get at most `throttledBundleCount` ops.
   */
  select(o: SelectOptions): PoolEntry[] {
    const cands = this.all()
      .filter((e) => e.status === 'pending')
      .filter((e) => e.op.maxFeePerGas >= o.baseFee)
      .filter((e) => e.validAfter <= o.nowSec && (e.validUntil === 0 || e.validUntil >= o.nowSec + o.minValidSeconds))
      .filter((e) => ![lc(e.op.sender), e.factory, e.paymaster].some((x) => x && (o.busy.has(x) || o.status(x) === 'banned')))
      .sort((a, b) => {
        const d = effectiveTip(b.op, o.baseFee) - effectiveTip(a.op, o.baseFee)
        return d > 0n ? 1 : d < 0n ? -1 : a.addedAt - b.addedAt
      })
    const chosen: PoolEntry[] = []
    const inBundle = new Set<PoolEntry>()
    const perThrottled = new Map<string, number>()
    let gas = 0n
    let progress = true
    while (progress && chosen.length < o.maxOps) {
      progress = false
      for (const e of cands) {
        if (inBundle.has(e) || chosen.length >= o.maxOps) continue
        // Every lower sequence of the same sender + key still in the pool must already be in this bundle.
        const lower = this.bySender(e.op.sender, e.nonceKey).filter((x) => x.nonceSeq < e.nonceSeq)
        if (lower.some((x) => !inBundle.has(x))) continue
        const g = opGasLimit(e.op)
        if (gas + g > o.maxGas) continue
        const throttled = [e.factory, e.paymaster].filter((x): x is string => !!x && o.status(x) === 'throttled')
        if (throttled.some((x) => (perThrottled.get(x) ?? 0) >= o.throttledBundleCount)) continue
        for (const x of throttled) perThrottled.set(x, (perThrottled.get(x) ?? 0) + 1)
        chosen.push(e)
        inBundle.add(e)
        gas += g
        progress = true
      }
    }
    return chosen
  }

  /** Ops to drop now (expired, banned entities, throttled entities' ops older than `liveBlocks`). */
  stale(o: { nowMs: number; nowSec: number; block: bigint; ttlSeconds: number; liveBlocks: number; status: (a: string) => EntityStatus }): {
    entry: PoolEntry
    reason: string
  }[] {
    const out: { entry: PoolEntry; reason: string }[] = []
    for (const e of this.all()) {
      if (e.status !== 'pending') continue
      const ents = [lc(e.op.sender), e.factory, e.paymaster].filter((x): x is string => !!x)
      if (o.nowMs - e.addedAt > o.ttlSeconds * 1000) out.push({ entry: e, reason: 'expired in the mempool' })
      else if (e.validUntil !== 0 && e.validUntil <= o.nowSec) out.push({ entry: e, reason: 'validUntil passed' })
      else if (ents.some((x) => o.status(x) === 'banned')) out.push({ entry: e, reason: 'entity banned' })
      else if (ents.some((x) => o.status(x) === 'throttled') && o.block - e.addedBlock > BigInt(o.liveBlocks)) {
        out.push({ entry: e, reason: 'throttled entity, not included in time' })
      }
    }
    return out
  }

  /** Entities (lower-case) of the given entries. */
  static entities(entries: PoolEntry[]): Set<string> {
    const s = new Set<string>()
    for (const e of entries) {
      s.add(lc(e.op.sender))
      if (e.factory) s.add(e.factory)
      if (e.paymaster) s.add(e.paymaster)
    }
    return s
  }
}

export type { Address }

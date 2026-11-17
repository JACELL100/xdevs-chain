// Pure network-stats computations for the status page (unit tested).

export interface BlockSample {
  number: number
  /** Unix seconds (block header timestamp). */
  timestamp: number
  txCount?: number
}

export interface CommitCertificate {
  height: number
  round: number
  blockHash: string
  /** Lowercase 0x-prefixed 20-byte validator ids, in certificate order. */
  signers: string[]
}

export type Health = 'up' | 'degraded' | 'down' | 'unknown'

/** Parse a 0x quantity ("0x2a") or a number into a JS number. */
export function hexToNumber(v: unknown): number | null {
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v !== 'string' || !/^0x[0-9a-fA-F]+$/.test(v)) return null
  const n = Number.parseInt(v, 16)
  return Number.isSafeInteger(n) ? n : null
}

/** Normalize an `xdevs_getCommitCertificate` result (null for unknown blocks / genesis). */
export function parseCertificate(raw: unknown): CommitCertificate | null {
  if (typeof raw !== 'object' || raw === null) return null
  const r = raw as Record<string, unknown>
  const height = hexToNumber(r.height)
  if (height === null) return null
  const sigs = Array.isArray(r.signatures) ? r.signatures : []
  const signers: string[] = []
  for (const s of sigs) {
    const v = (s as Record<string, unknown> | null)?.validator
    if (typeof v === 'string' && /^0x[0-9a-fA-F]{40}$/.test(v)) signers.push(v.toLowerCase())
  }
  return {
    height,
    round: hexToNumber(r.round) ?? 0,
    blockHash: typeof r.blockHash === 'string' ? r.blockHash : '',
    signers: [...new Set(signers)],
  }
}

/** Insert/replace samples by block number, keep them sorted ascending and keep the last `max`. */
export function mergeBlocks(existing: BlockSample[], incoming: BlockSample[], max = 120): BlockSample[] {
  const byNumber = new Map<number, BlockSample>()
  for (const b of existing) byNumber.set(b.number, b)
  for (const b of incoming) byNumber.set(b.number, b)
  return [...byNumber.values()].sort((a, b) => a.number - b.number).slice(-max)
}

export interface BlockInterval {
  number: number
  seconds: number
}

/** Seconds between consecutive blocks (only for directly consecutive numbers). */
export function blockIntervals(blocks: BlockSample[]): BlockInterval[] {
  const out: BlockInterval[] = []
  for (let i = 1; i < blocks.length; i++) {
    const prev = blocks[i - 1]!
    const cur = blocks[i]!
    if (cur.number !== prev.number + 1) continue
    out.push({ number: cur.number, seconds: Math.max(0, cur.timestamp - prev.timestamp) })
  }
  return out
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return NaN
  const idx = (sorted.length - 1) * p
  const lo = Math.floor(idx)
  const hi = Math.ceil(idx)
  return sorted[lo]! + (sorted[hi]! - sorted[lo]!) * (idx - lo)
}

export interface BlockTimeStats {
  count: number
  mean: number
  median: number
  p95: number
  min: number
  max: number
}

export function blockTimeStats(intervals: BlockInterval[]): BlockTimeStats | null {
  if (intervals.length === 0) return null
  const values = intervals.map((i) => i.seconds).sort((a, b) => a - b)
  const sum = values.reduce((a, b) => a + b, 0)
  return {
    count: values.length,
    mean: sum / values.length,
    median: percentile(values, 0.5),
    p95: percentile(values, 0.95),
    min: values[0]!,
    max: values[values.length - 1]!,
  }
}

/** Head freshness: up while blocks arrive, degraded when late, down when the chain looks halted. */
export function headHealth(secondsSinceBlock: number | null, targetBlockTime = 2): Health {
  if (secondsSinceBlock === null) return 'unknown'
  if (secondsSinceBlock <= Math.max(10, targetBlockTime * 5)) return 'up'
  if (secondsSinceBlock <= 60) return 'degraded'
  return 'down'
}

export interface Finality {
  signed: number
  total: number
  /** signed / total in [0, 1]. */
  ratio: number
  /** BFT quorum: strictly more than 2/3 of the voting set. */
  quorum: boolean
}

/**
 * Finality of one certificate. Assumes equal voting power (the Enclave genesis set); when
 * `xdevs_validators` lands (Phase 3) pass power-weighted totals instead.
 */
export function finality(cert: CommitCertificate, validatorCount: number): Finality {
  const total = Math.max(validatorCount, cert.signers.length, 1)
  const signed = cert.signers.length
  return { signed, total, ratio: signed / total, quorum: signed * 3 > total * 2 }
}

export interface ValidatorStat {
  id: string
  name?: string
  lastSigned: number | null
  /** Certificates in the window this validator is absent from. */
  missed: number
  /** Certificates considered (window size actually available). */
  window: number
  uptime: number
}

/**
 * Per-validator participation over a window of certificates.
 * Set = configured validators ∪ everyone seen signing in the window.
 */
export function validatorStats(
  certs: CommitCertificate[],
  known: { id: string; name?: string }[] = [],
): ValidatorStat[] {
  const window = certs.length
  const ids = new Map<string, string | undefined>()
  for (const k of known) ids.set(k.id.toLowerCase(), k.name)
  for (const c of certs) for (const s of c.signers) if (!ids.has(s)) ids.set(s, undefined)
  const sorted = [...certs].sort((a, b) => a.height - b.height)
  const stats: ValidatorStat[] = []
  for (const [id, name] of ids) {
    let lastSigned: number | null = null
    let missed = 0
    for (const c of sorted) {
      if (c.signers.includes(id)) lastSigned = c.height
      else missed++
    }
    const s: ValidatorStat = { id, lastSigned, missed, window, uptime: window === 0 ? 0 : (window - missed) / window }
    if (name) s.name = name
    stats.push(s)
  }
  return stats.sort((a, b) => (a.name ?? a.id).localeCompare(b.name ?? b.id))
}

/** Classify a probe by outcome and latency. */
export function probeHealth(ok: boolean, latencyMs: number | null, slowMs = 1500): Health {
  if (!ok) return 'down'
  if (latencyMs !== null && latencyMs > slowMs) return 'degraded'
  return 'up'
}

/** Worst of several healths (down > degraded > unknown > up). */
export function worst(...hs: Health[]): Health {
  const rank: Record<Health, number> = { up: 0, unknown: 1, degraded: 2, down: 3 }
  return hs.reduce<Health>((acc, h) => (rank[h] > rank[acc] ? h : acc), 'up')
}

/** "3 s", "1 min 4 s", "2 h 5 min" */
export function formatAgo(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  if (s < 60) return `${s} s`
  const m = Math.floor(s / 60)
  if (m < 60) return s % 60 ? `${m} min ${s % 60} s` : `${m} min`
  const h = Math.floor(m / 60)
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`
}

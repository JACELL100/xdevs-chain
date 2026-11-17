// Network statistics computed from recent blocks (never claimed): average block time and throughput.

import { sha256, type Hex } from 'viem'

export interface BlockSample {
  number: number
  timestamp: number
  txCount: number
}

export interface ChainStats {
  height: number | null
  /** Seconds between blocks over the sample, `null` with fewer than 2 blocks. */
  avgBlockTime: number | null
  /** Transactions per second over the sample window. */
  tps: number | null
  /** Blocks the numbers are computed from. */
  sample: number
}

/** Sorts by height, drops duplicates and uses the span between the first and the last block of the window. */
export function computeStats(blocks: readonly BlockSample[]): ChainStats {
  const byNum = new Map<number, BlockSample>()
  for (const b of blocks) byNum.set(b.number, b)
  const s = [...byNum.values()].sort((a, b) => a.number - b.number)
  const last = s[s.length - 1]
  if (!last) return { height: null, avgBlockTime: null, tps: null, sample: 0 }
  const first = s[0]!
  if (s.length < 2 || last.number === first.number) return { height: last.number, avgBlockTime: null, tps: null, sample: s.length }
  const span = last.timestamp - first.timestamp
  const avgBlockTime = span / (last.number - first.number)
  // Transactions of the blocks AFTER the first one happened inside the span.
  const txs = s.slice(1).reduce((n, b) => n + b.txCount, 0)
  return { height: last.number, avgBlockTime, tps: span > 0 ? txs / span : null, sample: s.length }
}

export function formatTps(tps: number | null): string {
  if (tps === null || !Number.isFinite(tps)) return '—'
  if (tps === 0) return '0'
  if (tps < 0.01) return '<0.01'
  return tps < 10 ? tps.toFixed(2) : tps.toFixed(1)
}

export function formatBlockTime(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return '—'
  return `${seconds.toFixed(seconds < 10 ? 2 : 1)} s`
}

// ---------------------------------------------------------------------------------------------------------------
// Commit certificates (BFT finality)
// ---------------------------------------------------------------------------------------------------------------

/** CometBFT-style validator address: first 20 bytes of sha256(ed25519 public key). */
export function consensusAddress(consensusKey: string): string {
  if (!/^0x[0-9a-fA-F]{64}$/.test(consensusKey)) return '0x'
  return sha256(consensusKey as Hex).slice(0, 42).toLowerCase()
}

export interface SetMember {
  operator: string
  consensusKey: string
  votingPower: bigint
  name?: string | null
}

export interface CommitVote {
  operator: string | null
  consensusAddress: string
  votingPower: bigint
  signed: boolean
  signature: string | null
  name: string | null
}

export interface CommitView {
  votes: CommitVote[]
  signedPower: bigint
  totalPower: bigint
  /** Signed share in basis points of the total power (0 when unknown). */
  signedBps: number
  /** More than 2/3 of the power signed. `null` when the validator set is unknown. */
  quorum: boolean | null
}

/**
 * Joins a certificate's signatures with the validator set. Signers that are not in the set (an unknown or newer set)
 * are kept with zero power, so nothing silently disappears.
 */
export function commitView(signatures: readonly { validator: string; signature: string }[], set: readonly SetMember[]): CommitView {
  const signedBy = new Map(signatures.map((s) => [s.validator.toLowerCase(), s.signature]))
  const votes: CommitVote[] = set.map((m) => {
    const ca = consensusAddress(m.consensusKey)
    const sig = signedBy.get(ca) ?? signedBy.get(m.operator.toLowerCase()) ?? null
    return { operator: m.operator, consensusAddress: ca, votingPower: m.votingPower, signed: sig !== null, signature: sig, name: m.name ?? null }
  })
  const known = new Set(votes.flatMap((v) => [v.consensusAddress, v.operator?.toLowerCase() ?? '']))
  for (const s of signatures)
    if (!known.has(s.validator.toLowerCase()))
      votes.push({ operator: null, consensusAddress: s.validator.toLowerCase(), votingPower: 0n, signed: true, signature: s.signature, name: null })
  votes.sort((a, b) => (a.votingPower === b.votingPower ? 0 : a.votingPower > b.votingPower ? -1 : 1))
  const totalPower = set.reduce((n, m) => n + m.votingPower, 0n)
  const signedPower = votes.reduce((n, v) => (v.signed ? n + v.votingPower : n), 0n)
  const signedBps = totalPower > 0n ? Number((signedPower * 10000n) / totalPower) : 0
  return { votes, signedPower, totalPower, signedBps, quorum: totalPower > 0n ? signedPower * 3n > totalPower * 2n : null }
}

// Ethereum consensus side: fetch light-client data from a beacon node (REST API) and convert it to the
// calldata of `EthereumLightClient` (uncompressed BLS points, non-signer key proofs, SSZ roots).

import { bls12_381 } from "@noble/curves/bls12-381";
import { createHash } from "node:crypto";
import { bytesToHex, concat, encodeAbiParameters, hexToBytes, keccak256, pad, type Hex } from "viem";

export const SLOTS_PER_PERIOD = 8192;
export const SYNC_COMMITTEE_SIZE = 512;

export interface BeaconHeaderJson {
  slot: string;
  proposer_index: string;
  parent_root: Hex;
  state_root: Hex;
  body_root: Hex;
}

export interface LcHeaderJson {
  beacon: BeaconHeaderJson;
  execution_block_hash: Hex;
  execution_branch: Hex[];
}

export interface LcUpdateJson {
  attested_header: LcHeaderJson;
  finalized_header: LcHeaderJson;
  finality_branch: Hex[];
  next_sync_committee?: { pubkeys: Hex[]; aggregate_pubkey: Hex };
  next_sync_committee_branch?: Hex[];
  sync_aggregate: { sync_committee_bits: Hex; sync_committee_signature: Hex };
  signature_slot: string;
}

/** `BLS12381.G1` of the contract: (xHi, xLo, yHi, yLo) in the EIP-2537 layout. */
export interface G1 {
  xHi: bigint;
  xLo: bigint;
  yHi: bigint;
  yLo: bigint;
}

const MASK256 = (1n << 256n) - 1n;
const split = (v: bigint): [bigint, bigint] => [v >> 256n, v & MASK256];

export function g1FromAffine(x: bigint, y: bigint): G1 {
  const [xHi, xLo] = split(x);
  const [yHi, yLo] = split(y);
  return { xHi, xLo, yHi, yLo };
}

/** Uncompressed G1 key from its 48-byte compressed form. */
export function decompressG1(compressed: Hex): G1 {
  const p = bls12_381.G1.ProjectivePoint.fromHex(compressed.slice(2)).toAffine();
  return g1FromAffine(p.x, p.y);
}

/** Uncompressed G2 signature (x.c0, x.c1, y.c0, y.c1 as hi/lo words) from its 96-byte compressed form. */
export function decompressG2(compressed: Hex): bigint[] {
  const p = bls12_381.G2.ProjectivePoint.fromHex(compressed.slice(2)).toAffine();
  return g2Words(p.x.c0, p.x.c1, p.y.c0, p.y.c1);
}

export function g2Words(...coords: bigint[]): bigint[] {
  return coords.flatMap((c) => split(c));
}

export const sha = (...parts: Hex[]): Hex =>
  bytesToHex(createHash("sha256").update(hexToBytes(concat(parts))).digest());

/** Root of a complete binary tree (power-of-two leaves) with SHA-256 or keccak256. */
export function merkleize(leaves: Hex[], hash: (a: Hex, b: Hex) => Hex): Hex {
  let layer = [...leaves];
  while (layer.length > 1) {
    const next: Hex[] = [];
    for (let i = 0; i < layer.length; i += 2) next.push(hash(layer[i]!, layer[i + 1]!));
    layer = next;
  }
  return layer[0]!;
}

const keccakPair = (a: Hex, b: Hex): Hex => keccak256(concat([a, b]));

/** hash_tree_root(SyncCommittee{pubkeys, aggregate_pubkey}). */
export function syncCommitteeRoot(pubkeys: Hex[], aggregate: Hex): Hex {
  const leaf = (pk: Hex): Hex => sha(pad(pk, { dir: "right", size: 64 }));
  return sha(merkleize(pubkeys.map(leaf), (a, b) => sha(a, b)), leaf(aggregate));
}

/** Leaf of the contract's keccak key tree: keccak256(abi.encode(G1)). */
export function keyLeaf(k: G1): Hex {
  return keccak256(
    encodeAbiParameters(
      [{ type: "uint256" }, { type: "uint256" }, { type: "uint256" }, { type: "uint256" }],
      [k.xHi, k.xLo, k.yHi, k.yLo],
    ),
  );
}

/** Merkle proof (9 siblings) of leaf `index` in the keccak key tree. */
export function keyProof(leaves: Hex[], index: number): Hex[] {
  const proof: Hex[] = [];
  let layer = [...leaves];
  let i = index;
  while (layer.length > 1) {
    proof.push(layer[i ^ 1]!);
    const next: Hex[] = [];
    for (let j = 0; j < layer.length; j += 2) next.push(keccakPair(layer[j]!, layer[j + 1]!));
    layer = next;
    i >>= 1;
  }
  return proof;
}

export function bitSet(bits: Hex, i: number): boolean {
  const b = hexToBytes(bits);
  return ((b[i >> 3]! >> (i & 7)) & 1) === 1;
}

export interface Committee {
  keys: G1[];
  leaves: Hex[];
  root: Hex;
}

export function committeeFromPubkeys(pubkeys: Hex[], aggregate: Hex): Committee {
  const keys = pubkeys.map(decompressG1);
  return { keys, leaves: keys.map(keyLeaf), root: syncCommitteeRoot(pubkeys, aggregate) };
}

const header = (h: BeaconHeaderJson) => ({
  slot: BigInt(h.slot),
  proposerIndex: BigInt(h.proposer_index),
  parentRoot: h.parent_root,
  stateRoot: h.state_root,
  bodyRoot: h.body_root,
});

/** `submitUpdate` arguments for an update signed by `committee` (the committee of the signature period). */
export function toContractUpdate(u: LcUpdateJson, committee: Committee, rotate: boolean) {
  const bits = u.sync_aggregate.sync_committee_bits;
  const bitsWords = [bits.slice(0, 66) as Hex, `0x${bits.slice(66)}` as Hex];
  const nonSigners = [];
  for (let i = 0; i < SYNC_COMMITTEE_SIZE; i++) {
    if (!bitSet(bits, i)) nonSigners.push({ index: BigInt(i), pubkey: committee.keys[i]!, proof: keyProof(committee.leaves, i) });
  }
  const next = rotate && u.next_sync_committee ? syncCommitteeRoot(u.next_sync_committee.pubkeys, u.next_sync_committee.aggregate_pubkey) : null;
  const zero = `0x${"00".repeat(32)}` as Hex;
  const update = {
    attested: header(u.attested_header.beacon),
    finalized: header(u.finalized_header.beacon),
    finalityBranch: u.finality_branch,
    finalizedExecutionBlockHash: u.finalized_header.execution_block_hash,
    executionBranch: u.finalized_header.execution_branch,
    nextSyncCommitteeRoot: next ?? zero,
    nextSyncCommitteeBranch: next ? u.next_sync_committee_branch! : [],
    syncCommitteeBits: bitsWords as [Hex, Hex],
    signature: decompressG2(u.sync_aggregate.sync_committee_signature) as unknown as readonly [bigint, bigint, bigint, bigint, bigint, bigint, bigint, bigint],
    signatureSlot: BigInt(u.signature_slot),
  };
  return { update, nonSigners };
}

// ------------------------------------------------------------------------------------------------------------
// Beacon REST API
// ------------------------------------------------------------------------------------------------------------

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return (await res.json()) as T;
}

export class BeaconClient {
  constructor(readonly baseUrl: string) {}

  async updates(startPeriod: number, count: number): Promise<LcUpdateJson[]> {
    const r = await getJson<{ data: LcUpdateJson }[]>(
      `${this.baseUrl}/eth/v1/beacon/light_client/updates?start_period=${startPeriod}&count=${count}`,
    );
    return r.map((x) => x.data);
  }

  async finalityUpdate(): Promise<LcUpdateJson> {
    return (await getJson<{ data: LcUpdateJson }>(`${this.baseUrl}/eth/v1/beacon/light_client/finality_update`)).data;
  }

  async genesis(): Promise<{ genesis_time: string; genesis_validators_root: Hex }> {
    return (await getJson<{ data: { genesis_time: string; genesis_validators_root: Hex } }>(`${this.baseUrl}/eth/v1/beacon/genesis`)).data;
  }

  async forkSchedule(): Promise<{ epoch: string; current_version: Hex }[]> {
    return (await getJson<{ data: { epoch: string; current_version: Hex }[] }>(`${this.baseUrl}/eth/v1/config/fork_schedule`)).data;
  }
}

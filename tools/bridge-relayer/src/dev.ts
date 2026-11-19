// Dev-mode consensus for local tests ONLY: deterministic Ed25519 "validators" that sign commit certificates over
// any chain's block hashes, and a deterministic BLS sync committee that signs beacon headers. The contracts
// verify these exactly like real certificates / updates; only the keys are local. Never use on a real network.

import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { bls12_381 } from "@noble/curves/bls12-381";
import { bytesToHex, type Hex } from "viem";
import { consensusAddress, precommitSignBytes, sortSet, type Validator } from "./xdevs.ts";
import { g1FromAffine, g2Words, keyLeaf, keyProof as keyProofOf, sha, syncCommitteeRoot, SYNC_COMMITTEE_SIZE, type G1 } from "./beacon.ts";

export interface DevValidator {
  key: Hex;
  sign(message: Uint8Array): Hex; // 64-byte signature
}

export function devValidator(seed: string): DevValidator {
  const sk = createHash("sha256").update(`xdevs-dev-validator:${seed}`).digest();
  const priv = createPrivateKey({
    key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), sk]),
    format: "der",
    type: "pkcs8",
  });
  const pub = createPublicKey(priv).export({ format: "der", type: "spki" }).subarray(12);
  return {
    key: bytesToHex(pub),
    sign: (m) => bytesToHex(sign(null, m, priv)),
  };
}

/** A commit certificate (contract form) signed by `signers` of `set`. */
export function devCommit(
  chainId: bigint,
  height: bigint,
  round: number,
  blockHash: Hex,
  set: Validator[],
  signers: DevValidator[],
) {
  const msg = precommitSignBytes(chainId, height, round, blockHash);
  const idx = new Map(set.map((v, i) => [v.key.toLowerCase(), i]));
  const sigs = signers
    .map((s) => ({ i: idx.get(s.key.toLowerCase())!, sig: s.sign(msg) }))
    .sort((a, b) => a.i - b.i)
    .map(({ i, sig }) => ({ validatorIndex: BigInt(i), r: `0x${sig.slice(2, 66)}` as Hex, s: `0x${sig.slice(66)}` as Hex }));
  return { height, round, blockHash, set, sigs };
}

export function devSet(validators: DevValidator[], powers: bigint[]): Validator[] {
  return sortSet(validators.map((v, i) => ({ key: v.key, power: powers[i]! })));
}

export { consensusAddress };

// ------------------------------------------------------------------------------------------------------------
// Dev sync committee (BLS12-381, Ethereum POP ciphersuite)
// ------------------------------------------------------------------------------------------------------------

const R = bls12_381.params.r;
export const POP_DST = "BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

export interface DevCommittee {
  secrets: bigint[];
  keys: G1[];
  compressed: Hex[];
  aggregate: Hex;
  leaves: Hex[];
}

export function devCommittee(seed: string): DevCommittee {
  const secrets: bigint[] = [];
  for (let i = 0; i < SYNC_COMMITTEE_SIZE; i++) {
    const h = createHash("sha256").update(`xdevs-dev-sync:${seed}:${i}`).digest("hex");
    secrets.push((BigInt(`0x${h}`) % (R - 1n)) + 1n);
  }
  const G = bls12_381.G1.ProjectivePoint.BASE;
  const points = secrets.map((s) => G.multiply(s));
  const keys = points.map((p) => {
    const a = p.toAffine();
    return g1FromAffine(a.x, a.y);
  });
  const compressed = points.map((p) => `0x${p.toHex(true)}` as Hex);
  const agg = points.reduce((a, b) => a.add(b));
  return { secrets, keys, compressed, aggregate: `0x${agg.toHex(true)}` as Hex, leaves: keys.map(keyLeaf) };
}

/** Aggregate signature (uncompressed G2 words) of the members with `bits` set over a 32-byte signing root. */
export function devSign(c: DevCommittee, participants: boolean[], signingRoot: Hex): bigint[] {
  let sk = 0n;
  participants.forEach((p, i) => {
    if (p) sk = (sk + c.secrets[i]!) % R;
  });
  const h = bls12_381.G2.hashToCurve(Buffer.from(signingRoot.slice(2), "hex"), { DST: POP_DST });
  const sig = (h as unknown as { multiply(k: bigint): { toAffine(): { x: { c0: bigint; c1: bigint }; y: { c0: bigint; c1: bigint } } } }).multiply(sk).toAffine();
  return g2Words(sig.x.c0, sig.x.c1, sig.y.c0, sig.y.c1);
}

export { sha };

// ------------------------------------------------------------------------------------------------------------
// Dev beacon chain: synthetic beacon headers whose state / body commit (by real SSZ Merkle branches) to a given
// execution block hash, signed by a dev sync committee. Lets the local e2e drive the real EthereumLightClient.
// ------------------------------------------------------------------------------------------------------------

export const FINALIZED_ROOT_GINDEX = 735;
export const EXECUTION_BLOCK_HASH_GINDEX = 2856;

const ZERO = `0x${"00".repeat(32)}` as Hex;

/** A root and a branch placing `leaf` at `gindex` (siblings: distinct filler nodes). */
export function branchFor(leaf: Hex, gindex: number, salt: string): { root: Hex; branch: Hex[] } {
  const branch: Hex[] = [];
  let node = leaf;
  for (let i = 0; gindex >> (i + 1) > 0; i++) {
    const sib = sha(bytesToHex(new TextEncoder().encode(`${salt}:${i}`)));
    branch.push(sib);
    node = ((gindex >> i) & 1) === 1 ? sha(sib, node) : sha(node, sib);
  }
  return { root: node, branch };
}

function u64Chunk(v: bigint): Hex {
  const b = Buffer.alloc(32);
  b.writeBigUInt64LE(v);
  return bytesToHex(b);
}

export interface DevBeaconHeader {
  slot: bigint;
  proposerIndex: bigint;
  parentRoot: Hex;
  stateRoot: Hex;
  bodyRoot: Hex;
}

export function beaconHeaderRoot(h: DevBeaconHeader): Hex {
  return sha(
    sha(sha(u64Chunk(h.slot), u64Chunk(h.proposerIndex)), sha(h.parentRoot, h.stateRoot)),
    sha(sha(h.bodyRoot, ZERO), sha(ZERO, ZERO)),
  );
}

export function syncDomain(forkVersion: Hex, genesisValidatorsRoot: Hex): Hex {
  const forkDataRoot = sha(`${forkVersion}${"00".repeat(28)}` as Hex, genesisValidatorsRoot);
  return `0x07000000${forkDataRoot.slice(2, 58)}` as Hex;
}

/** A finality update (contract form) proving `executionBlockHash` final at `finalizedSlot`. */
export function devFinalityUpdate(
  c: DevCommittee,
  executionBlockHash: Hex,
  finalizedSlot: bigint,
  forkVersion: Hex,
  genesisValidatorsRoot: Hex,
  absent: number[] = [1, 7, 300],
) {
  const body = branchFor(executionBlockHash, EXECUTION_BLOCK_HASH_GINDEX, `body-${finalizedSlot}`);
  const finalized: DevBeaconHeader = { slot: finalizedSlot, proposerIndex: 1n, parentRoot: ZERO, stateRoot: sha(executionBlockHash), bodyRoot: body.root };
  const state = branchFor(beaconHeaderRoot(finalized), FINALIZED_ROOT_GINDEX, `state-${finalizedSlot}`);
  const attested: DevBeaconHeader = { slot: finalizedSlot + 1n, proposerIndex: 2n, parentRoot: ZERO, stateRoot: state.root, bodyRoot: ZERO };
  const signatureSlot = finalizedSlot + 2n;
  const participants = c.keys.map((_, i) => !absent.includes(i));
  const signingRoot = sha(beaconHeaderRoot(attested), syncDomain(forkVersion, genesisValidatorsRoot));
  const bits = new Uint8Array(64);
  participants.forEach((p, i) => {
    if (p) bits[i >> 3]! |= 1 << (i & 7);
  });
  const bitsHex = bytesToHex(bits);
  const nonSigners = absent.map((i) => ({ index: BigInt(i), pubkey: c.keys[i]!, proof: keyProofOf(c.leaves, i) }));
  return {
    update: {
      attested,
      finalized,
      finalityBranch: state.branch,
      finalizedExecutionBlockHash: executionBlockHash,
      executionBranch: body.branch,
      nextSyncCommitteeRoot: ZERO,
      nextSyncCommitteeBranch: [] as Hex[],
      syncCommitteeBits: [bitsHex.slice(0, 66) as Hex, `0x${bitsHex.slice(66)}` as Hex] as const,
      signature: devSign(c, participants, signingRoot) as unknown as readonly [bigint, bigint, bigint, bigint, bigint, bigint, bigint, bigint],
      signatureSlot,
    },
    nonSigners,
  };
}

export function devCommitteeRoot(c: DevCommittee): Hex {
  return syncCommitteeRoot(c.compressed, c.aggregate);
}

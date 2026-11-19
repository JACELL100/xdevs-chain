// Dev-mode consensus for local tests ONLY: deterministic Ed25519 "validators" that sign commit certificates over
// any chain's block hashes, and a deterministic BLS sync committee that signs beacon headers. The contracts
// verify these exactly like real certificates / updates; only the keys are local. Never use on a real network.

import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { bls12_381 } from "@noble/curves/bls12-381";
import { bytesToHex, type Hex } from "viem";
import { consensusAddress, precommitSignBytes, sortSet, type Validator } from "./xdevs.ts";
import { g1FromAffine, g2Words, keyLeaf, sha, SYNC_COMMITTEE_SIZE, type G1 } from "./beacon.ts";

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

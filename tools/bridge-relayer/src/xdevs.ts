// xDEVS Chain side: commit certificates (`xdevs_getCommitCertificate`) and validator sets, converted to the
// calldata of `XdevsLightClient`.

import { createHash } from "node:crypto";
import { bytesToHex, hexToBytes, type Hex } from "viem";

export const STAKING_ADDRESS = "0x0000000000000000000000000000000000001000" as const;
const WEI_PER_POWER = 10n ** 18n;
const MAX_POWER = (1n << 48n) - 1n; // u64::MAX >> 16

export interface CommitCertificateJson {
  height: Hex | bigint;
  round: Hex | number;
  blockHash: Hex;
  signatures: { validator: Hex; signature: Hex }[];
  raw: Hex;
}

export interface Validator {
  key: Hex;
  power: bigint;
}

/** Consensus address: sha256(key)[..20]. */
export function consensusAddress(key: Hex): Hex {
  return bytesToHex(createHash("sha256").update(hexToBytes(key)).digest().subarray(0, 20));
}

/** `set_from_staking`: Staking powers (wei) to consensus powers, sorted by consensus address. Keys that are not
 *  valid Ed25519 points are dropped by the node and the contract; `isValidKey` decides (default: keep all). */
export function setFromStaking(
  keys: readonly Hex[],
  powersWei: readonly bigint[],
  isValidKey: (k: Hex) => boolean = () => true,
): Validator[] {
  const set: Validator[] = [];
  keys.forEach((key, i) => {
    if (!isValidKey(key)) return;
    let p = powersWei[i]! / WEI_PER_POWER;
    if (p > MAX_POWER) p = MAX_POWER;
    if (p === 0n) p = 1n;
    set.push({ key, power: p });
  });
  return sortSet(set);
}

export function sortSet(set: Validator[]): Validator[] {
  return [...set].sort((a, b) => (consensusAddress(a.key) < consensusAddress(b.key) ? -1 : 1));
}

/** `submitCommit` arguments. With `quorumOnly`, only as many signatures as a > 2/3 quorum needs (gas). */
export function toContractCommit(cert: CommitCertificateJson, set: Validator[], quorumOnly = true) {
  const index = new Map(set.map((v, i) => [consensusAddress(v.key), i]));
  const total = set.reduce((s, v) => s + v.power, 0n);
  const sigs: { validatorIndex: bigint; r: Hex; s: Hex }[] = [];
  let signed = 0n;
  const sorted = cert.signatures
    .map((s) => ({ ...s, idx: index.get(s.validator.toLowerCase() as Hex) }))
    .filter((s): s is typeof s & { idx: number } => s.idx !== undefined)
    .sort((a, b) => a.idx - b.idx);
  // Prefer the highest-power signers when trimming to a quorum.
  const chosen = quorumOnly ? [...sorted].sort((a, b) => Number(set[b.idx]!.power - set[a.idx]!.power)) : sorted;
  const picked: typeof sorted = [];
  for (const s of chosen) {
    if (quorumOnly && signed * 3n > total * 2n) break;
    picked.push(s);
    signed += set[s.idx]!.power;
  }
  if (signed * 3n <= total * 2n) throw new Error(`certificate at ${String(cert.height)} holds no quorum of this set`);
  picked.sort((a, b) => a.idx - b.idx);
  for (const s of picked) {
    sigs.push({ validatorIndex: BigInt(s.idx), r: `0x${s.signature.slice(2, 66)}` as Hex, s: `0x${s.signature.slice(66)}` as Hex });
  }
  return {
    height: BigInt(cert.height),
    round: Number(BigInt(cert.round)),
    blockHash: cert.blockHash,
    set,
    sigs,
  };
}

/** Sign-bytes of a precommit (`xdevs_consensus::vote_sign_bytes`). */
export function precommitSignBytes(chainId: bigint, height: bigint, round: number, blockHash: Hex): Uint8Array {
  const out = new Uint8Array(12 + 8 + 1 + 8 + 4 + 33);
  const dv = new DataView(out.buffer);
  out.set(new TextEncoder().encode("XDEVS/VOTE/1"), 0);
  dv.setBigUint64(12, chainId);
  out[20] = 2;
  dv.setBigUint64(21, height);
  dv.setUint32(29, round);
  out[33] = 1;
  out.set(hexToBytes(blockHash), 34);
  return out;
}

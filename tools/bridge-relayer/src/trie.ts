// Ethereum Merkle-Patricia trie: build a trie from (key, value) pairs and produce inclusion proofs in the
// `eth_getProof` format (RLP nodes from the root to the leaf; nodes < 32 bytes stay embedded in their parent).
// Used to prove receipts against a block's `receiptsRoot` (nodes do not serve receipt proofs over RPC).

import { bytesToHex, concat, hexToBytes, keccak256, toRlp, type Hex } from "viem";

type RlpNode = Hex | RlpNode[];

interface Pair {
  nibbles: number[];
  value: Hex;
}

function toNibbles(key: Uint8Array): number[] {
  const out: number[] = [];
  for (const b of key) out.push(b >> 4, b & 0x0f);
  return out;
}

/** Hex-prefix encoding of a nibble path (Yellow Paper appendix C). */
function hexPrefix(nibbles: number[], leaf: boolean): Hex {
  const odd = nibbles.length % 2 === 1;
  const flag = (leaf ? 2 : 0) + (odd ? 1 : 0);
  const all = odd ? [flag, ...nibbles] : [flag, 0, ...nibbles];
  const bytes = new Uint8Array(all.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = (all[2 * i]! << 4) | all[2 * i + 1]!;
  return bytesToHex(bytes);
}

/** A built trie node with its encoding and children (for proof extraction). */
interface Built {
  node: RlpNode;
  rlp: Hex;
  /** What a parent stores: the node itself if its RLP is < 32 bytes, else its hash. */
  ref: RlpNode;
  /** Child to follow for a key, given the remaining nibbles (returns the child and nibbles consumed). */
  step(rest: number[]): { child: Built; used: number } | undefined;
}

function finish(node: RlpNode, step: Built["step"]): Built {
  const rlp = toRlp(node);
  const ref: RlpNode = (rlp.length - 2) / 2 < 32 ? node : keccak256(rlp);
  return { node, rlp, ref, step };
}

function build(pairs: Pair[], depth: number): Built {
  if (pairs.length === 0) throw new Error("empty subtrie");
  if (pairs.length === 1) {
    const p = pairs[0]!;
    return finish([hexPrefix(p.nibbles.slice(depth), true), p.value], () => undefined);
  }
  // Longest common prefix of all keys below `depth`.
  let cp = 0;
  for (;;) {
    const n = pairs[0]!.nibbles[depth + cp];
    if (n === undefined || !pairs.every((p) => p.nibbles[depth + cp] === n)) break;
    cp++;
  }
  if (cp > 0) {
    const child = build(pairs, depth + cp);
    const prefix = pairs[0]!.nibbles.slice(depth, depth + cp);
    return finish([hexPrefix(prefix, false), child.ref], (rest) =>
      rest.slice(0, cp).every((n, i) => n === prefix[i]) ? { child, used: cp } : undefined,
    );
  }
  const children: (Built | undefined)[] = [];
  let value: Hex = "0x";
  for (let i = 0; i < 16; i++) {
    const group = pairs.filter((p) => p.nibbles[depth] === i);
    children.push(group.length ? build(group, depth + 1) : undefined);
  }
  const terminal = pairs.find((p) => p.nibbles.length === depth);
  if (terminal) value = terminal.value;
  const node: RlpNode = [...children.map((c) => (c ? c.ref : ("0x" as Hex))), value];
  return finish(node, (rest) => {
    const c = rest.length ? children[rest[0]!] : undefined;
    return c ? { child: c, used: 1 } : undefined;
  });
}

export interface Trie {
  root: Hex;
  /** Inclusion proof for `key` (RLP nodes, root first). */
  proof(key: Hex): Hex[];
}

export function buildTrie(entries: { key: Hex; value: Hex }[]): Trie {
  const pairs = entries.map((e) => ({ nibbles: toNibbles(hexToBytes(e.key)), value: e.value }));
  const top = build(pairs, 0);
  return {
    root: keccak256(top.rlp),
    proof(key: Hex): Hex[] {
      const nibbles = toNibbles(hexToBytes(key));
      const out: Hex[] = [top.rlp];
      let cur = top;
      let pos = 0;
      for (;;) {
        const s = cur.step(nibbles.slice(pos));
        if (!s) break;
        pos += s.used;
        cur = s.child;
        if (typeof cur.ref === "string") out.push(cur.rlp); // hashed child: separate proof element
      }
      return out;
    },
  };
}

/** RLP of a transaction index (the receipts-trie key). */
export function indexKey(i: number): Hex {
  return toRlp(i === 0 ? "0x" : (`0x${i.toString(16).padStart(2 * Math.ceil(i.toString(16).length / 2), "0")}` as Hex));
}

export const concatHex = (parts: Hex[]): Hex => concat(parts);

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createPublicKey, verify } from "node:crypto";
import { keccak256, toHex, type Hex } from "viem";
import { buildTrie } from "../src/trie.ts";
import { consensusAddress, precommitSignBytes, setFromStaking } from "../src/xdevs.ts";
import { keyProof, merkleize } from "../src/beacon.ts";
import { branchFor, EXECUTION_BLOCK_HASH_GINDEX, devCommit, devSet, devValidator } from "../src/dev.ts";
import { isEd25519Point } from "../src/sources.ts";

const FIX = path.resolve(import.meta.dirname, "../../../contracts/test/bridge/fixtures");
const real = JSON.parse(fs.readFileSync(path.join(FIX, "xdevs-testnet.json"), "utf8"));

test("trie: Ethereum 'dogs' vector (prefix keys, branch values)", () => {
  const t = buildTrie(
    [["do", "verb"], ["dog", "puppy"], ["doge", "coin"], ["horse", "stallion"]].map(([k, v]) => ({ key: toHex(k!), value: toHex(v!) })),
  );
  assert.equal(t.root, "0x5991bb8c6514148a29db676a14ac506cd2cd5775ace63c30a4fe457715e9ac84");
  const p = t.proof(toHex("doge"));
  assert.equal(keccak256(p[0]!), t.root);
});

test("xdevs: real testnet certificate verifies over our sign-bytes", () => {
  const set = setFromStaking(real.stakingKeys, real.stakingPowersWei.map(BigInt), isEd25519Point);
  assert.deepEqual(set.map((v) => v.key), real.validators.map((v: { key: Hex }) => v.key));
  for (const c of real.commits) {
    const msg = precommitSignBytes(717171n, BigInt(c.height), c.round, c.blockHash);
    for (const s of c.allSigs) {
      const v = set[Number(s.validatorIndex)]!;
      const key = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(v.key.slice(2), "hex")]), format: "der", type: "spki" });
      const sig = Buffer.from(s.r.slice(2) + s.s.slice(2), "hex");
      assert.ok(verify(null, msg, key, sig), `height ${c.height} validator ${consensusAddress(v.key)}`);
    }
  }
});

test("dev: commit certificates pick address-sorted indexes", () => {
  const vs = [0, 1, 2].map((i) => devValidator(`t${i}`));
  const set = devSet(vs, [1n, 1n, 1n]);
  const c = devCommit(7n, 5n, 0, keccak256("0x01"), set, vs);
  assert.deepEqual(c.sigs.map((s) => s.validatorIndex), [0n, 1n, 2n]);
  assert.ok(set.every((v, i) => i === 0 || consensusAddress(set[i - 1]!.key) < consensusAddress(v.key)));
});

test("beacon: key proofs and SSZ branches are consistent", () => {
  const leaves = Array.from({ length: 512 }, (_, i) => keccak256(toHex(i)));
  const root = merkleize(leaves, (a, b) => keccak256(`${a}${b.slice(2)}` as Hex));
  for (const i of [0, 3, 511]) {
    let node = leaves[i]!;
    keyProof(leaves, i).forEach((sib, d) => {
      node = ((i >> d) & 1) === 1 ? keccak256(`${sib}${node.slice(2)}` as Hex) : keccak256(`${node}${sib.slice(2)}` as Hex);
    });
    assert.equal(node, root);
  }
  assert.equal(branchFor(keccak256("0x02"), EXECUTION_BLOCK_HASH_GINDEX, "x").branch.length, 11);
});

test("ed25519 point check drops invalid keys like the node", () => {
  assert.ok(isEd25519Point(real.validators[0].key));
  const dev = JSON.parse(fs.readFileSync(path.join(FIX, "xdevs-dev.json"), "utf8"));
  assert.equal(isEd25519Point(dev.staking.keys[2]), false);
});

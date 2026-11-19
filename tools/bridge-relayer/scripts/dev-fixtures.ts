// Deterministic dev fixtures for the paths real testnet data cannot exercise yet (the Enclave testnet has not
// activated staking-driven validator sets): an epoch-boundary block whose state proves a new validator set in the
// Staking predeploy's storage, commit certificates of the old and the new set, and a conflicting certificate.
// Keys are local dev keys (src/dev.ts); the state, header and proofs are built exactly like a real chain's.
//
// Usage: node scripts/dev-fixtures.ts   (writes contracts/test/bridge/fixtures/xdevs-dev.json)

import fs from "node:fs";
import path from "node:path";
import { encodeAbiParameters, keccak256, pad, toHex, toRlp, type Hex } from "viem";
import { buildTrie } from "../src/trie.ts";
import { quantity, receiptsTrie, type RpcReceipt } from "../src/encoding.ts";
import { indexKey } from "../src/trie.ts";
import { devCommit, devSet, devValidator } from "../src/dev.ts";
import { STAKING_ADDRESS } from "../src/xdevs.ts";

const OUT = path.resolve(import.meta.dirname, "../../../contracts/test/bridge/fixtures/xdevs-dev.json");
const CHAIN_ID = 717171n;
const EPOCH = 10n;
const ZERO32 = pad("0x", { size: 32 });
const EMPTY_TRIE = "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421" as Hex;

/** An Ed25519 encoding that is not a curve point (y with (y²-1)/(dy²+1) a non-square). */
function invalidKey(): Hex {
  const p = 2n ** 255n - 19n;
  const pow = (b: bigint, e: bigint) => {
    let r = 1n;
    b %= p;
    for (; e > 0n; e >>= 1n, b = (b * b) % p) if (e & 1n) r = (r * b) % p;
    return r;
  };
  const d = (((-121665n * pow(121666n, p - 2n)) % p) + p) % p;
  for (let y = 2n; ; y++) {
    const u = (y * y - 1n) % p;
    const v = (d * y * y + 1n) % p;
    if (pow((u * pow(v, p - 2n)) % p, (p - 1n) / 2n) === p - 1n) {
      const le = Buffer.from(y.toString(16).padStart(64, "0"), "hex").reverse();
      return `0x${le.toString("hex")}`;
    }
  }
}

function header(parentHash: Hex, stateRoot: Hex, number: bigint): Hex {
  return toRlp([
    parentHash,
    "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
    "0x0000000000000000000000000000000000000000",
    stateRoot,
    EMPTY_TRIE,
    EMPTY_TRIE,
    pad("0x", { size: 256 }),
    "0x",
    quantity(number),
    quantity(30_000_000n),
    "0x",
    quantity(1_800_000_000n + number * 2n),
    "0x",
    ZERO32,
    "0x0000000000000000",
  ]);
}

const g = [0, 1, 2, 3].map((i) => devValidator(`genesis-${i}`));
const genesisSet = devSet(g, [10n, 10n, 10n, 10n]);

// New Staking set at boundary 10: g1 (20 tXDS), n0 (30 tXDS), n1 (0.5 tXDS -> power 1), and an invalid key.
const n = [0, 1].map((i) => devValidator(`epoch1-${i}`));
const operators = ["0x00000000000000000000000000000000000000a1", "0x00000000000000000000000000000000000000a2", "0x00000000000000000000000000000000000000a3", "0x00000000000000000000000000000000000000a4"] as Hex[];
const keys = [n[0]!.key, g[1]!.key, invalidKey(), n[1]!.key];
const powersWei = [30n * 10n ** 18n, 20n * 10n ** 18n, 50n * 10n ** 18n, 5n * 10n ** 17n];
const newSet = devSet([n[0]!, g[1]!, n[1]!], [30n, 20n, 1n]);
const setHash = keccak256(
  encodeAbiParameters([{ type: "address[]" }, { type: "bytes32[]" }, { type: "uint256[]" }], [operators, keys, powersWei]),
);

// Staking storage: slot 3 = currentEpoch(1) | setVersion(2) << 64 | lastSetChangeBlock(10) << 128 | lastEpochBlock(10) << 192.
const word3 = 1n | (2n << 64n) | (10n << 128n) | (10n << 192n);
const slotKey = (s: bigint) => keccak256(pad(toHex(s), { size: 32 }));
const storage = buildTrie([
  { key: slotKey(3n), value: toRlp(toHex(word3)) },
  { key: slotKey(4n), value: toRlp(setHash) },
  { key: slotKey(7n), value: toRlp("0x01") },
]);
const account = (storageRoot: Hex) => toRlp(["0x01", quantity(10n ** 24n), storageRoot, keccak256("0x6000")]);
const state = buildTrie([
  { key: keccak256(STAKING_ADDRESS), value: account(storage.root) },
  { key: keccak256("0x00000000000000000000000000000000000000ff"), value: account(EMPTY_TRIE) },
  { key: keccak256("0x1111111111111111111111111111111111111111"), value: account(EMPTY_TRIE) },
]);

const h9 = header(ZERO32, state.root, 9n);
const h10 = header(keccak256(h9), state.root, 10n);
const h11 = header(keccak256(h10), state.root, 11n);

const commit10 = devCommit(CHAIN_ID, 10n, 0, keccak256(h10), genesisSet, [g[0]!, g[1]!, g[2]!]);
const conflict10 = devCommit(CHAIN_ID, 10n, 1, keccak256(h9), genesisSet, [g[0]!, g[1]!, g[2]!]);
const commit11 = devCommit(CHAIN_ID, 11n, 0, keccak256(h11), newSet, [n[0]!, g[1]!]);
// The old set cannot certify height 11 any more.
const commit11Old = devCommit(CHAIN_ID, 11n, 0, keccak256(h11), genesisSet, [g[0]!, g[1]!, g[2]!]);

// Bridge event receipts (for the endpoint unit tests): Locked on "Sepolia", BurnedForRelease on "xDEVS".
const VAULT = "0x000000000000000000000000000000000000fa17" as Hex;
const XBRIDGE = "0x000000000000000000000000000000000000b21d" as Hex;
const TOKEN = "0x00000000000000000000000000000000000070c3" as Hex;
const USER = "0x0000000000000000000000000000000000005e4d" as Hex;
const RECIPIENT = "0x000000000000000000000000000000000000be11" as Hex;
const word = (v: bigint | Hex) => pad(typeof v === "bigint" ? toHex(v) : v, { size: 32 });
const LOCKED = keccak256(toHex("Locked(uint256,address,address,address,uint256)"));
const BURNED = keccak256(toHex("BurnedForRelease(uint256,address,address,address,uint256)"));
const data = (to: Hex, amount: bigint) => encodeAbiParameters([{ type: "address" }, { type: "uint256" }], [to, amount]);
const rc = (status: Hex, logs: { address: Hex; topics: Hex[]; data: Hex }[]): RpcReceipt => ({
  type: "0x2", status, cumulativeGasUsed: "0x5208", logsBloom: pad("0x", { size: 256 }), logs,
  transactionIndex: "0x0", transactionHash: ZERO32, blockNumber: "0x1", blockHash: ZERO32,
});
const lockLog = (emitter: Hex, nonce: bigint) => ({ address: emitter, topics: [LOCKED, word(nonce), word(TOKEN), word(USER)], data: data(RECIPIENT, 100n * 10n ** 18n) });
const lockReceipts = [
  rc("0x1", [{ address: TOKEN, topics: [keccak256(toHex("Transfer(address,address,uint256)")), word(USER), word(VAULT)], data: word(1n) }]),
  rc("0x1", [lockLog(VAULT, 0n)]),
  rc("0x1", [lockLog(XBRIDGE, 1n)]), // wrong emitter
  rc("0x0", [lockLog(VAULT, 2n)]), // failed transaction
];
const burnReceipts = [
  rc("0x1", [{ address: XBRIDGE, topics: [BURNED, word(0n), word(TOKEN), word(USER)], data: data(RECIPIENT, 40n * 10n ** 18n) }]),
];
const lockTrie = receiptsTrie(lockReceipts);
const burnTrie = receiptsTrie(burnReceipts);

const j = (v: unknown) => JSON.parse(JSON.stringify(v, (_, x) => (typeof x === "bigint" ? Number(x) : x)));
fs.writeFileSync(
  OUT,
  JSON.stringify(
    {
      note: "LOCAL DEV KEYS - generated by tools/bridge-relayer/scripts/dev-fixtures.ts",
      chainId: Number(CHAIN_ID),
      epochLength: Number(EPOCH),
      stakingStart: 1,
      genesisSet: j(genesisSet),
      newSet: j(newSet),
      headers: { h9, h10, h11 },
      commit10: j(commit10.sigs),
      conflict10: { round: 1, blockHash: keccak256(h9), sigs: j(conflict10.sigs) },
      commit11: j(commit11.sigs),
      commit11Old: j(commit11Old.sigs),
      bridge: {
        vault: VAULT,
        xdevsBridge: XBRIDGE,
        token: TOKEN,
        user: USER,
        recipient: RECIPIENT,
        lockReceiptsRoot: lockTrie.root,
        lockProofs: [0, 1, 2, 3].map((i) => lockTrie.proof(indexKey(i))),
        burnReceiptsRoot: burnTrie.root,
        burnProof: burnTrie.proof(indexKey(0)),
      },
      staking: {
        operators,
        keys,
        powersWei: powersWei.map(String),
        accountProof: state.proof(keccak256(STAKING_ADDRESS)),
        epochWordProof: storage.proof(slotKey(3n)),
        setHashProof: storage.proof(slotKey(4n)),
      },
    },
    null,
    1,
  ) + "\n",
);
console.log(`wrote ${OUT}`);

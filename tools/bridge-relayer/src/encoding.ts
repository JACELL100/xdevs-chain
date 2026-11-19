// RLP encodings of Ethereum-format block headers and receipts from their JSON-RPC form. xDEVS Chain and
// Ethereum use the same formats; the light clients check `keccak256(headerRlp)` against a verified block hash.

import { concat, keccak256, toRlp, type Hex } from "viem";
import { buildTrie, indexKey } from "./trie.ts";

/** JSON-RPC block (header fields only; anything else is ignored). */
export interface RpcBlock {
  hash: Hex;
  parentHash: Hex;
  sha3Uncles: Hex;
  miner: Hex;
  stateRoot: Hex;
  transactionsRoot: Hex;
  receiptsRoot: Hex;
  logsBloom: Hex;
  difficulty: Hex;
  number: Hex;
  gasLimit: Hex;
  gasUsed: Hex;
  timestamp: Hex;
  extraData: Hex;
  mixHash: Hex;
  nonce: Hex;
  baseFeePerGas?: Hex;
  withdrawalsRoot?: Hex;
  blobGasUsed?: Hex;
  excessBlobGas?: Hex;
  parentBeaconBlockRoot?: Hex;
  requestsHash?: Hex;
  blockAccessListHash?: Hex;
  slotNumber?: Hex;
}

/** Minimal big-endian quantity for RLP ("0x0" -> empty string). */
export function quantity(q: Hex | bigint | number): Hex {
  const v = BigInt(q);
  if (v === 0n) return "0x";
  const h = v.toString(16);
  return `0x${h.length % 2 ? "0" : ""}${h}`;
}

// Optional fields in fork order: each fork appends to the header list.
const OPTIONAL: [keyof RpcBlock, boolean][] = [
  ["baseFeePerGas", true], // London
  ["withdrawalsRoot", false], // Shanghai
  ["blobGasUsed", true], // Cancun
  ["excessBlobGas", true],
  ["parentBeaconBlockRoot", false],
  ["requestsHash", false], // Prague
  ["blockAccessListHash", false], // Amsterdam (EIP-7928)
  ["slotNumber", true], // Amsterdam (EIP-7843)
];

export function headerRlp(b: RpcBlock): Hex {
  const fields: Hex[] = [
    b.parentHash,
    b.sha3Uncles,
    b.miner,
    b.stateRoot,
    b.transactionsRoot,
    b.receiptsRoot,
    b.logsBloom,
    quantity(b.difficulty),
    quantity(b.number),
    quantity(b.gasLimit),
    quantity(b.gasUsed),
    quantity(b.timestamp),
    b.extraData,
    b.mixHash,
    b.nonce,
  ];
  for (const [k, isQty] of OPTIONAL) {
    const v = b[k];
    if (v === undefined || v === null) break;
    fields.push(isQty ? quantity(v) : v);
  }
  const rlp = toRlp(fields);
  if (keccak256(rlp) !== b.hash.toLowerCase()) {
    throw new Error(`header RLP of block ${b.number} does not hash to ${b.hash} (unknown header format?)`);
  }
  return rlp;
}

export interface RpcLog {
  address: Hex;
  topics: Hex[];
  data: Hex;
  logIndex?: Hex;
}

export interface RpcReceipt {
  type: Hex;
  status: Hex;
  cumulativeGasUsed: Hex;
  logsBloom: Hex;
  logs: RpcLog[];
  transactionIndex: Hex;
  transactionHash: Hex;
  blockNumber: Hex;
  blockHash: Hex;
}

/** Consensus encoding of a receipt: `type ‖ RLP([status, cumulativeGasUsed, logsBloom, logs])` (legacy: no type). */
export function receiptRlp(r: RpcReceipt): Hex {
  const body = toRlp([
    quantity(r.status),
    quantity(r.cumulativeGasUsed),
    r.logsBloom,
    r.logs.map((l) => [l.address, l.topics, l.data]),
  ]);
  const ty = Number(r.type);
  return ty === 0 ? body : concat([quantity(ty), body]);
}

/** The receipts trie of a block. */
export function receiptsTrie(receipts: RpcReceipt[]) {
  return buildTrie(receipts.map((r, i) => ({ key: indexKey(i), value: receiptRlp(r) })));
}

/** Receipt-trie proof for `txIndex` from all receipts of a block; checks the root against `receiptsRoot`. */
export function receiptProof(receipts: RpcReceipt[], txIndex: number, receiptsRoot: Hex): Hex[] {
  const trie = receiptsTrie(receipts);
  if (trie.root !== receiptsRoot.toLowerCase()) {
    throw new Error(`rebuilt receipts root ${trie.root} != header ${receiptsRoot}`);
  }
  return trie.proof(indexKey(txIndex));
}

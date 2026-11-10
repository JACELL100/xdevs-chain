// The node's genesis format (crates/genesis/src/lib.rs) and the alloc merge used by `build` and `merge`.

import {addrKey, checksumAddress, fail, formatTxds, pad32, parseQuantity, strip0x} from './util.ts';

export const SYSTEM_ADDRESS = '0xfffffffffffffffffffffffffffffffffffffffe';
export const TREASURY = '0x0000000000000000000000000000000000001002';
export const FEE_CONFIG = '0x0000000000000000000000000000000000001003';

/** One alloc entry as the node reads it (`GenesisAccount`). */
export interface Account {
  balance: string; // decimal wei
  code?: string; // runtime bytecode
  storage?: Record<string, string>; // 32-byte slot -> 32-byte value, zero values omitted
  nonce?: string; // 0x-hex, omitted when 0
}

export type Alloc = Record<string, Account>;

/** Normalize one alloc entry (any accepted input spelling) into the canonical output form. */
export function normalizeAccount(raw: any, where: string): Account {
  if (raw === null || typeof raw !== 'object') fail(`${where}: account must be an object`);
  const out: Account = {balance: parseQuantity(raw.balance ?? '0').toString()};
  if (raw.code !== undefined && strip0x(String(raw.code)).length > 0) {
    const code = strip0x(String(raw.code)).toLowerCase();
    if (!/^[0-9a-f]*$/.test(code) || code.length % 2) fail(`${where}: invalid code`);
    out.code = '0x' + code;
  }
  if (raw.storage && Object.keys(raw.storage).length) {
    const entries: [string, string][] = [];
    for (const [k, v] of Object.entries(raw.storage)) {
      const slot = pad32(String(k));
      const val = parseQuantity(String(v));
      if (val !== 0n) entries.push([slot, pad32(val)]);
    }
    entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (entries.length) out.storage = Object.fromEntries(entries);
  }
  const nonce = parseQuantity(raw.nonce ?? '0');
  if (nonce > BigInt(Number.MAX_SAFE_INTEGER)) fail(`${where}: nonce too large`);
  if (nonce !== 0n) out.nonce = '0x' + nonce.toString(16);
  return out;
}

/** `lenient`: accept a wrong mixed-case checksum (existing genesis files; e.g. deploy/configs/dev-genesis.json). */
export function normalizeAlloc(raw: Record<string, any>, where: string, lenient = false): Alloc {
  const out: Alloc = {};
  for (const [a, acc] of Object.entries(raw ?? {})) {
    const addr = checksumAddress(a, lenient);
    if (Object.keys(out).some((k) => addrKey(k) === addrKey(addr))) fail(`${where}: duplicate address ${addr}`);
    out[addr] = normalizeAccount(acc, `${where}[${addr}]`);
  }
  return sortAlloc(out);
}

export function sortAlloc(alloc: Alloc): Alloc {
  return Object.fromEntries(
    Object.entries(alloc).sort(([a], [b]) => (addrKey(a) < addrKey(b) ? -1 : addrKey(a) > addrKey(b) ? 1 : 0)),
  );
}

function isBalanceOnly(a: Account): boolean {
  return !a.code && !a.storage && !a.nonce;
}

/**
 * Merge `extra` into `base`. Any address present on both sides is a collision and an error, with one
 * exception (unless `strict`): funding a predeploy, i.e. one side is a balance-only entry and the other side
 * has a zero balance (e.g. `xdevs-cli testnet init` funds the Treasury at 0x…1002 while the predeploy alloc
 * carries its code and storage with balance 0). Nothing is ever summed or overwritten.
 */
export function mergeAlloc(base: Alloc, extra: Alloc, strict: boolean, notes: string[]): Alloc {
  const out: Alloc = {...base};
  const byKey = new Map(Object.keys(base).map((k) => [addrKey(k), k]));
  for (const [addr, acc] of Object.entries(extra)) {
    const existingKey = byKey.get(addrKey(addr));
    if (existingKey === undefined) {
      out[addr] = acc;
      continue;
    }
    const prev = base[existingKey];
    const fundable =
      (isBalanceOnly(prev) && acc.balance === '0') || (isBalanceOnly(acc) && prev.balance === '0');
    if (strict || !fundable) {
      fail(
        `address collision at ${addr}: both inputs define this account ` +
          `(base: balance ${prev.balance}${prev.code ? ', code' : ''}${prev.storage ? ', storage' : ''}; ` +
          `incoming: balance ${acc.balance}${acc.code ? ', code' : ''}${acc.storage ? ', storage' : ''})`,
      );
    }
    const [funded, contract] = isBalanceOnly(prev) && acc.balance === '0' ? [prev, acc] : [acc, prev];
    delete out[existingKey];
    out[addr] = {...contract, balance: funded.balance};
    notes.push(`${addr}: predeploy funded with ${formatTxds(BigInt(funded.balance))} tXDS from the balance-only entry`);
  }
  return sortAlloc(out);
}

export function totalBalance(alloc: Alloc): bigint {
  return Object.values(alloc).reduce((s, a) => s + BigInt(a.balance), 0n);
}

/** Write an account in the field order of the node's serializer (balance, code, storage, nonce). */
export function orderedAccount(a: Account): Account {
  const o: Account = {balance: a.balance};
  if (a.code) o.code = a.code;
  if (a.storage) o.storage = a.storage;
  if (a.nonce) o.nonce = a.nonce;
  return o;
}

/** FeeConfig slots 1..5 must agree with the genesis header / config (the node reads FeeConfig from block 1). */
export function checkFeeConfig(genesis: any, alloc: Alloc): string[] {
  const fee = Object.entries(alloc).find(([k]) => addrKey(k) === addrKey(FEE_CONFIG))?.[1];
  if (!fee?.storage) return ['FeeConfig not present / not initialized'];
  const slot = (i: number) => BigInt(fee.storage?.[pad32(BigInt(i))] ?? '0x0');
  const bf = genesis.config?.baseFee ?? {};
  const want: [string, bigint, bigint | undefined][] = [
    ['gasLimit vs FeeConfig.blockGasLimit (slot 1)', slot(1), genesis.gasLimit !== undefined ? parseQuantity(genesis.gasLimit) : undefined],
    ['baseFee.minimum vs FeeConfig.minBaseFee (slot 2)', slot(2), bf.minimum !== undefined ? parseQuantity(bf.minimum) : undefined],
    ['baseFee.burnBps vs FeeConfig.burnBps (slot 3)', slot(3), bf.burnBps !== undefined ? parseQuantity(bf.burnBps) : undefined],
    ['baseFee.maxChangeDenominator vs FeeConfig slot 4', slot(4), bf.maxChangeDenominator !== undefined ? parseQuantity(bf.maxChangeDenominator) : undefined],
    ['baseFee.elasticity vs FeeConfig.elasticityMultiplier (slot 5)', slot(5), bf.elasticity !== undefined ? parseQuantity(bf.elasticity) : undefined],
  ];
  const problems: string[] = [];
  for (const [name, have, expected] of want) {
    if (expected !== undefined && have !== expected) problems.push(`${name}: genesis ${expected} != FeeConfig ${have}`);
  }
  if (slot(10) === 0n) problems.push('FeeConfig.version (slot 10) is 0: not initialized');
  return problems;
}

/** Consensus key `ed25519:0x<64 hex>` -> 0x bytes32. */
export function parseConsensusKey(s: string): string {
  const m = /^ed25519:(?:0x)?([0-9a-fA-F]{64})$/.exec(String(s));
  if (!m) fail(`invalid consensusKey ${JSON.stringify(s)} (expected "ed25519:0x<64 hex chars>")`);
  return '0x' + m[1].toLowerCase();
}

export function formatConsensusKey(b32: string): string {
  return 'ed25519:0x' + strip0x(b32).toLowerCase();
}

// Generic check of a running node / anvil against a genesis file: every alloc account's code, balance, nonce and
// every storage slot must read back exactly. Usage: node check-state.ts <genesis.json> <rpc-url> [totalSupplyTxds]

import fs from 'node:fs';

import {parseQuantity, pad32, parseTxds, formatTxds} from './util.ts';

const [file, rpc, supply] = process.argv.slice(2);
if (!file || !rpc) {
  console.error('usage: node check-state.ts <genesis.json> <rpc-url> [totalSupplyTxds]');
  process.exit(2);
}
const g = JSON.parse(fs.readFileSync(file, 'utf8'));

let id = 0;
async function batch(calls: [string, unknown[]][]): Promise<any[]> {
  const out: any[] = [];
  for (let i = 0; i < calls.length; i += 50) {
    const chunk = calls.slice(i, i + 50).map(([method, params]) => ({jsonrpc: '2.0', id: ++id, method, params}));
    const res = await fetch(rpc, {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify(chunk)});
    const j: any[] = await res.json();
    const byId = new Map(j.map((r) => [r.id, r]));
    for (const c of chunk) {
      const r = byId.get(c.id);
      if (!r || r.error) throw new Error(`${c.method}(${JSON.stringify(c.params)}): ${JSON.stringify(r?.error)}`);
      out.push(r.result);
    }
  }
  return out;
}

const calls: [string, unknown[]][] = [];
const expect: [string, (v: any) => boolean][] = [];
let sum = 0n;
for (const [addr, acc] of Object.entries<any>(g.alloc)) {
  const bal = parseQuantity(acc.balance ?? '0');
  sum += bal;
  calls.push(['eth_getBalance', [addr, '0x0']]);
  expect.push([`${addr} balance`, (v) => BigInt(v) === bal]);
  calls.push(['eth_getTransactionCount', [addr, '0x0']]);
  expect.push([`${addr} nonce`, (v) => BigInt(v) === parseQuantity(acc.nonce ?? '0')]);
  calls.push(['eth_getCode', [addr, '0x0']]);
  expect.push([`${addr} code`, (v) => String(v).toLowerCase() === String(acc.code ?? '0x').toLowerCase()]);
  for (const [slot, val] of Object.entries<string>(acc.storage ?? {})) {
    calls.push(['eth_getStorageAt', [addr, slot, '0x0']]);
    expect.push([`${addr} slot ${slot}`, (v) => pad32(String(v)) === pad32(val)]);
  }
}
calls.push(['eth_chainId', []]);
expect.push(['chainId', (v) => BigInt(v) === parseQuantity(g.config.chainId)]);

const res = await batch(calls);
let bad = 0;
res.forEach((v, i) => {
  if (!expect[i][1](v)) {
    bad++;
    console.log(`MISMATCH ${expect[i][0]}: got ${String(v).slice(0, 80)}`);
  }
});
const accounts = Object.keys(g.alloc).length;
const slots = Object.values<any>(g.alloc).reduce((n, a) => n + Object.keys(a.storage ?? {}).length, 0);
console.log(`${bad ? 'FAIL' : 'ok  '} state: ${accounts} accounts, ${slots} storage slots, ${calls.length} reads at block 0`);
if (supply !== undefined) {
  const want = parseTxds(supply);
  console.log(`${sum === want ? 'ok  ' : 'FAIL'} total supply of alloc = ${formatTxds(sum)} tXDS (expected ${formatTxds(want)})`);
  if (sum !== want) bad++;
} else {
  console.log(`info total supply of alloc = ${formatTxds(sum)} tXDS`);
}
process.exitCode = bad ? 1 : 0; // (process.exit right after fetch trips a libuv assertion on Windows)

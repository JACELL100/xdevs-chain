// xDEVS Contract Risk: browser/Node port of xdevs_contract_risk.py (feature version 2).
// Must reproduce the Python featurizer and model exactly; parity.mjs checks it against Python scores in CI.

const OPS = {};
const base = {
  0x00: 'STOP', 0x01: 'ADD', 0x02: 'MUL', 0x03: 'SUB', 0x04: 'DIV', 0x05: 'SDIV', 0x06: 'MOD', 0x07: 'SMOD', 0x08: 'ADDMOD', 0x09: 'MULMOD',
  0x0a: 'EXP', 0x0b: 'SIGNEXTEND', 0x10: 'LT', 0x11: 'GT', 0x12: 'SLT', 0x13: 'SGT', 0x14: 'EQ', 0x15: 'ISZERO', 0x16: 'AND', 0x17: 'OR',
  0x18: 'XOR', 0x19: 'NOT', 0x1a: 'BYTE', 0x1b: 'SHL', 0x1c: 'SHR', 0x1d: 'SAR', 0x1e: 'CLZ', 0x20: 'KECCAK256',
  0x30: 'ADDRESS', 0x31: 'BALANCE', 0x32: 'ORIGIN', 0x33: 'CALLER', 0x34: 'CALLVALUE', 0x35: 'CALLDATALOAD', 0x36: 'CALLDATASIZE',
  0x37: 'CALLDATACOPY', 0x38: 'CODESIZE', 0x39: 'CODECOPY', 0x3a: 'GASPRICE', 0x3b: 'EXTCODESIZE', 0x3c: 'EXTCODECOPY',
  0x3d: 'RETURNDATASIZE', 0x3e: 'RETURNDATACOPY', 0x3f: 'EXTCODEHASH', 0x40: 'BLOCKHASH', 0x41: 'COINBASE', 0x42: 'TIMESTAMP',
  0x43: 'NUMBER', 0x44: 'PREVRANDAO', 0x45: 'GASLIMIT', 0x46: 'CHAINID', 0x47: 'SELFBALANCE', 0x48: 'BASEFEE', 0x49: 'BLOBHASH',
  0x4a: 'BLOBBASEFEE', 0x50: 'POP', 0x51: 'MLOAD', 0x52: 'MSTORE', 0x53: 'MSTORE8', 0x54: 'SLOAD', 0x55: 'SSTORE', 0x56: 'JUMP',
  0x57: 'JUMPI', 0x58: 'PC', 0x59: 'MSIZE', 0x5a: 'GAS', 0x5b: 'JUMPDEST', 0x5c: 'TLOAD', 0x5d: 'TSTORE', 0x5e: 'MCOPY', 0x5f: 'PUSH0',
  0xf0: 'CREATE', 0xf1: 'CALL', 0xf2: 'CALLCODE', 0xf3: 'RETURN', 0xf4: 'DELEGATECALL', 0xf5: 'CREATE2', 0xfa: 'STATICCALL',
  0xfd: 'REVERT', 0xfe: 'INVALID', 0xff: 'SELFDESTRUCT',
};
Object.assign(OPS, base);
for (let i = 0; i < 32; i++) OPS[0x60 + i] = `PUSH${i + 1}`;
for (let i = 0; i < 16; i++) { OPS[0x80 + i] = `DUP${i + 1}`; OPS[0x90 + i] = `SWAP${i + 1}`; }
for (let i = 0; i < 5; i++) OPS[0xa0 + i] = `LOG${i}`;

export const SCALAR_OPS = ['CALL', 'DELEGATECALL', 'STATICCALL', 'CALLCODE', 'SELFDESTRUCT', 'CREATE', 'CREATE2', 'SSTORE', 'SLOAD',
  'ORIGIN', 'CALLER', 'CALLVALUE', 'BALANCE', 'SELFBALANCE', 'GAS', 'TIMESTAMP', 'NUMBER', 'COINBASE', 'BLOCKHASH', 'EXTCODESIZE',
  'EXTCODEHASH', 'EXTCODECOPY', 'LOG0', 'LOG1', 'LOG2', 'LOG3', 'LOG4', 'TSTORE', 'TLOAD', 'JUMPDEST', 'REVERT', 'PUSH20', 'PUSH32',
  'CHAINID', 'PREVRANDAO', 'GASPRICE'];
// Same insertion order as KNOWN_SELECTORS in Python.
export const KNOWN_SELECTORS = [
  ['a9059cbb', 'transfer'], ['23b872dd', 'transferFrom'], ['095ea7b3', 'approve'], ['70a08231', 'balanceOf'], ['dd62ed3e', 'allowance'],
  ['18160ddd', 'totalSupply'], ['42842e0e', 'safeTransferFrom'], ['a22cb465', 'setApprovalForAll'], ['8da5cb5b', 'owner'],
  ['715018a6', 'renounceOwnership'], ['f2fde38b', 'transferOwnership'], ['3ccfd60b', 'withdraw'], ['2e1a7d4d', 'withdraw(uint256)'],
  ['d0e30db0', 'deposit'], ['ac9650d8', 'multicall'], ['5cffe9de', 'flashLoan'], ['ab9c4b5d', 'flashLoan(aave)'], ['490e6cbc', 'flash(uniswapV3)'],
  ['022c0d9f', 'swap(uniswapV2)'], ['128acb08', 'swap(uniswapV3)'], ['38ed1739', 'swapExactTokensForTokens'], ['7ff36ab5', 'swapExactETHForTokens'],
  ['1249c58b', 'mint'], ['40c10f19', 'mint(address,uint256)'], ['42966c68', 'burn'], ['e8e33700', 'addLiquidity'], ['f305d719', 'addLiquidityETH'],
  ['c9567bf9', 'openTrading'], ['8f70ccf7', 'setTrading'], ['3b124fe7', 'setFee?'], ['1cff79cd', 'execute(address,bytes)'],
  ['b61d27f6', 'execute(address,uint256,bytes)'], ['6a761202', 'execTransaction'], ['e9e05c42', 'unknown_claim'], ['4e71d92d', 'claim'],
  ['379607f5', 'claim(uint256)'], ['f4f3b200', 'unknown_airdrop'],
];
const N_OPS = 2 ** 18;
const N_SEL = 2 ** 16;
const CBOR = ['a264697066735822', 'a2646970667358', 'a165627a7a72305820', 'a265627a7a72315820', 'a164736f6c6343'];

export function cleanHex(code) {
  let c = (code || '').trim().toLowerCase();
  if (c.startsWith('0x')) c = c.slice(2);
  if (c.length % 2) c = c.slice(0, -1);
  return c;
}

export function runtimePart(codeHex) {
  const c = cleanHex(codeHex);
  const i = c.slice(0, 4000).indexOf('f3fe'); // Python: first match of f3fe within [0, 4000)
  if (i >= 0 && i % 2 === 0 && i + 4 < c.length - 20) return c.slice(i + 4);
  return c;
}

export function stripMetadata(c) {
  let cut = -1;
  for (const m of CBOR) {
    const i = c.lastIndexOf(m);
    if (i > cut && i % 2 === 0) cut = i;
  }
  return cut > 0 ? c.slice(0, cut) : c;
}

export function disassemble(hex) {
  const n = hex.length / 2;
  const ops = [];
  const sels = [];
  let i = 0;
  while (i < n) {
    const op = parseInt(hex.substr(i * 2, 2), 16);
    ops.push(OPS[op] ?? 'UNK');
    if (op >= 0x60 && op <= 0x7f) {
      const k = op - 0x5f;
      if (k === 4 && i + 5 <= n) sels.push(hex.substr((i + 1) * 2, 8));
      i += 1 + k;
    } else i += 1;
  }
  return { ops, sels, nbytes: n };
}

// MurmurHash3 x86 32-bit, seed 0, signed result (sklearn murmurhash3_32(positive=False)).
function murmur3(str) {
  let h = 0;
  const len = str.length; // tokens are ASCII, so UTF-8 bytes == char codes
  const nblocks = len >> 2;
  const c1 = 0xcc9e2d51, c2 = 0x1b873593;
  for (let b = 0; b < nblocks; b++) {
    const j = b * 4;
    let k = (str.charCodeAt(j)) | (str.charCodeAt(j + 1) << 8) | (str.charCodeAt(j + 2) << 16) | (str.charCodeAt(j + 3) << 24);
    k = Math.imul(k, c1); k = (k << 15) | (k >>> 17); k = Math.imul(k, c2);
    h ^= k; h = (h << 13) | (h >>> 19); h = (Math.imul(h, 5) + 0xe6546b64) | 0;
  }
  let k = 0;
  const t = nblocks * 4;
  switch (len & 3) {
    case 3: k ^= str.charCodeAt(t + 2) << 16; // fallthrough
    case 2: k ^= str.charCodeAt(t + 1) << 8; // fallthrough
    case 1: k ^= str.charCodeAt(t); k = Math.imul(k, c1); k = (k << 15) | (k >>> 17); k = Math.imul(k, c2); h ^= k;
  }
  h ^= len;
  h ^= h >>> 16; h = Math.imul(h, 0x85ebca6b); h ^= h >>> 13; h = Math.imul(h, 0xc2b2ae35); h ^= h >>> 16;
  return h | 0;
}
const hidx = (tok, n) => Math.abs(murmur3(tok)) % n;
const f32 = Math.fround;

/** Sparse feature vector: Map(column -> float32 value), same column layout as the Python hstack. */
export function featurize(code) {
  const h = stripMetadata(runtimePart(code));
  const { ops, sels, nbytes } = disassemble(h);
  const cnt = new Map();
  for (const o of ops) cnt.set(o, (cnt.get(o) || 0) + 1);
  const nops = Math.max(1, ops.length);
  const selSet = new Set(sels);
  const known = KNOWN_SELECTORS.filter(([k]) => selSet.has(k)).map(([, v]) => v);
  const erc20 = ['a9059cbb', '095ea7b3', '70a08231'].every((s) => selSet.has(s)) ? 1 : 0;
  const scal = [Math.log1p(nbytes), Math.log1p(ops.length), selSet.size, known.length, erc20, (cnt.get('UNK') || 0) / nops];
  for (const o of SCALAR_OPS) scal.push(cnt.get(o) || 0);
  for (const o of SCALAR_OPS) scal.push((cnt.get(o) || 0) / nops);
  for (const [, v] of KNOWN_SELECTORS) scal.push(known.includes(v) ? 1 : 0);
  const x = new Map();
  scal.forEach((v, i) => { if (v !== 0) x.set(i, f32(v)); });
  const nS = scal.length;
  // opcode 1-2-grams, PUSH1..PUSH32 collapsed to PUSH, counts then L1-normalised
  const toks = ops.map((o) => (o.startsWith('PUSH') && o !== 'PUSH0' ? 'PUSH' : o));
  const oc = new Map();
  let total = 0;
  const add = (t) => { const j = hidx(t, N_OPS); oc.set(j, (oc.get(j) || 0) + 1); total += 1; };
  for (const t of toks) add(t);
  for (let i = 0; i + 1 < toks.length; i++) add(`${toks[i]} ${toks[i + 1]}`);
  for (const [j, v] of oc) x.set(nS + j, f32(v / total));
  // selector presence, binary
  for (const s of selSet) x.set(nS + N_OPS + hidx(`s${s}`, N_SEL), 1);
  return x;
}

export function explain(code) {
  const { ops, sels, nbytes } = disassemble(stripMetadata(runtimePart(code)));
  const cnt = {};
  for (const o of ops) cnt[o] = (cnt[o] || 0) + 1;
  const selSet = new Set(sels);
  const notable = {};
  for (const o of ['DELEGATECALL', 'SELFDESTRUCT', 'CALL', 'CREATE2', 'ORIGIN', 'PUSH20']) if (cnt[o]) notable[o] = cnt[o];
  return {
    bytes: nbytes,
    known_functions: KNOWN_SELECTORS.filter(([k]) => selSet.has(k)).map(([, v]) => v),
    notable_opcodes: notable,
    erc20_like: ['a9059cbb', '095ea7b3', '70a08231'].every((s) => selSet.has(s)),
  };
}

/** model = { trees, coef: Float32Array, intercept, config } (see export.py). */
export function score(model, code) {
  if (cleanHex(code).length < 4) return 0;
  const x = featurize(code);
  let raw = 0;
  for (const t of model.trees) {
    // t = { f, th, dl, mt, l, r, v }: node arrays; child < 0 means leaf ~child
    let n = 0;
    for (;;) {
      const fv = x.get(t.f[n]) ?? 0;
      let left;
      if (t.mt[n] === 1 && Math.abs(fv) <= 1e-35) left = t.dl[n] === 1; // missing type Zero
      else left = fv <= t.th[n];
      const c = left ? t.l[n] : t.r[n];
      if (c < 0) { raw += t.v[~c]; break; }
      n = c;
    }
  }
  let p = 1 / (1 + Math.exp(-raw));
  p = Math.min(Math.max(p, 1e-7), 1 - 1e-7);
  const w = model.config.blend_weight_lightgbm ?? 1;
  if (w >= 1 || !model.coef) return p;
  let zl = model.intercept;
  for (const [j, v] of x) zl += Math.log1p(Math.abs(v)) * model.coef[j];
  const z = w * Math.log(p / (1 - p)) + (1 - w) * zl;
  return 1 / (1 + Math.exp(-z));
}

export function predict(model, code) {
  const p = score(model, code);
  const c = model.config;
  const level = p >= c.threshold ? 'high' : p >= (c.medium_threshold ?? c.threshold / 3) ? 'medium' : 'low';
  return { risk_score: p, risk_level: level, threshold: c.threshold, signals: explain(code) };
}

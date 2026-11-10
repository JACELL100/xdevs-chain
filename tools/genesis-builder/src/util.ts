// Small dependency-free helpers: keccak-256, EIP-55 checksums, unit parsing, canonical JSON.

const MASK = (1n << 64n) - 1n;
const RC: bigint[] = [
  0x0000000000000001n, 0x0000000000008082n, 0x800000000000808an, 0x8000000080008000n, 0x000000000000808bn,
  0x0000000080000001n, 0x8000000080008081n, 0x8000000000008009n, 0x000000000000008an, 0x0000000000000088n,
  0x0000000080008009n, 0x000000008000000an, 0x000000008000808bn, 0x800000000000008bn, 0x8000000000008089n,
  0x8000000000008003n, 0x8000000000008002n, 0x8000000000000080n, 0x000000000000800an, 0x800000008000000an,
  0x8000000080008081n, 0x8000000000008080n, 0x0000000080000001n, 0x8000000080008008n,
];
// ROT[x][y]
const ROT = [
  [0, 36, 3, 41, 18],
  [1, 44, 10, 45, 2],
  [62, 6, 43, 15, 61],
  [28, 55, 25, 21, 56],
  [27, 20, 39, 8, 14],
].map((r) => r.map(BigInt));

function rotl(v: bigint, n: bigint): bigint {
  return n === 0n ? v : ((v << n) | (v >> (64n - n))) & MASK;
}

function keccakF(a: bigint[]): void {
  const c = new Array<bigint>(5);
  const b = new Array<bigint>(25);
  for (let round = 0; round < 24; round++) {
    for (let x = 0; x < 5; x++) c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
    for (let x = 0; x < 5; x++) {
      const d = c[(x + 4) % 5] ^ rotl(c[(x + 1) % 5], 1n);
      for (let y = 0; y < 25; y += 5) a[x + y] ^= d;
    }
    for (let x = 0; x < 5; x++) {
      for (let y = 0; y < 5; y++) b[y + 5 * ((2 * x + 3 * y) % 5)] = rotl(a[x + 5 * y], ROT[x][y]);
    }
    for (let y = 0; y < 25; y += 5) {
      for (let x = 0; x < 5; x++) a[x + y] = b[x + y] ^ (~b[((x + 1) % 5) + y] & MASK & b[((x + 2) % 5) + y]);
    }
    a[0] ^= RC[round];
  }
}

/** keccak-256 (Ethereum's, not SHA3-256). */
export function keccak256(data: Uint8Array): Uint8Array {
  const rate = 136;
  const padded = new Uint8Array(Math.floor(data.length / rate) * rate + rate);
  padded.set(data);
  padded[data.length] ^= 0x01;
  padded[padded.length - 1] ^= 0x80;
  const s = new Array<bigint>(25).fill(0n);
  for (let off = 0; off < padded.length; off += rate) {
    for (let i = 0; i < rate / 8; i++) {
      let lane = 0n;
      for (let k = 7; k >= 0; k--) lane = (lane << 8n) | BigInt(padded[off + i * 8 + k]);
      s[i] ^= lane;
    }
    keccakF(s);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 4; i++) {
    let lane = s[i];
    for (let k = 0; k < 8; k++) {
      out[i * 8 + k] = Number(lane & 0xffn);
      lane >>= 8n;
    }
  }
  return out;
}

export function hexToBytes(hex: string): Uint8Array {
  const h = strip0x(hex);
  if (h.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(h)) throw new Error(`invalid hex: ${hex.slice(0, 20)}…`);
  return Uint8Array.from(Buffer.from(h, 'hex'));
}

export function bytesToHex(b: Uint8Array): string {
  return '0x' + Buffer.from(b).toString('hex');
}

export function keccakHex(hex: string): string {
  return bytesToHex(keccak256(hexToBytes(hex)));
}

export function strip0x(s: string): string {
  return s.startsWith('0x') || s.startsWith('0X') ? s.slice(2) : s;
}

/** EIP-55 checksum; throws on malformed input or on a wrong mixed-case checksum. */
export function checksumAddress(addr: string, lenient = false): string {
  const h = strip0x(addr);
  if (!/^[0-9a-fA-F]{40}$/.test(h)) throw new Error(`invalid address: ${addr}`);
  const lower = h.toLowerCase();
  const hash = Buffer.from(keccak256(Buffer.from(lower, 'ascii'))).toString('hex');
  let out = '';
  for (let i = 0; i < 40; i++) out += parseInt(hash[i], 16) >= 8 ? lower[i].toUpperCase() : lower[i];
  const mixed = h !== lower && h !== h.toUpperCase();
  if (mixed && h !== out && !lenient) throw new Error(`bad EIP-55 checksum: ${addr}`);
  return '0x' + out;
}

export const WEI_PER_TXDS = 10n ** 18n;

/** Parse an amount in tXDS ("12500000", "0.5") into wei. */
export function parseTxds(v: string | number): bigint {
  const s = String(v).trim();
  const m = /^(\d+)(?:\.(\d{1,18}))?$/.exec(s);
  if (!m) throw new Error(`invalid tXDS amount: ${v}`);
  return BigInt(m[1]) * WEI_PER_TXDS + BigInt((m[2] ?? '').padEnd(18, '0') || '0');
}

/** Parse a wei quantity (decimal string, 0x-hex string or JSON number). */
export function parseQuantity(v: unknown): bigint {
  if (typeof v === 'number') {
    if (!Number.isSafeInteger(v) || v < 0) throw new Error(`unsafe numeric quantity ${v}: use a string`);
    return BigInt(v);
  }
  if (typeof v !== 'string') throw new Error(`invalid quantity: ${JSON.stringify(v)}`);
  const s = v.trim();
  if (/^0[xX][0-9a-fA-F]+$/.test(s)) return BigInt('0x' + s.slice(2));
  if (/^\d+$/.test(s)) return BigInt(s);
  throw new Error(`invalid quantity: ${v}`);
}

export function formatTxds(wei: bigint): string {
  const whole = wei / WEI_PER_TXDS;
  const frac = wei % WEI_PER_TXDS;
  const w = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return frac === 0n ? w : `${w}.${frac.toString().padStart(18, '0').replace(/0+$/, '')}`;
}

export function pad32(v: bigint | string): string {
  const b = typeof v === 'bigint' ? v : parseQuantity(v);
  return '0x' + b.toString(16).padStart(64, '0');
}

export function addrKey(a: string): string {
  return strip0x(a).toLowerCase();
}

/** Pretty JSON with a trailing newline (stable: callers insert keys in a deterministic order). */
export function toJson(v: unknown): string {
  return JSON.stringify(v, null, 2) + '\n';
}

export function fail(msg: string): never {
  throw new Error(msg);
}

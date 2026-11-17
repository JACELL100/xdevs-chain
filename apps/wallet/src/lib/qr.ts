// Minimal QR Code encoder (ISO/IEC 18004): byte mode, error-correction level M, versions 1-10, automatic mask.
// Enough for addresses and short URIs (version 10-M holds 213 bytes); no dependency. Follows the structure of
// Project Nayuki's reference implementation (MIT). Tested against the standard's Reed-Solomon, format and version
// information vectors in test/qr.test.ts.

/** Error-correction codewords per block, level M, index = version (1-10). */
const ECC_PER_BLOCK_M = [-1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26]
/** Number of error-correction blocks, level M, index = version (1-10). */
const BLOCKS_M = [-1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5]
const FORMAT_BITS_M = 0 // L=1, M=0, Q=3, H=2
export const QR_MAX_VERSION = 10

export type QrMatrix = { size: number; version: number; mask: number; modules: boolean[][] }

function rawDataModules(ver: number): number {
  let r = (16 * ver + 128) * ver + 64
  if (ver >= 2) {
    const align = Math.floor(ver / 7) + 2
    r -= (25 * align - 10) * align - 55
    if (ver >= 7) r -= 36
  }
  return r
}

export function dataCodewords(ver: number): number {
  return Math.floor(rawDataModules(ver) / 8) - ECC_PER_BLOCK_M[ver] * BLOCKS_M[ver]
}

function gfMul(x: number, y: number): number {
  let z = 0
  for (let i = 7; i >= 0; i--) {
    z = (z << 1) ^ ((z >>> 7) * 0x11d)
    z ^= ((y >>> i) & 1) * x
  }
  return z & 0xff
}

export function rsDivisor(degree: number): number[] {
  const result = new Array<number>(degree).fill(0)
  result[degree - 1] = 1
  let root = 1
  for (let i = 0; i < degree; i++) {
    for (let j = 0; j < result.length; j++) {
      result[j] = gfMul(result[j], root)
      if (j + 1 < result.length) result[j] ^= result[j + 1]
    }
    root = gfMul(root, 0x02)
  }
  return result
}

export function rsRemainder(data: readonly number[], divisor: readonly number[]): number[] {
  const result = new Array<number>(divisor.length).fill(0)
  for (const b of data) {
    const factor = b ^ (result.shift() as number)
    result.push(0)
    divisor.forEach((coef, i) => {
      result[i] ^= gfMul(coef, factor)
    })
  }
  return result
}

/** 15-bit format information (level M) for a mask, BCH-encoded and XOR-masked. */
export function formatBits(mask: number): number {
  const data = (FORMAT_BITS_M << 3) | mask
  let rem = data
  for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537)
  return ((data << 10) | rem) ^ 0x5412
}

/** 18-bit version information (versions >= 7). */
export function versionBits(ver: number): number {
  let rem = ver
  for (let i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >>> 11) * 0x1f25)
  return (ver << 12) | rem
}

const bit = (x: number, i: number) => ((x >>> i) & 1) !== 0

function alignmentPositions(ver: number, size: number): number[] {
  if (ver === 1) return []
  const n = Math.floor(ver / 7) + 2
  const step = Math.ceil((ver * 4 + 4) / (n * 2 - 2)) * 2
  const result = [6]
  for (let pos = size - 7; result.length < n; pos -= step) result.splice(1, 0, pos)
  return result
}

function interleave(ver: number, data: number[]): number[] {
  const numBlocks = BLOCKS_M[ver]
  const eccLen = ECC_PER_BLOCK_M[ver]
  const raw = Math.floor(rawDataModules(ver) / 8)
  const numShort = numBlocks - (raw % numBlocks)
  const shortLen = Math.floor(raw / numBlocks)
  const div = rsDivisor(eccLen)
  const blocks: number[][] = []
  for (let i = 0, k = 0; i < numBlocks; i++) {
    const dat = data.slice(k, k + shortLen - eccLen + (i < numShort ? 0 : 1))
    k += dat.length
    const ecc = rsRemainder(dat, div)
    if (i < numShort) dat.push(0)
    blocks.push(dat.concat(ecc))
  }
  const out: number[] = []
  for (let i = 0; i < blocks[0].length; i++) {
    blocks.forEach((b, j) => {
      if (i !== shortLen - eccLen || j >= numShort) out.push(b[i])
    })
  }
  return out
}

function maskBit(mask: number, x: number, y: number): boolean {
  switch (mask) {
    case 0:
      return (x + y) % 2 === 0
    case 1:
      return y % 2 === 0
    case 2:
      return x % 3 === 0
    case 3:
      return (x + y) % 3 === 0
    case 4:
      return (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0
    case 5:
      return ((x * y) % 2) + ((x * y) % 3) === 0
    case 6:
      return (((x * y) % 2) + ((x * y) % 3)) % 2 === 0
    default:
      return (((x + y) % 2) + ((x * y) % 3)) % 2 === 0
  }
}

/** Penalty score (rules 1-4 of the standard, rule 3 simplified to the 1:1:3:1:1 run with 4 light modules). */
function penalty(m: boolean[][]): number {
  const size = m.length
  let score = 0
  const line = (get: (i: number, j: number) => boolean) => {
    for (let i = 0; i < size; i++) {
      let run = 1
      for (let j = 1; j <= size; j++) {
        if (j < size && get(i, j) === get(i, j - 1)) run++
        else {
          if (run >= 5) score += run - 2
          run = 1
        }
      }
      // Finder-like 1:1:3:1:1 run with 4 light modules on either side (outside the symbol counts as light).
      for (let j = -4; j < size; j++) {
        const s = Array.from({ length: 11 }, (_, k) => (j + k >= 0 && j + k < size ? get(i, j + k) : false))
        const core = s[4] && !s[5] && s[6] && s[7] && s[8] && !s[9] && s[10]
        const coreL = s[0] && !s[1] && s[2] && s[3] && s[4] && !s[5] && s[6]
        if (core && !s[0] && !s[1] && !s[2] && !s[3]) score += 40
        if (coreL && !s[7] && !s[8] && !s[9] && !s[10]) score += 40
      }
    }
  }
  line((i, j) => m[i][j])
  line((i, j) => m[j][i])
  for (let y = 0; y < size - 1; y++)
    for (let x = 0; x < size - 1; x++) {
      const c = m[y][x]
      if (c === m[y][x + 1] && c === m[y + 1][x] && c === m[y + 1][x + 1]) score += 3
    }
  let dark = 0
  for (const row of m) for (const c of row) if (c) dark++
  const total = size * size
  score += (Math.ceil(Math.abs(dark * 20 - total * 10) / total) - 1) * 10
  return score
}

/** Encodes `text` (UTF-8) into a QR matrix (`modules[y][x]`, true = dark). Throws if it needs more than version 10-M. */
export function encodeQr(text: string, forceMask?: number): QrMatrix {
  const bytes = Array.from(new TextEncoder().encode(text))
  let ver = 1
  for (; ver <= QR_MAX_VERSION; ver++) {
    const countBits = ver <= 9 ? 8 : 16
    if (4 + countBits + bytes.length * 8 <= dataCodewords(ver) * 8) break
  }
  if (ver > QR_MAX_VERSION) throw new Error('text too long for a version 1-10 QR code')
  const countBits = ver <= 9 ? 8 : 16
  const capacity = dataCodewords(ver) * 8
  const bits: number[] = []
  const push = (v: number, n: number) => {
    for (let i = n - 1; i >= 0; i--) bits.push((v >>> i) & 1)
  }
  push(0b0100, 4)
  push(bytes.length, countBits)
  for (const b of bytes) push(b, 8)
  push(0, Math.min(4, capacity - bits.length))
  push(0, (8 - (bits.length % 8)) % 8)
  for (let pad = 0xec; bits.length < capacity; pad ^= 0xec ^ 0x11) push(pad, 8)
  const data: number[] = []
  for (let i = 0; i < bits.length; i += 8) data.push(bits.slice(i, i + 8).reduce((a, b) => (a << 1) | b, 0))

  const size = ver * 4 + 17
  const modules = Array.from({ length: size }, () => new Array<boolean>(size).fill(false))
  const isFn = Array.from({ length: size }, () => new Array<boolean>(size).fill(false))
  const set = (x: number, y: number, dark: boolean) => {
    modules[y][x] = dark
    isFn[y][x] = true
  }
  for (let i = 0; i < size; i++) {
    set(6, i, i % 2 === 0)
    set(i, 6, i % 2 === 0)
  }
  for (const [cx, cy] of [
    [3, 3],
    [size - 4, 3],
    [3, size - 4],
  ]) {
    for (let dy = -4; dy <= 4; dy++)
      for (let dx = -4; dx <= 4; dx++) {
        const d = Math.max(Math.abs(dx), Math.abs(dy))
        const x = cx + dx
        const y = cy + dy
        if (x >= 0 && x < size && y >= 0 && y < size) set(x, y, d !== 2 && d !== 4)
      }
  }
  const al = alignmentPositions(ver, size)
  for (let i = 0; i < al.length; i++)
    for (let j = 0; j < al.length; j++) {
      if ((i === 0 && j === 0) || (i === 0 && j === al.length - 1) || (i === al.length - 1 && j === 0)) continue
      for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) set(al[i] + dx, al[j] + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1)
    }
  const drawFormat = (mask: number) => {
    const f = formatBits(mask)
    for (let i = 0; i <= 5; i++) set(8, i, bit(f, i))
    set(8, 7, bit(f, 6))
    set(8, 8, bit(f, 7))
    set(7, 8, bit(f, 8))
    for (let i = 9; i < 15; i++) set(14 - i, 8, bit(f, i))
    for (let i = 0; i < 8; i++) set(size - 1 - i, 8, bit(f, i))
    for (let i = 8; i < 15; i++) set(8, size - 15 + i, bit(f, i))
    set(8, size - 8, true)
  }
  drawFormat(0)
  if (ver >= 7) {
    const v = versionBits(ver)
    for (let i = 0; i < 18; i++) {
      const a = size - 11 + (i % 3)
      const b = Math.floor(i / 3)
      set(a, b, bit(v, i))
      set(b, a, bit(v, i))
    }
  }

  const codewords = interleave(ver, data)
  let i = 0
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5
    for (let vert = 0; vert < size; vert++)
      for (let j = 0; j < 2; j++) {
        const x = right - j
        const upward = ((right + 1) & 2) === 0
        const y = upward ? size - 1 - vert : vert
        if (!isFn[y][x] && i < codewords.length * 8) {
          modules[y][x] = bit(codewords[i >>> 3], 7 - (i & 7))
          i++
        }
      }
  }

  const applyMask = (mask: number) => {
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (!isFn[y][x] && maskBit(mask, x, y)) modules[y][x] = !modules[y][x]
  }
  let best = forceMask ?? 0
  if (forceMask === undefined) {
    let min = Number.POSITIVE_INFINITY
    for (let mask = 0; mask < 8; mask++) {
      applyMask(mask)
      drawFormat(mask)
      const p = penalty(modules)
      if (p < min) {
        min = p
        best = mask
      }
      applyMask(mask)
    }
  }
  applyMask(best)
  drawFormat(best)
  return { size, version: ver, mask: best, modules }
}

/** One SVG path (`d`) drawing every dark module as a 1x1 square, offset by a quiet zone of `margin` modules. */
export function qrPath(m: QrMatrix, margin = 4): string {
  let d = ''
  for (let y = 0; y < m.size; y++)
    for (let x = 0; x < m.size; x++) if (m.modules[y][x]) d += `M${x + margin} ${y + margin}h1v1h-1z`
  return d
}

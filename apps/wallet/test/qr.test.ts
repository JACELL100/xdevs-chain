// QR encoder: standard vectors (Reed-Solomon, format and version information) and symbol structure.
import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { dataCodewords, encodeQr, formatBits, qrPath, rsDivisor, rsRemainder, versionBits } from '../src/lib/qr.ts'

describe('QR encoder', () => {
  it('Reed-Solomon: the ISO 18004 "HELLO WORLD" 1-M example', () => {
    const data = [32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17]
    assert.deepEqual(rsRemainder(data, rsDivisor(10)), [196, 35, 39, 119, 235, 215, 231, 226, 93, 23])
  })

  it('format and version information match the standard tables', () => {
    assert.equal(formatBits(0), 0b101010000010010)
    assert.equal(formatBits(1), 0b101000100100101)
    assert.equal(versionBits(7), 0b000111110010010100)
    assert.equal(versionBits(10), 0b001010010011010011)
  })

  it('data capacity per version (level M)', () => {
    assert.deepEqual([1, 2, 3, 4, 5, 10].map(dataCodewords), [16, 28, 44, 64, 86, 216])
  })

  it('an address fits version 3; structure is valid for every mask', () => {
    const addr = '0x7a3F9c21D4e5B6a7C8d9E0f1A2b3C4d5E6f70D1e'
    for (let mask = 0; mask < 8; mask++) {
      const q = encodeQr(addr, mask)
      assert.equal(q.version, 3)
      assert.equal(q.size, 29)
      const m = q.modules
      // finder patterns: dark 7x7 ring, light ring, dark 3x3 core
      for (const [ox, oy] of [
        [0, 0],
        [q.size - 7, 0],
        [0, q.size - 7],
      ]) {
        for (let i = 0; i < 7; i++) {
          assert.ok(m[oy][ox + i] && m[oy + 6][ox + i] && m[oy + i][ox] && m[oy + i][ox + 6])
          if (i >= 1 && i <= 5) assert.ok(!m[oy + 1][ox + i] && !m[oy + 5][ox + i])
        }
        assert.ok(m[oy + 3][ox + 3])
      }
      // timing patterns and the dark module
      for (let i = 8; i < q.size - 8; i++) {
        assert.equal(m[6][i], i % 2 === 0)
        assert.equal(m[i][6], i % 2 === 0)
      }
      assert.ok(m[q.size - 8][8])
      // both copies of the format information carry the same 15 bits
      const f = formatBits(mask)
      const a: boolean[] = []
      const b: boolean[] = []
      for (let i = 0; i <= 5; i++) a.push(m[i][8])
      a.push(m[7][8], m[8][8], m[8][7])
      for (let i = 9; i < 15; i++) a.push(m[8][14 - i])
      for (let i = 0; i < 8; i++) b.push(m[8][q.size - 1 - i])
      for (let i = 8; i < 15; i++) b.push(m[q.size - 15 + i][8])
      a.forEach((v, i) => assert.equal(v, ((f >>> i) & 1) === 1, `format bit ${i} (copy 1, mask ${mask})`))
      b.forEach((v, i) => assert.equal(v, ((f >>> i) & 1) === 1, `format bit ${i} (copy 2, mask ${mask})`))
    }
  })

  it('longer text uses bigger versions; too long throws; path has a quiet zone', () => {
    assert.equal(encodeQr('ethereum:0x7a3F9c21D4e5B6a7C8d9E0f1A2b3C4d5E6f70D1e@717171?value=1e18').version, 5)
    assert.equal(encodeQr('x'.repeat(200)).version, 10)
    assert.throws(() => encodeQr('x'.repeat(300)))
    assert.match(qrPath(encodeQr('a')), /^M4 4h1v1h-1z/)
  })
})

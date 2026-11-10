import { test } from 'node:test'
import assert from 'node:assert/strict'
import { classifyTransfer, findLookAlike, isDust, isPoisonCandidate, NATIVE_SYMBOLS } from '../src/spam.ts'

const symbols = new Set([...NATIVE_SYMBOLS, 'USDX'])
const base = { protectedSymbols: symbols, dustNativeWei: 10n ** 15n, lookAlikeOf: null }

test('dust thresholds: native by wei, tokens by 10^(decimals-3)', () => {
  assert.equal(isDust('native', 0n, null, 10n ** 15n), false, 'zero is not dust, it is zero-value')
  assert.equal(isDust('native', 10n ** 15n - 1n, null, 10n ** 15n), true)
  assert.equal(isDust('native', 10n ** 15n, null, 10n ** 15n), false)
  assert.equal(isDust('erc20', 999n, 6, 0n), true)
  assert.equal(isDust('erc20', 1000n, 6, 0n), false)
  assert.equal(isDust('erc20', 10n ** 15n - 1n, null, 0n), true, 'unknown decimals default to 18')
  assert.equal(isDust('erc20', 0n, 2, 0n), false)
  assert.equal(isDust('erc20', 1n, 2, 0n), false, 'tiny-decimal tokens: threshold 1')
})

test('zero-value transfers are always spam', () => {
  const r = classifyTransfer({ ...base, kind: 'erc20', value: 0n, token: { known: true, symbol: 'USDX', decimals: 6 } })
  assert.deepEqual(r, { spam: true, reasons: ['zero-value'] })
})

test('look-alike only counts for zero/dust transfers', () => {
  const dust = classifyTransfer({ ...base, kind: 'native', value: 5n, token: null, lookAlikeOf: '0xabc' })
  assert.equal(dust.spam, true)
  assert.deepEqual(dust.reasons, ['dust', 'look-alike'])
  const big = classifyTransfer({ ...base, kind: 'native', value: 10n ** 18n, token: null, lookAlikeOf: '0xabc' })
  assert.deepEqual(big, { spam: false, reasons: [] })
})

test('dust of an unknown token is spam; dust of a known token is not', () => {
  const unknown = classifyTransfer({ ...base, kind: 'erc20', value: 1n, token: { known: false, symbol: 'FOO', decimals: 18 } })
  assert.deepEqual(unknown, { spam: true, reasons: ['dust', 'unknown-token'] })
  const known = classifyTransfer({ ...base, kind: 'erc20', value: 1n, token: { known: true, symbol: 'FOO', decimals: 18 } })
  assert.deepEqual(known, { spam: false, reasons: ['dust'] })
  const normal = classifyTransfer({ ...base, kind: 'erc20', value: 10n ** 18n, token: { known: false, symbol: 'FOO', decimals: 18 } })
  assert.deepEqual(normal, { spam: false, reasons: ['unknown-token'] })
})

test('unknown tokens copying a protected symbol are spam (case-insensitive)', () => {
  const r = classifyTransfer({ ...base, kind: 'erc20', value: 10n ** 20n, token: { known: false, symbol: ' txds ', decimals: 18 } })
  assert.deepEqual(r, { spam: true, reasons: ['unknown-token', 'impersonating-token'] })
  const usdx = classifyTransfer({ ...base, kind: 'erc20', value: 10n ** 20n, token: { known: false, symbol: 'USDX', decimals: 6 } })
  assert.equal(usdx.spam, true)
})

test('poison candidates', () => {
  assert.equal(isPoisonCandidate({ kind: 'native', value: 0n, token: null, dustNativeWei: 10n }), true)
  assert.equal(isPoisonCandidate({ kind: 'native', value: 9n, token: null, dustNativeWei: 10n }), true)
  assert.equal(isPoisonCandidate({ kind: 'native', value: 10n, token: null, dustNativeWei: 10n }), false)
})

test('findLookAlike: same first 4 / last 4 hex chars, different address', () => {
  const real = '0x1234aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678'
  const fake = '0x1234bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb5678'
  const other = '0x9999bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb5678'
  assert.equal(findLookAlike(fake, [other, real]), real)
  assert.equal(findLookAlike(real, [real]), null, 'an address is not its own look-alike')
  assert.equal(findLookAlike(fake, [other]), null)
  assert.equal(findLookAlike(fake.toUpperCase().replace('0X', '0x'), [real]), real, 'case-insensitive')
})

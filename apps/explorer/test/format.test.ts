import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  classifySearch,
  expiryBadge,
  formatDuration,
  formatGwei,
  formatNative,
  formatToken,
  formatUnits,
  percent,
  shortAddress,
  shortHash,
  spamReasonLabel,
  timeAgo,
  vaultStatusTone,
} from '../src/lib/format.ts'
import { parseRoute } from '../src/lib/router.ts'

test('formatUnits groups thousands, trims zeros, never shows dust as 0', () => {
  assert.equal(formatUnits(1234567890000000000000n, 18), '1,234.56789')
  assert.equal(formatUnits('0xde0b6b3a7640000', 18), '1')
  assert.equal(formatUnits(0n, 18), '0')
  assert.equal(formatUnits(1n, 18), '<0.000001')
  assert.equal(formatUnits(1n, 18, 2), '<0.01')
  assert.equal(formatUnits(1500000n, 6), '1.5')
  assert.equal(formatUnits(123n, 0), '123')
  assert.equal(formatUnits(-2500000000000000000n, 18), '-2.5')
  assert.equal(formatUnits(10n ** 30n, 18), '1,000,000,000,000')
})

test('native, gwei and token amounts', () => {
  assert.equal(formatNative('0x14d1120d7b160000'), '1.5 tXDS')
  assert.equal(formatGwei('0x3b9aca00'), '1 gwei')
  assert.equal(formatGwei(null), '—')
  assert.equal(formatToken('0x2710', { decimals: 2, symbol: 'USDX' }), '100 USDX')
  assert.equal(formatToken('0x1', { decimals: 18, symbol: 'X' }, true), 'Unlimited X')
  assert.equal(formatToken('0x5', { decimals: null, symbol: null }), '<0.000001 units (raw)')
})

test('short forms keep 6+6 hex chars so 4/4 look-alikes still look different', () => {
  const real = '0x1234aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678'
  const fake = '0x1234bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb5678'
  assert.equal(shortAddress(real), '0x1234aa…aa5678')
  assert.notEqual(shortAddress(real), shortAddress(fake))
  assert.equal(shortAddress(null), '—')
  assert.equal(shortHash(`0x${'ab'.repeat(32)}`), '0xabababab…abababab')
})

test('durations and relative time', () => {
  assert.equal(formatDuration(5), '5 s')
  assert.equal(formatDuration(125), '2 min')
  assert.equal(formatDuration(3 * 3600 + 120), '3 h 2 min')
  assert.equal(formatDuration(86400 * 2 + 3600), '2 d 1 h')
  assert.equal(timeAgo(100, 101), 'just now')
  assert.equal(timeAgo(100, 160), '1 min ago')
  assert.equal(timeAgo(200, 100), 'in 1 min')
})

test('approval expiry badges: never-expiring is the danger case', () => {
  const now = 1_000_000
  assert.deepEqual(expiryBadge(null, now), { text: 'Never expires', tone: 'danger' })
  assert.deepEqual(expiryBadge(now + 3600, now), { text: 'Expires in 1 h', tone: 'warn' })
  assert.deepEqual(expiryBadge(now + 30 * 86400, now), { text: 'Expires in 30 d', tone: 'ok' })
  assert.deepEqual(expiryBadge(now - 120, now), { text: 'Expired 2 min ago', tone: 'neutral' })
})

test('labels and tones', () => {
  assert.equal(spamReasonLabel('look-alike'), 'Look-alike address')
  assert.equal(spamReasonLabel('something-new'), 'something-new')
  assert.equal(vaultStatusTone('ready'), 'danger')
  assert.equal(vaultStatusTone('executed'), 'ok')
  assert.equal(percent(1n, 3n), '33.33%')
  assert.equal(percent(8n, 10n), '80%')
  assert.equal(percent(1n, 0n), '0%')
})

test('search classification', () => {
  assert.deepEqual(classifySearch('  '), { type: 'empty' })
  assert.deepEqual(classifySearch('0012'), { type: 'block', id: '12' })
  assert.equal(classifySearch('0x1234aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678').type, 'address')
  assert.equal(classifySearch(`0x${'AB'.repeat(32)}`).type, 'hash')
  assert.deepEqual(classifySearch('Alice.XDEVS'), { type: 'name', id: 'alice.xdevs' })
  assert.deepEqual(classifySearch('alice'), { type: 'text', id: 'alice' })
})

test('router', () => {
  assert.deepEqual(parseRoute('/'), { page: 'home' })
  assert.deepEqual(parseRoute('/block/12/'), { page: 'block', id: '12' })
  assert.deepEqual(parseRoute('/address/0xabc', '?tab=approvals'), { page: 'address', address: '0xabc', tab: 'approvals' })
  assert.deepEqual(parseRoute('/name/alice.xdevs'), { page: 'name', name: 'alice.xdevs' })
  assert.deepEqual(parseRoute('/search', '?q=a%20b'), { page: 'search', q: 'a b' })
  assert.deepEqual(parseRoute('/nope/x/y'), { page: 'notfound' })
  assert.deepEqual(parseRoute('/tx/%E0%A4%A'), { page: 'tx', hash: '%E0%A4%A' }, 'malformed escapes do not throw')
})

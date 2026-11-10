import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { keccak256, namehash, toHex } from 'viem'
import {
  DEV_PER_XDS,
  InvalidXnsNameError,
  areConfusableXnsLabels,
  checkXnsLabel,
  findLookAlikeAddresses,
  formatXds,
  isLookAlikeAddress,
  isValidXnsLabel,
  isValidXnsName,
  normalizeXnsName,
  parseXds,
  xnsLabelhash,
  xnsNamehash,
  xnsSkeleton,
} from '../src/index.ts'

describe('units', () => {
  it('parseXds / formatXds round-trip with 18 decimals', () => {
    assert.equal(parseXds('1'), DEV_PER_XDS)
    assert.equal(parseXds('12.5'), 12_500_000_000_000_000_000n)
    assert.equal(parseXds('0.000000000000000001'), 1n)
    assert.equal(formatXds(1n), '0.000000000000000001')
    assert.equal(formatXds(parseXds('300000000')), '300000000')
    assert.equal(formatXds(-parseXds('1.25')), '-1.25')
  })
})

describe('XNS names', () => {
  it('label rules mirror XNSLabel.sol', () => {
    assert.equal(checkXnsLabel('abc'), null)
    assert.equal(checkXnsLabel('a-b-c'), null)
    assert.equal(checkXnsLabel('007'), null)
    assert.equal(checkXnsLabel('ab'), 'too-short')
    assert.equal(checkXnsLabel('a'.repeat(63)), null)
    assert.equal(checkXnsLabel('a'.repeat(64)), 'too-long')
    assert.equal(checkXnsLabel('Abc'), 'forbidden-character')
    assert.equal(checkXnsLabel('a_c'), 'forbidden-character')
    assert.equal(checkXnsLabel('a c'), 'forbidden-character')
    assert.equal(checkXnsLabel('-abc'), 'bad-hyphen')
    assert.equal(checkXnsLabel('abc-'), 'bad-hyphen')
    assert.equal(checkXnsLabel('xn--abc'), 'bad-hyphen')
    assert.equal(isValidXnsLabel('alice'), true)
  })

  it('normalizes user input', () => {
    assert.equal(normalizeXnsName('alice'), 'alice.xdevs')
    assert.equal(normalizeXnsName('  ALICE.XDEVS '), 'alice.xdevs')
    assert.equal(normalizeXnsName('pay.alice.xdevs'), 'pay.alice.xdevs')
    for (const bad of ['al', 'al.xdevs', 'alice.eth', 'аlice', 'alice😀', 'a_b_c', '.xdevs', 'alice..xdevs', 'bob--x', ''])
      assert.throws(() => normalizeXnsName(bad), InvalidXnsNameError, bad)
    assert.equal(isValidXnsName('alice'), true)
    assert.equal(isValidXnsName('аlice'), false) // Cyrillic "а"
  })

  it('namehash / labelhash follow ENS (EIP-137)', () => {
    const xdevsNode = keccak256(`0x${'00'.repeat(32)}${keccak256(toHex('xdevs')).slice(2)}`)
    const expected = keccak256(`${xdevsNode}${keccak256(toHex('alice')).slice(2)}`)
    assert.equal(xnsNamehash('Alice'), expected)
    assert.equal(xnsNamehash('alice.xdevs'), namehash('alice.xdevs'))
    assert.equal(xnsLabelhash('alice'), keccak256(toHex('alice')))
    assert.throws(() => xnsLabelhash('Al'), InvalidXnsNameError)
  })

  it('skeletons catch ASCII look-alike labels', () => {
    assert.equal(xnsSkeleton('paypa1'), 'paypal')
    assert.equal(xnsSkeleton('g00gle'), 'google')
    assert.equal(xnsSkeleton('rnetamask'), 'metamask')
    assert.equal(xnsSkeleton('vvallet'), 'wallet')
    assert.equal(xnsSkeleton('cloge'), 'doge')
    assert.equal(areConfusableXnsLabels('paypa1', 'paypal'), true)
    assert.equal(areConfusableXnsLabels('paypal', 'paypal'), false)
    assert.equal(areConfusableXnsLabels('alice', 'bob'), false)
  })
})

describe('isLookAlikeAddress (§16.3)', () => {
  const real = '0x1234567890abcdef1234567890abcdef1234abcd'
  const fake = '0x1234ffffffffffffffffffffffffffffffffabcd'
  const fake6 = '0x123456ffffffffffffffffffffffffffff34abcd'

  it('flags different addresses with equal first/last n hex chars', () => {
    assert.equal(isLookAlikeAddress(real, fake), true)
    assert.equal(isLookAlikeAddress(real, fake.toUpperCase().replace('0X', '0x') as `0x${string}`), true)
    assert.equal(isLookAlikeAddress(real, fake, 6), false)
    assert.equal(isLookAlikeAddress(real, fake6, 6), true)
  })

  it('does not flag identical or clearly different addresses', () => {
    assert.equal(isLookAlikeAddress(real, real), false)
    assert.equal(isLookAlikeAddress(real, real.toUpperCase().replace('0X', '0x') as `0x${string}`), false)
    assert.equal(isLookAlikeAddress(real, '0x9234567890abcdef1234567890abcdef1234abcd'), false)
    assert.equal(isLookAlikeAddress(real, '0x1234567890abcdef1234567890abcdef1234abc0'), false)
  })

  it('validates inputs', () => {
    assert.throws(() => isLookAlikeAddress(real, '0x1234'), /invalid address/)
    assert.throws(() => isLookAlikeAddress(real, fake, 0), RangeError)
    assert.throws(() => isLookAlikeAddress(real, fake, 21), RangeError)
  })

  it('findLookAlikeAddresses filters a history', () => {
    assert.deepEqual(findLookAlikeAddresses(fake, [real, '0x0000000000000000000000000000000000000001', fake]), [real])
  })
})

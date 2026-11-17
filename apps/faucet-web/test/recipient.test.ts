import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { checkRecipient, shortHex } from '../lib/recipient.ts'

describe('checkRecipient', () => {
  it('accepts any-case hex addresses and normalizes to lowercase', () => {
    const r = checkRecipient('  0xAbCdEf0123456789aBcDeF0123456789AbCdEf01 ')
    assert.deepEqual(r, { ok: true, kind: 'address', value: '0xabcdef0123456789abcdef0123456789abcdef01' })
  })

  it('treats empty input as empty, not as an error', () => {
    assert.deepEqual(checkRecipient('   '), { ok: false, empty: true, message: '' })
  })

  it('explains what is wrong with malformed addresses', () => {
    const short = checkRecipient('0x1234')
    assert.equal(short.ok, false)
    assert.match((short as { message: string }).message, /40 hex characters.*has 4/)
    const bad = checkRecipient('0xZZ11111111111111111111111111111111111111')
    assert.match((bad as { message: string }).message, /0–9 and a–f/)
    const noPrefix = checkRecipient('1111111111111111111111111111111111111111')
    assert.match((noPrefix as { message: string }).message, /start with 0x/)
  })

  it('refuses the same reserved addresses as the Worker', () => {
    for (const a of [
      '0x0000000000000000000000000000000000000000',
      '0x0000000000000000000000000000000000000001',
      '0x0000000000000000000000000000000000001000',
      '0x000000000000000000000000000000000000ffff',
      '0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE',
    ]) {
      assert.equal(checkRecipient(a).ok, false, a)
    }
    assert.equal(checkRecipient('0x0000000000000000000000000000000000010000').ok, true)
  })

  it('accepts valid .xdevs names (lowercased) and rejects malformed ones', () => {
    assert.deepEqual(checkRecipient('Alice.XDEVS'), { ok: true, kind: 'name', value: 'alice.xdevs' })
    assert.deepEqual(checkRecipient('pay.alice-01.xdevs'), { ok: true, kind: 'name', value: 'pay.alice-01.xdevs' })
    for (const n of ['.xdevs', '-alice.xdevs', 'alice-.xdevs', 'al_ice.xdevs', 'alice..xdevs']) {
      assert.equal(checkRecipient(n).ok, false, n)
    }
  })

  it('rejects absurdly long input', () => {
    assert.equal(checkRecipient('a'.repeat(300)).ok, false)
  })
})

describe('shortHex', () => {
  it('shortens long hex and leaves short strings alone', () => {
    assert.equal(shortHex('0x1234567890abcdef1234'), '0x1234…1234')
    assert.equal(shortHex('0x12'), '0x12')
  })
})

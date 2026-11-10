import { test } from 'node:test'
import assert from 'node:assert/strict'
import { addressParts, findLookAlikes, HISTORY_MAX, loadHistory, memoryStore, recordAddress } from '../src/lib/lookalike.ts'

const REAL = '0x1234aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678'
const FAKE = '0x1234BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB5678'
const OTHER = '0x9999aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa5678'

test('records history newest first, deduplicated case-insensitively', () => {
  const s = memoryStore()
  recordAddress(s, REAL, 'view', 1)
  recordAddress(s, OTHER, 'search', 2)
  const list = recordAddress(s, REAL.toUpperCase().replace('0X', '0x'), 'view', 3)
  assert.deepEqual(list.map((e) => e.at), [3, 2])
  assert.equal(loadHistory(s).length, 2)
  assert.equal(loadHistory(s)[1]!.source, 'search')
})

test('history is capped and ignores invalid input', () => {
  const s = memoryStore()
  for (let i = 0; i < HISTORY_MAX + 10; i++) recordAddress(s, `0x${i.toString(16).padStart(40, '0')}`, 'view', i)
  assert.equal(loadHistory(s).length, HISTORY_MAX)
  recordAddress(s, 'not-an-address', 'view', 1)
  assert.equal(loadHistory(s).length, HISTORY_MAX)
  assert.deepEqual(loadHistory(memoryStore('{broken')), [])
  assert.deepEqual(loadHistory(memoryStore('[{"address":"0x12","at":1}, 5, null]')), [])
})

test('detects look-alikes of recently viewed addresses, never the address itself', () => {
  const s = memoryStore()
  recordAddress(s, REAL, 'view', 1)
  recordAddress(s, OTHER, 'view', 2)
  const hits = findLookAlikes(FAKE, loadHistory(s))
  assert.deepEqual(hits.map((h) => h.address), [REAL])
  assert.deepEqual(findLookAlikes(REAL, loadHistory(s)), [])
  assert.deepEqual(findLookAlikes('0x12', loadHistory(s)), [])
})

test('address parts expose the copied head/tail and the differing middle', () => {
  assert.deepEqual(addressParts(REAL), { head: '0x1234', middle: 'a'.repeat(32), tail: '5678' })
})

test('a throwing storage never breaks the page', () => {
  const bad = { get: () => { throw new Error('denied') }, set: () => { throw new Error('denied') } }
  assert.deepEqual(loadHistory(bad), [])
  assert.doesNotThrow(() => recordAddress(bad, REAL, 'view', 1))
})

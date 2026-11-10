import { test } from 'node:test'
import assert from 'node:assert/strict'
import { decodeLog, signatureOf, toJsonSafe, TOPIC, erc20Events } from '../src/decode.ts'
import { accountEvents, permit2Events, vaultEvents, xdrc20Events, xnsEvents } from '../src/abi.generated.ts'
import { addr, log } from './helpers.ts'
import { keccak256, toHex } from 'viem'

const A = addr(0xa11ce)
const B = addr(0xb0b)
const T = addr(0x7070)

test('ERC-20 Transfer decodes; the 4-topic ERC-721 shape does not', () => {
  const l = log(erc20Events, 'Transfer', { from: A, to: B, value: 5n }, T)
  const ev = decodeLog(l)
  assert.equal(ev?.event, 'Transfer')
  assert.equal(ev?.standard, 'erc20')
  assert.equal(ev?.args.value, 5n)
  const nft = { ...l, topics: [...l.topics, `0x${'0'.repeat(63)}1`] as `0x${string}`[], data: '0x' as const }
  assert.equal(decodeLog(nft), null)
})

test('Permit2 Approval and ERC-20 Approval have different topic0 and decode to different standards', () => {
  assert.notEqual(TOPIC['erc20.Approval'], TOPIC['permit2.Approval'])
  const p = decodeLog(log(permit2Events, 'Approval', { owner: A, token: T, spender: B, amount: 7n, expiration: 1234 }, addr(0x1008)))
  assert.equal(p?.standard, 'permit2')
  assert.equal(p?.args.expiration, 1234)
  const x = decodeLog(log(xdrc20Events, 'ApprovalWithExpiry', { owner: A, spender: B, value: 9n, expiry: 99n }, T))
  assert.equal(x?.standard, 'xdrc20')
  assert.equal(x?.signature, 'ApprovalWithExpiry(address,address,uint256,uint64)')
})

test('vault, account (tuple) and XNS events decode', () => {
  const q = decodeLog(log(vaultEvents, 'VaultActionQueued', { id: 3n, target: B, value: 10n, data: '0xdeadbeef', executeAfter: 777n }, A))
  assert.equal(q?.standard, 'vault')
  assert.equal(q?.args.data, '0xdeadbeef')
  const policy = { enabled: true, blockUnlimitedApprovals: false, allowInstantSignatures: false, delay: 86400n, nativeThreshold: 1n, dailyLimit: 2n, dailyGasBudget: 3n }
  const pu = decodeLog(log(accountEvents, 'VaultPolicyUpdated', { policy }, A))
  assert.equal((pu?.args.policy as { enabled: boolean }).enabled, true)
  assert.equal(pu?.signature, 'VaultPolicyUpdated((bool,bool,bool,uint64,uint128,uint128,uint128))')
  const nr = decodeLog(log(xnsEvents, 'NameRegistered', { label: 'alice', labelhash: keccak256(toHex('alice')), owner: A, expires: 5n, fee: 0n }, addr(0x1004)))
  assert.equal(nr?.args.label, 'alice')
  const xt = decodeLog(log(xnsEvents, 'Transfer', { node: keccak256(toHex('n')), owner: B }, addr(0x1004)))
  assert.equal(xt?.standard, 'xns', 'XNS Transfer(bytes32,address) is not an ERC-20 Transfer')
})

test('unknown topics and malformed data return null', () => {
  assert.equal(decodeLog({ address: A, topics: [keccak256(toHex('Nope()'))], data: '0x' }), null)
  assert.equal(decodeLog({ address: A, topics: [], data: '0x' }), null)
  const l = log(erc20Events, 'Transfer', { from: A, to: B, value: 5n }, T)
  assert.equal(decodeLog({ ...l, data: '0x01' }), null)
})

test('toJsonSafe turns bigints into hex, recursively', () => {
  assert.deepEqual(toJsonSafe({ a: 255n, b: [1n, { c: 'x', d: true }] }), { a: '0xff', b: ['0x1', { c: 'x', d: true }] })
})

test('signatureOf handles tuple arrays', () => {
  assert.equal(
    signatureOf({ type: 'event', name: 'E', inputs: [{ type: 'tuple[]', name: 'x', components: [{ type: 'address', name: 'a' }, { type: 'uint8', name: 'b' }] }] }),
    'E((address,uint8)[])',
  )
})

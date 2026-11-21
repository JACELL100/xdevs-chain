import { test } from 'node:test'
import assert from 'node:assert/strict'
import { encodeAbiParameters, encodeFunctionData, pad, parseAbi, toEventSelector } from 'viem'
import { predeploys } from '@xdevs/sdk'
import { commitView, computeStats, consensusAddress, formatBlockTime, formatTps } from '../src/lib/chainstats.ts'
import { decodeCall, decodeLog, displayValue, vaultActivity } from '../src/lib/decode.ts'
import { decodeDemoTxHash, demoBlock, demoCommit, demoHead, demoTx, demoTxHash, demoVaultTxHash, DEMO_VALIDATORS } from '../src/lib/fixtures.ts'
import { mergeBlocks, mergeTxs } from '../src/lib/live.ts'
import { isUnreachable } from '../src/lib/net.ts'
import { knownValidatorName, uptimeFromCerts } from '../src/lib/validators.ts'
import type { ValidatorRow } from '../src/lib/types.ts'

test('stats are computed from the block window, not claimed', () => {
  const blocks = Array.from({ length: 11 }, (_, i) => ({ number: 100 + i, timestamp: 1000 + i * 2, txCount: 4 }))
  const s = computeStats([...blocks].reverse())
  assert.equal(s.height, 110)
  assert.equal(s.avgBlockTime, 2)
  assert.equal(s.tps, 2) // 10 blocks × 4 txs after the first block, over 20 s
  assert.equal(s.sample, 11)
  assert.deepEqual(computeStats([]), { height: null, avgBlockTime: null, tps: null, sample: 0 })
  assert.equal(computeStats([blocks[0]!]).avgBlockTime, null)
  // Duplicates (a head seen twice) do not skew the average.
  assert.equal(computeStats([...blocks, blocks[3]!]).avgBlockTime, 2)
  assert.equal(formatTps(0.004), '<0.01')
  assert.equal(formatTps(3.456), '3.46')
  assert.equal(formatTps(null), '—')
  assert.equal(formatBlockTime(1.0049), '1.00 s')
})

test('commit certificate: 2/3+ quorum by voting power, unknown signers kept', () => {
  const set = [
    { operator: '0x' + '1'.repeat(40), consensusKey: '0x' + 'a'.repeat(64), votingPower: 40n },
    { operator: '0x' + '2'.repeat(40), consensusKey: '0x' + 'b'.repeat(64), votingPower: 30n },
    { operator: '0x' + '3'.repeat(40), consensusKey: '0x' + 'c'.repeat(64), votingPower: 30n },
  ]
  const sig = (k: string) => ({ validator: consensusAddress(k), signature: '0x01' })
  const two = commitView([sig(set[0]!.consensusKey), sig(set[1]!.consensusKey)], set)
  assert.equal(two.signedPower, 70n)
  assert.equal(two.signedBps, 7000)
  assert.equal(two.quorum, true)
  const one = commitView([sig(set[1]!.consensusKey), sig(set[2]!.consensusKey)], set)
  assert.equal(one.quorum, false, '60% is not more than 2/3')
  const stranger = commitView([{ validator: '0x' + '9'.repeat(40), signature: '0x02' }], set)
  assert.equal(stranger.votes.length, 4)
  assert.equal(stranger.votes.at(-1)!.operator, null)
  assert.equal(commitView([], []).quorum, null)
  assert.equal(consensusAddress('0x12'), '0x')
})

test('validators: Enclave names, uptime from commit certificates', () => {
  assert.equal(knownValidatorName(717171, '0x42073689894ba795fF54D4Aa2b528F5b48280782'), 'val-4 · Mumbai')
  assert.equal(knownValidatorName(717171, '0x4E0a69305925f213D717e002EF7E4bae4f5B0402'), 'val-1 · Hyderabad')
  assert.equal(knownValidatorName(1, '0x42073689894ba795fF54D4Aa2b528F5b48280782'), null)
  const row = (k: string, inSigningSet = true) =>
    ({ operator: '0x' + k.repeat(40), consensusKey: '0x' + k.repeat(64), inSigningSet, uptimeBps: null }) as unknown as ValidatorRow
  const [a, b, c] = [row('a'), row('b'), row('c', false)]
  const cert = (...keys: string[]) => ({ height: 1, round: 0, blockHash: '0x', signatures: keys.map((k) => ({ validator: consensusAddress('0x' + k.repeat(64)), signature: '0x01' })) })
  const certs = [cert('a', 'b'), cert('a'), cert('a', 'b'), cert('a')]
  assert.deepEqual(uptimeFromCerts([a!, b!, c!], certs), [10000, 5000, null])
  assert.deepEqual(uptimeFromCerts([a!], []), [null])
})

test('calldata decoding for predeploys and token standards', () => {
  const staking = encodeFunctionData({ abi: parseAbi(['function delegate(address validator) payable']), functionName: 'delegate', args: ['0x' + '4'.repeat(40) as `0x${string}`] })
  const d = decodeCall(predeploys.STAKING, staking)!
  assert.equal(d.contract, 'Staking')
  assert.equal(d.name, 'delegate')
  assert.equal(d.args[0]!.name, 'validator')

  const xdrc = encodeFunctionData({ abi: parseAbi(['function approve(address spender, uint256 value, uint64 expiry)']), functionName: 'approve', args: ['0x' + '5'.repeat(40) as `0x${string}`, 10n, 99n] })
  const a = decodeCall('0x' + '6'.repeat(40), xdrc)!
  assert.equal(a.standard, 'xdrc20')
  assert.equal(a.signature, 'approve(address,uint256,uint64)')
  assert.equal(displayValue(a.args[2]!.value), '99')

  const erc = encodeFunctionData({ abi: parseAbi(['function transfer(address to, uint256 value)']), functionName: 'transfer', args: ['0x' + '7'.repeat(40) as `0x${string}`, 5n] })
  assert.equal(decodeCall('0x' + '6'.repeat(40), erc)!.standard, 'erc20')
  assert.equal(decodeCall(predeploys.WXDS, '0xd0e30db0')!.name, 'deposit')
  assert.equal(decodeCall(null, '0x'), null)
  assert.equal(decodeCall('0x' + '6'.repeat(40), '0xdeadbeef'), null)
  assert.equal(displayValue([1n, { a: 2n }]), '[1, { a: 2 }]')
})

test('log decoding and Guardian Vault activity', () => {
  const sel = toEventSelector('VaultActionQueued(uint256,address,uint256,bytes,uint64)')
  const account = '0x' + '8'.repeat(40)
  const data = encodeAbiParameters([{ type: 'address' }, { type: 'uint256' }, { type: 'bytes' }, { type: 'uint64' }], ['0x' + '9'.repeat(40) as `0x${string}`, 5n, '0x', 1234n])
  const logs = [{ address: account, topics: [sel, pad('0x7', { size: 32 })], data }]
  const v = vaultActivity(logs)
  assert.equal(v.length, 1)
  assert.equal(v[0]!.kind, 'queued')
  assert.equal(v[0]!.id, 7n)
  assert.equal(v[0]!.executeAfter, 1234)
  assert.equal(decodeLog(logs[0]!.topics, data)!.event, 'VaultActionQueued')
  assert.equal(decodeLog([], '0x'), null)
  assert.deepEqual(vaultActivity([{ address: account, topics: ['0x' + '0'.repeat(64)], data: '0x' }]), [])
})

test('demo chain is deterministic and self-consistent', () => {
  const h = demoTxHash(1_234_567, 3)
  assert.deepEqual(decodeDemoTxHash(h), { block: 1_234_567, index: 3 })
  assert.equal(decodeDemoTxHash('0x' + 'ab'.repeat(32)), null)
  const head = demoHead()
  const b = demoBlock(head - 5)
  assert.deepEqual(demoBlock(head - 5), b)
  assert.equal(b.txCount, b.transactions.length)
  for (const t of b.transactions) assert.equal(demoTx(t.hash)!.hash, t.hash)
  const c = demoCommit(head - 5)
  const set = DEMO_VALIDATORS.filter((v) => v.status === 'active').map((v) => ({ operator: v.operator, consensusKey: v.consensusKey, votingPower: BigInt(v.votingPower!) }))
  assert.equal(commitView(c.signatures, set).quorum, true)
  const vault = demoTx(demoVaultTxHash(head))!
  assert.equal(vaultActivity(vault.logs).length, 1)
})

test('live window merging keeps the newest, without duplicates', () => {
  const b = (n: number) => ({ number: n } as never)
  assert.deepEqual(mergeBlocks([b(3), b(2)], [b(4), b(3)], 3).map((x: { number: number }) => x.number), [4, 3, 2])
  const t = (h: string, n: number, i = 0) => ({ hash: h, blockNumber: n, txIndex: i } as never)
  assert.deepEqual(mergeTxs([t('a', 1)], [t('b', 2), t('a', 1)], 5).map((x: { hash: string }) => x.hash), ['b', 'a'])
})

test('reachability classification', () => {
  assert.equal(isUnreachable({ name: 'HttpRequestError', message: 'x' }), true)
  assert.equal(isUnreachable({ code: 'NETWORK' }), true)
  assert.equal(isUnreachable(new TypeError('Failed to fetch')), true)
  assert.equal(isUnreachable({ status: 404 }), false)
  assert.equal(isUnreachable({ name: 'AbortError' }), false)
  assert.equal(isUnreachable(null), false)
})

// Phase 4 safety RPCs: request shapes and parsing of the node's extended responses.
import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { getAddress } from 'viem'
import { parseVaultEvent, watchVaultEvents } from '../src/index.ts'
import { mockClient } from './helpers.ts'

const ALICE = getAddress('0x00000000000000000000000000000000000a11ce')
const BOB = getAddress('0x0000000000000000000000000000000000000b0b')
const USDC = getAddress('0x1111111111111111111111111111111111111111')

const recipientAnswer = {
  address: BOB,
  firstTime: true,
  lookAlikeOf: [],
  name: 'bob.xdevs',
  isContract: false,
  warnings: [{ code: 'FIRST_TIME_RECIPIENT', severity: 'low', message: 'm' }],
  facts: {
    address: BOB,
    resolvedFrom: 'bob.xdevs',
    isZeroAddress: false,
    isPrecompile: false,
    isSystemContract: false,
    isContract: false,
    codeSize: 0,
    isSmartAccount: false,
    delegatedTo: null,
    eoaLock: null,
    nonce: 0,
    balance: '0x0',
    primaryName: 'bob.xdevs',
  },
  advice: {
    firstTime: true,
    lookAlikeOf: [],
    flags: [{ code: 'FIRST_TIME_RECIPIENT', severity: 'low', message: 'm' }],
    history: { fromBlock: 0, toBlock: 100, complete: true },
    disclaimer: 'advice, not fact',
  },
  blockNumber: '0x64',
}

describe('safety RPC actions', () => {
  it('checkRecipient accepts names, an optional sender and a block', async () => {
    const { client, calls } = mockClient(() => recipientAnswer)
    const r = await client.checkRecipient({ to: ' Bob ' })
    assert.equal(r.facts?.primaryName, 'bob.xdevs')
    assert.equal(r.advice?.history?.toBlock, 100)
    await client.checkRecipient({ from: ALICE, to: 'bob', blockTag: 'safe' })
    await client.checkRecipient({ to: BOB, blockNumber: 16n })
    assert.deepEqual(calls, [
      { method: 'xdevs_checkRecipient', params: ['bob.xdevs'] },
      { method: 'xdevs_checkRecipient', params: [ALICE, 'bob.xdevs', 'safe'] },
      { method: 'xdevs_checkRecipient', params: [null, BOB, '0x10'] },
    ])
    await assert.rejects(client.checkRecipient({ to: 'b' }), /label/)
  })

  it('getApprovals sends options only when given', async () => {
    const answer = [
      { kind: 'erc20', token: USDC, symbol: 'USDC', spender: BOB, amount: 'unlimited', expires: null, unlimited: true },
    ]
    const { client, calls } = mockClient(() => answer)
    assert.deepEqual(await client.getApprovals({ owner: ALICE, tokens: [USDC], fromBlock: 16n }), answer)
    await client.getApprovals({ owner: ALICE, fromBlock: 1n, blockTag: 'finalized' })
    assert.deepEqual(calls, [
      { method: 'xdevs_getApprovals', params: [ALICE, { tokens: [USDC], fromBlock: '0x10' }, 'latest'] },
      { method: 'xdevs_getApprovals', params: [ALICE, { fromBlock: '0x1' }, 'finalized'] },
    ])
    await assert.rejects(client.getApprovals({ owner: ALICE, tokens: ['0x12'] }), /token is not a valid address/)
  })

  it('getVaultQueue parses the extended node fields', async () => {
    const { client } = mockClient(() => [
      {
        account: ALICE,
        id: '0x2',
        target: BOB,
        value: '0x5',
        data: '0xbeef',
        executeAfter: '0x6553f100',
        expiresAt: '0x6566b780',
        status: 'queued',
        calls: [{ target: BOB, value: '0x5', data: '0xbeef' }],
        cancelVotes: 1,
      },
    ])
    const [a] = await client.getVaultQueue({ vault: ALICE })
    assert.deepEqual(a, {
      id: 2n,
      target: BOB,
      value: 5n,
      data: '0xbeef',
      executeAfter: 1_700_000_000n,
      account: ALICE,
      expiresAt: 1_701_230_464n,
      status: 'queued',
      calls: [{ target: BOB, value: 5n, data: '0xbeef' }],
      cancelVotes: 1,
    })
  })

  it('parseVaultEvent converts quantities', () => {
    assert.deepEqual(
      parseVaultEvent({
        type: 'queued',
        account: ALICE,
        id: '0x7',
        target: BOB,
        value: '0xde0b6b3a7640000',
        data: '0x',
        executeAfter: '0x6553f100',
        blockNumber: '0x9',
        logIndex: '0x0',
      }),
      {
        type: 'queued',
        account: ALICE,
        id: 7n,
        target: BOB,
        value: 10n ** 18n,
        data: '0x',
        executeAfter: 1_700_000_000n,
        blockNumber: 9n,
        logIndex: 0,
      },
    )
    assert.deepEqual(parseVaultEvent({ type: 'frozen', account: ALICE, frozenUntil: '0x10' }), {
      type: 'frozen',
      account: ALICE,
      frozenUntil: 16n,
    })
  })

  it('watchVaultEvents subscribes over a WebSocket transport', async () => {
    const seen: unknown[] = []
    let params: unknown[] = []
    let unsubscribed = false
    const fakeClient = {
      transport: {
        async subscribe(args: { params: unknown[]; onData: (d: unknown) => void }) {
          params = args.params
          args.onData({ subscription: '0x1', result: { type: 'executed', account: ALICE, id: '0x3' } })
          args.onData({ subscription: '0x1' }) // no result: ignored
          return {
            async unsubscribe() {
              unsubscribed = true
              return true
            },
          }
        },
      },
    }
    const stop = await watchVaultEvents(fakeClient as never, { account: ALICE, onEvent: (e) => seen.push(e) })
    assert.deepEqual(params, ['vaultEvents', { account: ALICE }])
    assert.deepEqual(seen, [{ type: 'executed', account: ALICE, id: 3n }])
    await stop()
    assert.equal(unsubscribed, true)

    // HTTP transports cannot subscribe.
    const { client } = mockClient(() => null)
    await assert.rejects(client.watchVaultEvents({ onEvent: () => {} }), /WebSocket/)
  })
})

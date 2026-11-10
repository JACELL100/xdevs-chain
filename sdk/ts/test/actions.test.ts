import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { createClient, createPublicClient, custom, getAddress } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import {
  XDEVS_RPC_METHODS,
  XdevsMethodNotAvailableError,
  InvalidXnsNameError,
  isMethodNotFoundError,
  parseXds,
  xdevsActions,
  xdevsDevnet,
} from '../src/index.ts'
import { mockClient, rpcError } from './helpers.ts'

const ALICE = getAddress('0x00000000000000000000000000000000000a11ce')
const BOB = getAddress('0x0000000000000000000000000000000000000b0b')
const USDC = '0x1111111111111111111111111111111111111111'

const simulationResponse = {
  success: true,
  gasUsed: '0x5208',
  assetChanges: [{ token: 'native', symbol: 'tXDS', from: ALICE, to: BOB, amount: '12.5' }],
  approvals: [{ token: USDC, spender: BOB, amount: 'unlimited', expires: null }],
  warnings: [
    { code: 'UNLIMITED_APPROVAL', severity: 'high', message: 'This gives 0xRouter permission to spend ALL your USDC forever.' },
    { code: 'NEW_CONTRACT', severity: 'medium', message: 'This contract was deployed 2 hours ago.' },
  ],
  disclaimer: 'Simulation results are advisory and may differ if state changes before inclusion.',
}

describe('xdevs_ actions', () => {
  it('decorator does not shadow any built-in viem public action', () => {
    const base = createPublicClient({ chain: xdevsDevnet, transport: custom({ request: async () => null }) })
    const added = Object.keys(xdevsActions(base))
    assert.equal(added.length, 10)
    for (const key of added) assert.equal(key in base, false, `${key} collides with viem`)
  })

  it('simulateTransaction formats the tx, defaults to latest, parses gasUsed', async () => {
    const { client, calls } = mockClient(() => simulationResponse)
    const res = await client.simulateTransaction({ account: ALICE, to: BOB, value: parseXds('12.5'), gas: 21000n })
    assert.deepEqual(calls, [
      {
        method: 'xdevs_simulateTransaction',
        params: [{ from: ALICE, to: BOB, value: '0xad78ebc5ac620000', gas: '0x5208' }, 'latest'],
      },
    ])
    assert.equal(res.gasUsed, 21000n)
    assert.equal(res.success, true)
    assert.equal(res.warnings[0]!.code, 'UNLIMITED_APPROVAL')
    assert.equal(res.approvals[0]!.amount, 'unlimited')
    assert.equal(res.assetChanges[0]!.token, 'native')
    assert.match(res.disclaimer, /advisory/)
  })

  it('simulateTransaction uses the client account and blockNumber', async () => {
    const account = privateKeyToAccount('0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d')
    const calls: { method: string; params?: unknown }[] = []
    const client = createClient({
      account,
      chain: xdevsDevnet,
      transport: custom({ request: async (c: { method: string; params?: unknown }) => (calls.push(c), simulationResponse) }),
    }).extend(xdevsActions)
    await client.simulateTransaction({ to: BOB, data: '0xdeadbeef', blockNumber: 255n })
    assert.deepEqual(calls[0]!.params, [{ from: account.address, to: BOB, data: '0xdeadbeef' }, '0xff'])
  })

  it('checkRecipient', async () => {
    const answer = { firstTime: true, lookAlikeOf: [BOB], name: null, isContract: false }
    const { client, calls } = mockClient(() => answer)
    assert.deepEqual(await client.checkRecipient({ from: ALICE, to: BOB }), answer)
    assert.deepEqual(calls, [{ method: 'xdevs_checkRecipient', params: [ALICE, BOB] }])
    await assert.rejects(client.checkRecipient({ from: ALICE, to: '0x1234' }), /not a valid address/)
  })

  it('getApprovals', async () => {
    const answer = [{ kind: 'erc20', token: USDC, symbol: 'USDC', spender: BOB, amount: 'unlimited', expires: null }]
    const { client, calls } = mockClient(() => answer)
    assert.deepEqual(await client.getApprovals({ owner: ALICE, blockTag: 'safe' }), answer)
    assert.deepEqual(calls, [{ method: 'xdevs_getApprovals', params: [ALICE, 'safe'] }])
  })

  it('getVaultQueue parses quantities', async () => {
    const { client, calls } = mockClient(() => [
      { id: '0x1', target: BOB, value: '0xde0b6b3a7640000', data: '0x', executeAfter: '0x6553f100' },
    ])
    const queue = await client.getVaultQueue({ vault: ALICE })
    assert.deepEqual(calls, [{ method: 'xdevs_getVaultQueue', params: [ALICE, 'latest'] }])
    assert.deepEqual(queue, [{ id: 1n, target: BOB, value: 10n ** 18n, data: '0x', executeAfter: 1700000000n }])
  })

  it('resolveXnsName normalizes the name and checksums the answer', async () => {
    const { client, calls } = mockClient(({ params }) =>
      (params as string[])[0] === 'alice.xdevs' ? ALICE.toLowerCase() : null,
    )
    assert.equal(await client.resolveXnsName({ name: '  Alice ' }), ALICE)
    assert.equal(await client.resolveXnsName({ name: 'nobody.xdevs' }), null)
    assert.deepEqual(calls.map((c) => c.params), [['alice.xdevs'], ['nobody.xdevs']])
    await assert.rejects(client.resolveXnsName({ name: 'al' }), InvalidXnsNameError)
    assert.equal(calls.length, 2, 'invalid names must not reach the node')
  })

  it('reverseXnsName', async () => {
    const { client, calls } = mockClient(() => 'alice.xdevs')
    assert.equal(await client.reverseXnsName({ address: ALICE }), 'alice.xdevs')
    assert.deepEqual(calls, [{ method: 'xdevs_reverseName', params: [ALICE] }])
  })

  it('getLocalFee', async () => {
    const { client, calls } = mockClient(() => ({ contract: USDC, globalBaseFee: '0x3b9aca00', localBaseFee: '0x5f5e100' }))
    const fee = await client.getLocalFee({ contract: USDC })
    assert.deepEqual(calls, [{ method: 'xdevs_localFee', params: [USDC, 'latest'] }])
    assert.deepEqual(fee, { contract: USDC, globalBaseFee: 1_000_000_000n, localBaseFee: 100_000_000n, totalBaseFee: 1_100_000_000n })
  })

  it('getValidators', async () => {
    const v = {
      operator: ALICE,
      consensusKey: `0x${'ab'.repeat(32)}`,
      status: 'active',
      votingPower: '0xbebc20',
      totalStake: '0x21e19e0c9bab2400000',
      commissionBps: 500,
      uptimeBps: 9990,
    }
    const { client, calls } = mockClient(() => [v])
    const [val] = await client.getValidators()
    assert.deepEqual(calls, [{ method: 'xdevs_validators', params: ['latest'] }])
    assert.equal(val!.votingPower, 12_500_000n)
    assert.equal(val!.totalStake, parseXds('10000'))
    assert.equal(val!.uptimeBps, 9990)
    assert.equal(val!.status, 'active')
  })

  it('getCommit', async () => {
    const cert = {
      height: '0x2a',
      round: '0x1',
      blockHash: `0x${'11'.repeat(32)}`,
      signatures: [{ validator: `0x${'33'.repeat(20)}`, signature: `0x${'22'.repeat(64)}` }],
      raw: '0xc0',
    }
    const { client, calls } = mockClient(({ params }) => ((params as string[])[0] === '0x2a' ? cert : null))
    const res = await client.getCommit({ height: 42n })
    assert.equal(res!.height, 42n)
    assert.equal(res!.round, 1)
    assert.equal(res!.signatures.length, 1)
    assert.equal(await client.getCommit({ height: 7 }), null)
    assert.deepEqual(calls.map((c) => c.params), [['0x2a'], ['0x7']])
    assert.ok(calls.every((c) => c.method === 'xdevs_getCommitCertificate'))
  })

  it('getEpochInfo', async () => {
    const { client, calls } = mockClient(() => ({
      epoch: '0x3',
      epochLength: '0xe10',
      startBlock: '0x2a30',
      nextTransitionBlock: '0x3840',
      validatorSetHash: `0x${'33'.repeat(32)}`,
    }))
    const info = await client.getEpochInfo({ blockTag: 'finalized' })
    assert.deepEqual(calls, [{ method: 'xdevs_epochInfo', params: ['finalized'] }])
    assert.deepEqual(info, {
      epoch: 3n,
      epochLength: 3600n,
      startBlock: 10800n,
      nextTransitionBlock: 14400n,
      validatorSetHash: `0x${'33'.repeat(32)}`,
    })
  })

  it('rejects malformed quantities from the node', async () => {
    const { client } = mockClient(() => ({ ...simulationResponse, gasUsed: 'lots' }))
    await assert.rejects(client.simulateTransaction({ account: ALICE, to: BOB }), /gasUsed/)
  })
})

describe('method-not-found handling', () => {
  const invocations = {
    xdevs_simulateTransaction: (c: ReturnType<typeof mockClient>['client']) => c.simulateTransaction({ account: ALICE, to: BOB }),
    xdevs_checkRecipient: (c: ReturnType<typeof mockClient>['client']) => c.checkRecipient({ from: ALICE, to: BOB }),
    xdevs_getApprovals: (c: ReturnType<typeof mockClient>['client']) => c.getApprovals({ owner: ALICE }),
    xdevs_getVaultQueue: (c: ReturnType<typeof mockClient>['client']) => c.getVaultQueue({ vault: ALICE }),
    xdevs_resolveName: (c: ReturnType<typeof mockClient>['client']) => c.resolveXnsName({ name: 'alice' }),
    xdevs_reverseName: (c: ReturnType<typeof mockClient>['client']) => c.reverseXnsName({ address: ALICE }),
    xdevs_localFee: (c: ReturnType<typeof mockClient>['client']) => c.getLocalFee({ contract: USDC }),
    xdevs_validators: (c: ReturnType<typeof mockClient>['client']) => c.getValidators(),
    xdevs_getCommitCertificate: (c: ReturnType<typeof mockClient>['client']) => c.getCommit({ height: 1n }),
    xdevs_epochInfo: (c: ReturnType<typeof mockClient>['client']) => c.getEpochInfo(),
  } satisfies Record<keyof typeof XDEVS_RPC_METHODS, unknown>

  for (const [method, invoke] of Object.entries(invocations)) {
    const { phase } = XDEVS_RPC_METHODS[method as keyof typeof XDEVS_RPC_METHODS]
    it(`${method} → XdevsMethodNotAvailableError (Phase ${phase})`, async () => {
      const { client, calls } = mockClient(({ method: m }) => {
        throw rpcError(-32601, `the method ${m} does not exist/is not available`)
      })
      await assert.rejects(invoke(client), (err: unknown) => {
        assert.ok(err instanceof XdevsMethodNotAvailableError)
        assert.equal(err.method, method)
        assert.equal(err.phase, phase)
        assert.match(err.message, new RegExp(`does not implement \`${method}\``))
        assert.match(err.message, new RegExp(`Phase ${phase}`))
        return true
      })
      assert.equal(calls.length, 1, 'must not retry')
    })
  }

  it('phases match plan §19.2', () => {
    assert.deepEqual(
      Object.fromEntries(Object.entries(XDEVS_RPC_METHODS).map(([m, i]) => [m, i.phase])),
      {
        xdevs_getCommitCertificate: 2,
        xdevs_validators: 3,
        xdevs_epochInfo: 3,
        xdevs_simulateTransaction: 4,
        xdevs_checkRecipient: 4,
        xdevs_getApprovals: 4,
        xdevs_getVaultQueue: 4,
        xdevs_resolveName: 4,
        xdevs_reverseName: 4,
        xdevs_localFee: 5,
      },
    )
  })

  it('also maps -32004 and message-only variants, but not other errors', async () => {
    const { client } = mockClient(() => {
      throw rpcError(-32004, 'Method not supported')
    })
    await assert.rejects(client.getValidators(), XdevsMethodNotAvailableError)

    assert.equal(isMethodNotFoundError(rpcError(-32603, 'Method xdevs_validators not found')), true)
    assert.equal(isMethodNotFoundError(rpcError(-32000, 'execution reverted: method not found')), false)
    assert.equal(isMethodNotFoundError(new Error('method not found')), false)

    const other = mockClient(() => {
      throw rpcError(-32602, 'invalid params')
    })
    await assert.rejects(other.client.getValidators(), (err: unknown) => {
      assert.equal(err instanceof XdevsMethodNotAvailableError, false)
      assert.match(String((err as Error).message), /invalid params|Invalid parameters/i)
      return true
    })
  })
})

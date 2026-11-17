// Data layer: indexer first, node RPC fallback, and the "network down" signal. fetch is mocked; no network.

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { config } from '../src/lib/config.ts'
import { loadBlock, loadTx, NotFoundError } from '../src/lib/data.ts'
import { getNet, isUnreachable, networkDown, setNet } from '../src/lib/net.ts'

const Z32 = `0x${'0'.repeat(64)}`
const block = (n: number) => ({
  number: `0x${n.toString(16)}`, hash: `0x${n.toString(16).padStart(64, 'a')}`, parentHash: Z32, timestamp: '0x6a000000', miner: `0x${'1'.repeat(40)}`,
  gasUsed: '0x5208', gasLimit: '0x3938700', baseFeePerGas: '0x3b9aca00', size: '0x220', transactions: [], uncles: [], nonce: '0x0000000000000000',
  difficulty: '0x0', totalDifficulty: '0x0', extraData: '0x', logsBloom: `0x${'0'.repeat(512)}`, mixHash: Z32, receiptsRoot: Z32, sha3Uncles: Z32, stateRoot: Z32, transactionsRoot: Z32,
})

type Rpc = { id: number; method: string; params: unknown[] }
function mockFetch({ api, rpc }: { api: 'down' | 'up'; rpc: 'down' | ((r: Rpc) => unknown) }) {
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input instanceof Request ? input.url : input)
    if (url.startsWith(config.apiUrl)) {
      if (api === 'down') throw new TypeError('Failed to fetch')
      return new Response(JSON.stringify({ error: { code: 'NOT_FOUND', message: 'not indexed' } }), { status: 404 })
    }
    if (rpc === 'down') throw new TypeError('Failed to fetch')
    const body = JSON.parse(String(init?.body)) as Rpc | Rpc[]
    const one = (r: Rpc) => {
      const result = rpc(r)
      return result instanceof Error ? { jsonrpc: '2.0', id: r.id, error: { code: -32601, message: result.message } } : { jsonrpc: '2.0', id: r.id, result }
    }
    return new Response(JSON.stringify(Array.isArray(body) ? body.map(one) : one(body)), { headers: { 'content-type': 'application/json' } })
  }) as typeof fetch
}

const node = (r: Rpc) => {
  if (r.method === 'eth_getBlockByNumber') return block(Number(BigInt(r.params[0] as string)))
  if (r.method === 'eth_getBlockReceipts') return []
  if (r.method === 'xdevs_getCommitCertificate') return new Error('the method xdevs_getCommitCertificate does not exist/is not available')
  if (r.method === 'eth_getTransactionByHash') return null
  return new Error(`unexpected ${r.method}`)
}

test('indexer down: the block comes from the node, labelled as RPC', async () => {
  setNet({ api: 'unknown', rpc: 'unknown' })
  mockFetch({ api: 'down', rpc: node })
  const r = await loadBlock('5')
  assert.equal(r.source, 'rpc')
  assert.equal(r.block.number, 5)
  assert.equal(r.block.fees.priority, '0x0', 'receipts present (empty block): tips computed')
  assert.equal(r.commit, null, 'nodes without xdevs_getCommitCertificate give no certificate, not an error')
  assert.equal(getNet().api, 'down')
  assert.equal(getNet().rpc, 'up')
  assert.equal(networkDown(getNet()), false)
})

test('indexer has not indexed it yet (404): falls back to the node too', async () => {
  setNet({ api: 'unknown', rpc: 'unknown' })
  mockFetch({ api: 'up', rpc: node })
  const r = await loadBlock('7')
  assert.equal(r.source, 'rpc')
  assert.equal(r.block.number, 7)
})

test('unknown transaction is a clear not-found, not a crash', async () => {
  mockFetch({ api: 'down', rpc: node })
  await assert.rejects(loadTx(`0x${'ab'.repeat(32)}`), NotFoundError)
  await assert.rejects(loadTx('0x1234'), NotFoundError)
})

test('both down: errors are classified as unreachable and the app shows the launch state', async () => {
  setNet({ api: 'unknown', rpc: 'unknown' })
  mockFetch({ api: 'down', rpc: 'down' })
  await assert.rejects(loadBlock('5'), (e: unknown) => isUnreachable(e))
  assert.equal(getNet().rpc, 'down')
  assert.equal(getNet().api, 'down')
  assert.equal(networkDown(getNet()), true)
})

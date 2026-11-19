// HTTP layer tests with a stub bundler (no chain): limits, rate limiting, client IPs behind Cloudflare, CORS, health.
import assert from 'node:assert/strict'
import type { Server } from 'node:http'
import { after, describe, it } from 'node:test'
import { RpcCodes } from '../src/errors.ts'
import { Metrics } from '../src/metrics.ts'
import { type BundlerLike, type ServerOptions, createBundlerServer } from '../src/server.ts'

const servers: Server[] = []
after(() => {
  for (const s of servers) s.close()
})

async function serve(o: ServerOptions = {}, healthy = true) {
  const seen: (string | undefined)[] = []
  const metrics = new Metrics()
  const stub: BundlerLike = {
    entryPoint: '0x433709009B8330FDa32311DF1C2AFA402eD8D009',
    chainId: 717171,
    metrics,
    handle: async (method: string, _params: unknown[], ctx?: { ip?: string }) => {
      seen.push(ctx?.ip)
      if (method === 'boom') throw new Error('fetch failed: http://10.0.0.5:8545 secret-internal-detail')
      return 'ok'
    },
    health: () => ({ ok: healthy, body: { status: healthy ? 'ok' : 'degraded' } }),
    metricsText: () => metrics.render(),
  } as unknown as BundlerLike
  const server = createBundlerServer(stub, o)
  servers.push(server)
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r))
  const url = `http://127.0.0.1:${(server.address() as { port: number }).port}`
  const post = (body: unknown, headers: Record<string, string> = {}) =>
    fetch(url, { method: 'POST', headers: { 'content-type': 'application/json', ...headers }, body: typeof body === 'string' ? body : JSON.stringify(body) })
  return { url, post, seen }
}

const J = async (p: Promise<Response>): Promise<any> => (await p).json()
const call = (method: string, id = 1) => ({ jsonrpc: '2.0', id, method, params: [] })

describe('bundler HTTP server', () => {
  it('limits body size and batch length', async () => {
    const { post } = await serve({ maxBodyBytes: 2_048, maxBatch: 3 })
    const big = await post({ jsonrpc: '2.0', id: 1, method: 'eth_chainId', params: ['x'.repeat(5_000)] })
    assert.equal(big.status, 413)
    const batch = await J(post([call('eth_chainId', 1), call('eth_chainId', 2), call('eth_chainId', 3), call('eth_chainId', 4)]))
    assert.equal(batch.error.code, RpcCodes.invalidRequest)
    const ok = await J(post([call('eth_chainId', 1), call('eth_chainId', 2)]))
    assert.deepEqual(ok.map((r: { result: string }) => r.result), ['ok', 'ok'])
    const bad = await J(post('{not json'))
    assert.equal(bad.error.code, RpcCodes.parseError)
  })

  it('rate-limits per client IP, using CF-Connecting-IP only when trusted', async () => {
    const { post, seen } = await serve({ rateLimitRps: 1, rateLimitBurst: 2, trustProxy: 'cloudflare' })
    const a = { 'cf-connecting-ip': '203.0.113.1' }
    assert.equal((await post(call('eth_chainId'), a)).status, 200)
    assert.equal((await post(call('eth_chainId'), a)).status, 200)
    const limited = await post(call('eth_chainId'), a)
    assert.equal(limited.status, 429)
    assert.ok(Number(limited.headers.get('retry-after')) >= 1)
    // Another client behind the same tunnel is not affected.
    assert.equal((await post(call('eth_chainId'), { 'cf-connecting-ip': '203.0.113.2' })).status, 200)
    assert.deepEqual(seen, ['203.0.113.1', '203.0.113.1', '203.0.113.2'])

    const direct = await serve({ trustProxy: 'none' })
    await direct.post(call('eth_chainId'), a)
    assert.deepEqual(direct.seen, ['127.0.0.1'], 'without TRUST_PROXY the header is ignored')
  })

  it('has stricter per-IP limits for eth_sendUserOperation and eth_estimateUserOperationGas', async () => {
    const { post } = await serve({ sendPerMinute: 4, estimatePerMinute: 4 })
    const r1 = await J(post(call('eth_sendUserOperation')))
    assert.equal(r1.result, 'ok')
    const r2 = await J(post(call('eth_sendUserOperation')))
    assert.equal(r2.error.code, RpcCodes.limitExceeded)
    assert.equal((await J(post(call('eth_estimateUserOperationGas')))).result, 'ok')
    assert.equal((await J(post(call('eth_estimateUserOperationGas')))).error.code, RpcCodes.limitExceeded)
  })

  it('CORS: echoes allowed origins only', async () => {
    const { post, url } = await serve({ corsOrigins: ['https://wallet.xdevschain.tech'] })
    const ok = await post(call('eth_chainId'), { origin: 'https://wallet.xdevschain.tech' })
    assert.equal(ok.headers.get('access-control-allow-origin'), 'https://wallet.xdevschain.tech')
    const other = await post(call('eth_chainId'), { origin: 'https://evil.example' })
    assert.equal(other.headers.get('access-control-allow-origin'), null)
    const pre = await fetch(url, { method: 'OPTIONS', headers: { origin: 'https://wallet.xdevschain.tech' } })
    assert.equal(pre.status, 204)
  })

  it('never echoes internal error details; serves /health, /metrics and /', async () => {
    const { post, url } = await serve({}, false)
    const r = await J(post(call('boom')))
    assert.equal(r.error.code, RpcCodes.internal)
    assert.doesNotMatch(r.error.message, /10\.0\.0\.5|secret/)
    assert.equal((await fetch(`${url}/health`)).status, 503)
    assert.equal((await fetch(`${url}/metrics`, { headers: { 'cf-connecting-ip': '203.0.113.9' } })).status, 404, 'not through the tunnel')
    const metrics = await (await fetch(`${url}/metrics`)).text()
    assert.match(metrics, /bundler_rpc_requests_total\{method="other"\} 1/)
    const info = await J(fetch(url))
    assert.equal(info.chainId, 717171)
    assert.equal((await fetch(`${url}/nope`)).status, 404)
  })
})

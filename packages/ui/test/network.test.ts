import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { getAddChainParameters, xdevsEnclave } from '@xdevs/sdk'
import { cx, formatInt, truncateHex } from '../src/format.ts'
import { ENCLAVE, enclaveAddChainParameters, probeRpc } from '../src/network.ts'

describe('ENCLAVE constants', () => {
  it('match xdevsEnclave from @xdevs/sdk', () => {
    assert.equal(ENCLAVE.chainId, xdevsEnclave.id)
    assert.equal(ENCLAVE.chainIdHex, `0x${xdevsEnclave.id.toString(16)}`)
    assert.equal(ENCLAVE.name, xdevsEnclave.name)
    assert.deepEqual(ENCLAVE.nativeCurrency, xdevsEnclave.nativeCurrency)
    assert.equal(ENCLAVE.rpcUrl, xdevsEnclave.rpcUrls.default.http[0])
    assert.equal(ENCLAVE.wsUrl, xdevsEnclave.rpcUrls.default.webSocket[0])
    assert.equal(ENCLAVE.explorerUrl, xdevsEnclave.blockExplorers.default.url)
  })

  it('builds the same EIP-3085 parameters as the SDK (plus an icon)', () => {
    const { iconUrls, ...rest } = enclaveAddChainParameters()
    assert.deepEqual(rest, getAddChainParameters(xdevsEnclave))
    assert.equal(iconUrls?.length, 1)
  })
})

describe('probeRpc', () => {
  const reply = (body: unknown, ok = true) =>
    (async () => ({ ok, json: async () => body })) as unknown as typeof fetch

  it('is live when eth_chainId matches', async () => {
    assert.equal(await probeRpc('https://rpc.test', { fetchImpl: reply({ result: '0xaf173' }) }), 'live')
  })
  it('is degraded on a wrong chain or an error object', async () => {
    assert.equal(await probeRpc('https://rpc.test', { fetchImpl: reply({ result: '0x1' }) }), 'degraded')
    assert.equal(await probeRpc('https://rpc.test', { fetchImpl: reply({ error: { code: -32000 } }) }), 'degraded')
  })
  it('is down on HTTP or network failure', async () => {
    assert.equal(await probeRpc('https://rpc.test', { fetchImpl: reply({}, false) }), 'down')
    const fail = (async () => {
      throw new TypeError('fetch failed')
    }) as unknown as typeof fetch
    assert.equal(await probeRpc('https://rpc.test', { fetchImpl: fail }), 'down')
  })
})

describe('format', () => {
  it('truncates hex', () => {
    assert.equal(truncateHex('0x1234567890abcdef1234567890abcdef12345678'), '0x1234…5678')
    assert.equal(truncateHex('0x1234567890abcdef', 6, 6), '0x123456…abcdef')
    assert.equal(truncateHex('0x12345678'), '0x12345678')
    assert.equal(truncateHex(''), '')
  })
  it('joins classes and formats integers', () => {
    assert.equal(cx('a', false, undefined, 'b'), 'a b')
    assert.equal(formatInt(1234567n), '1,234,567')
    assert.equal(formatInt('nope'), 'nope')
  })
})

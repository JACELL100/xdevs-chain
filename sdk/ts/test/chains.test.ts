import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { createWalletClient, custom, defineChain, getAddress } from 'viem'
import {
  MULTICALL3_ADDRESS,
  XDEVS_ENCLAVE_CHAIN_ID,
  XDEVS_ENCLAVE_ENDPOINTS,
  XDEVS_TESTNET_CHAIN_ID,
  addXdevsChainToWallet,
  defineXdevsDevnet,
  getAddChainParameters,
  predeploys,
  xdevsChains,
  xdevsDevnet,
  xdevsEnclave,
  xdevsLocal,
  xdevsTestnet,
} from '../src/index.ts'

describe('chains', () => {
  it('local matches the `xdevs run --dev` defaults', () => {
    assert.equal(xdevsLocal.id, 7117)
    assert.equal(xdevsLocal.name, 'xDEVS Chain Local')
    assert.deepEqual(xdevsLocal.nativeCurrency, { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 })
    assert.deepEqual(xdevsLocal.rpcUrls.default.http, ['http://127.0.0.1:8545'])
    assert.deepEqual(xdevsLocal.rpcUrls.default.webSocket, ['ws://127.0.0.1:8545'])
    assert.equal(xdevsLocal.testnet, true)
    assert.equal(xdevsLocal.blockExplorers, undefined)
  })

  it('Enclave is chain 717171 = 0xAF173 with EnclaveXDS and the xdevschain.tech endpoints', () => {
    assert.equal(xdevsEnclave.id, 717171)
    assert.equal(xdevsEnclave.id, 0xaf173)
    assert.equal(XDEVS_ENCLAVE_CHAIN_ID, 717171)
    assert.equal(xdevsEnclave.name, 'xDEVS Chain Enclave')
    assert.deepEqual(xdevsEnclave.nativeCurrency, { name: 'EnclaveXDS', symbol: 'tXDS', decimals: 18 })
    assert.deepEqual(xdevsEnclave.rpcUrls.default.http, ['https://rpc.xdevschain.tech'])
    assert.deepEqual(xdevsEnclave.rpcUrls.default.webSocket, ['wss://ws.xdevschain.tech'])
    assert.equal(xdevsEnclave.blockExplorers.default.url, 'https://explorer.xdevschain.tech')
    assert.equal(xdevsEnclave.blockExplorers.default.url, XDEVS_ENCLAVE_ENDPOINTS.explorer)
    assert.equal(xdevsEnclave.testnet, true)
    for (const url of Object.values(XDEVS_ENCLAVE_ENDPOINTS)) assert.ok(/(^|\.)xdevschain\.tech$/.test(new URL(url).hostname), url)
  })

  it('keeps xdevsTestnet / XDEVS_TESTNET_CHAIN_ID as deprecated aliases of Enclave', () => {
    assert.equal(xdevsTestnet, xdevsEnclave)
    assert.equal(XDEVS_TESTNET_CHAIN_ID, XDEVS_ENCLAVE_CHAIN_ID)
  })

  it('devnet is chain 717170 without a public RPC until one is configured', () => {
    assert.equal(xdevsDevnet.id, 717170)
    assert.equal(xdevsDevnet.name, 'xDEVS Chain Devnet')
    assert.equal(xdevsDevnet.nativeCurrency.symbol, 'tXDS')
    assert.deepEqual(xdevsDevnet.rpcUrls.default.http, [])
    const dev = defineXdevsDevnet({ rpcUrl: 'https://devnet.example.org', wsUrl: 'wss://devnet.example.org' })
    assert.equal(dev.id, 717170)
    assert.deepEqual(dev.rpcUrls.default.http, ['https://devnet.example.org'])
    assert.deepEqual(dev.rpcUrls.default.webSocket, ['wss://devnet.example.org'])
    assert.equal(dev.blockExplorers, undefined)
    const withExplorer = defineXdevsDevnet({ rpcUrl: 'https://devnet.example.org', explorerUrl: 'https://x.example.org' })
    assert.equal(withExplorer.blockExplorers?.default.url, 'https://x.example.org')
  })

  it('has unique chain ids', () => {
    assert.equal(new Set(xdevsChains.map((c) => c.id)).size, xdevsChains.length)
  })

  it('exposes multicall3 at the canonical address and every predeploy as a contract', () => {
    for (const chain of xdevsChains) {
      assert.equal(chain.contracts.multicall3.address, '0xcA11bde05977b3631167028862bE2a173976CA11')
      assert.equal(chain.contracts.multicall3.address, MULTICALL3_ADDRESS)
      assert.equal(chain.contracts.multicall3.blockCreated, 0)
      assert.equal(chain.contracts.staking.address, predeploys.STAKING)
      assert.equal(chain.contracts.xnsRegistry.address, predeploys.XNS_REGISTRY)
      assert.equal(chain.contracts.wxds.address, predeploys.WXDS)
    }
  })
})

describe('predeploys', () => {
  it('match contracts/src/common/Predeploys.sol exactly', () => {
    const sol = readFileSync(
      new URL('../../../contracts/src/common/Predeploys.sol', import.meta.url),
      'utf8',
    )
    const fromSol: Record<string, string> = {}
    for (const m of sol.matchAll(/address\s+internal\s+constant\s+(\w+)\s*=\s*(0x[0-9a-fA-F]{40})\s*;/g))
      fromSol[m[1]!] = m[2]!
    assert.ok(Object.keys(fromSol).length >= 12, 'parsed too few constants from Predeploys.sol')
    assert.deepEqual(Object.keys(predeploys).sort(), Object.keys(fromSol).sort())
    for (const [name, address] of Object.entries(fromSol)) {
      assert.equal(predeploys[name as keyof typeof predeploys], address, name)
      assert.equal(getAddress(address), address, `${name} must be checksummed`)
    }
  })

  it('has the §18 layout (0x…1000 – 0x…1009)', () => {
    assert.equal(predeploys.STAKING, '0x0000000000000000000000000000000000001000')
    assert.equal(predeploys.WXDS, '0x0000000000000000000000000000000000001009')
    assert.equal(predeploys.P256_VERIFY, '0x0000000000000000000000000000000000000100')
    assert.equal(predeploys.SYSTEM_ADDRESS.toLowerCase(), '0xfffffffffffffffffffffffffffffffffffffffe')
  })
})

describe('wallet_addEthereumChain (§23.1)', () => {
  it('builds the plan §23.1 parameters', () => {
    assert.deepEqual(getAddChainParameters(xdevsEnclave), {
      chainId: '0xaf173',
      chainName: 'xDEVS Chain Enclave',
      nativeCurrency: { name: 'EnclaveXDS', symbol: 'tXDS', decimals: 18 },
      rpcUrls: ['https://rpc.xdevschain.tech'],
      blockExplorerUrls: ['https://explorer.xdevschain.tech'],
    })
    const local = getAddChainParameters(xdevsLocal, { iconUrls: ['https://example.org/xds.svg'] })
    assert.equal(local.chainId, '0x1bcd')
    assert.equal('blockExplorerUrls' in local, false)
    assert.deepEqual(local.iconUrls, ['https://example.org/xds.svg'])
  })

  function mockWallet() {
    const calls: { method: string; params: unknown }[] = []
    const wallet = createWalletClient({
      transport: custom({
        async request({ method, params }: { method: string; params?: unknown }) {
          calls.push({ method, params })
          return null
        },
      }),
    })
    return { wallet, calls }
  }

  it('sends wallet_addEthereumChain once', async () => {
    const { wallet, calls } = mockWallet()
    await addXdevsChainToWallet(wallet, xdevsEnclave)
    assert.deepEqual(calls, [{ method: 'wallet_addEthereumChain', params: [getAddChainParameters(xdevsEnclave)] }])
  })

  it('refuses chains without an RPC or with a placeholder URL', async () => {
    const { wallet, calls } = mockWallet()
    await assert.rejects(addXdevsChainToWallet(wallet, xdevsDevnet), /no RPC URL/)
    const placeholder = defineChain({ ...xdevsEnclave, rpcUrls: { default: { http: ['https://rpc.testnet.xdevs.invalid'] } } })
    await assert.rejects(addXdevsChainToWallet(wallet, placeholder), /placeholder URL/)
    assert.equal(calls.length, 0)
    await addXdevsChainToWallet(wallet, defineXdevsDevnet({ rpcUrl: 'https://devnet.example.org' }))
    assert.equal(calls.length, 1)
  })
})

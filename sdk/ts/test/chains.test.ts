import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { createWalletClient, custom, defineChain, getAddress } from 'viem'
import {
  MULTICALL3_ADDRESS,
  XDEVS_TESTNET_IS_PLACEHOLDER,
  addXdevsChainToWallet,
  getAddChainParameters,
  predeploys,
  xdevsChains,
  xdevsDevnet,
  xdevsTestnet,
} from '../src/index.ts'

describe('chains', () => {
  it('devnet matches the local node defaults', () => {
    assert.equal(xdevsDevnet.id, 7117)
    assert.equal(xdevsDevnet.name, 'xDEVS Chain Devnet')
    assert.deepEqual(xdevsDevnet.nativeCurrency, { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 })
    assert.deepEqual(xdevsDevnet.rpcUrls.default.http, ['http://127.0.0.1:8545'])
    assert.deepEqual(xdevsDevnet.rpcUrls.default.webSocket, ['ws://127.0.0.1:8546'])
    assert.equal(xdevsDevnet.testnet, true)
    assert.equal(xdevsDevnet.blockExplorers, undefined)
  })

  it('testnet uses the (placeholder) id 717171 = 0xAF173 and clearly marked placeholder URLs', () => {
    assert.equal(xdevsTestnet.id, 717171)
    assert.equal(xdevsTestnet.id, 0xaf173)
    assert.equal(XDEVS_TESTNET_IS_PLACEHOLDER, true)
    assert.deepEqual(xdevsTestnet.nativeCurrency, { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 })
    assert.match(xdevsTestnet.rpcUrls.default.http[0]!, /^https:\/\/rpc\.testnet\..*\.invalid$/)
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
    assert.deepEqual(getAddChainParameters(xdevsTestnet), {
      chainId: '0xaf173',
      chainName: 'xDEVS Chain Testnet',
      nativeCurrency: { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 },
      rpcUrls: ['https://rpc.testnet.xdevs.invalid'],
      blockExplorerUrls: ['https://explorer.testnet.xdevs.invalid'],
    })
    const devnet = getAddChainParameters(xdevsDevnet, { iconUrls: ['https://example.org/xds.svg'] })
    assert.equal(devnet.chainId, '0x1bcd')
    assert.equal('blockExplorerUrls' in devnet, false)
    assert.deepEqual(devnet.iconUrls, ['https://example.org/xds.svg'])
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
    await addXdevsChainToWallet(wallet, xdevsDevnet)
    assert.deepEqual(calls, [{ method: 'wallet_addEthereumChain', params: [getAddChainParameters(xdevsDevnet)] }])
  })

  it('refuses the placeholder testnet but accepts an overridden one', async () => {
    const { wallet, calls } = mockWallet()
    await assert.rejects(addXdevsChainToWallet(wallet, xdevsTestnet), /placeholder URL/)
    assert.equal(calls.length, 0)
    const real = defineChain({
      ...xdevsTestnet,
      rpcUrls: { default: { http: ['https://rpc.testnet.example.org'] } },
      blockExplorers: { default: { name: 'Explorer', url: 'https://explorer.testnet.example.org' } },
    })
    await addXdevsChainToWallet(wallet, real)
    assert.equal(calls.length, 1)
  })
})

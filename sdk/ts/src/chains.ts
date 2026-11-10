import { type Chain, defineChain } from 'viem'
import { MULTICALL3_ADDRESS, predeploys } from './predeploys.ts'

/** Native currency of every pre-mainnet xDEVS network (plan §1.4, §20.1). 1 tXDS = 10^18 dev. */
export const testnetXds = { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 } as const

/** Chain id of a local single-node devnet (`xdevs-primitives::DEV_CHAIN_ID`). */
export const XDEVS_DEVNET_CHAIN_ID = 7117

/**
 * Chain id of the public testnet.
 *
 * TODO(chain-id): 717171 (0xAF173) is the PLACEHOLDER from plan §21.1 / §23.1. It is NOT yet registered
 * on chainlist.org / ethereum-lists/chains. Replace it once the id is registered (plan §8.5).
 */
export const XDEVS_TESTNET_CHAIN_ID = 717171

/**
 * TODO(domain): the testnet domain is not chosen yet (plan §1.4 "Secure a domain").
 * `.invalid` is reserved by RFC 2606, so requests fail fast instead of hitting a random host.
 */
export const XDEVS_TESTNET_PLACEHOLDER_DOMAIN = 'testnet.xdevs.invalid'

/** `true` while `xdevsTestnet` still uses the placeholder chain id and domain above. */
export const XDEVS_TESTNET_IS_PLACEHOLDER = true

/** A viem `Chain` narrowed with the literal config of an xDEVS network. */
export type XdevsChain<config extends Chain = Chain> = Chain<undefined> & config

/** Genesis predeploys exposed as viem chain contracts (all deployed at block 0). */
const genesisContracts = {
  multicall3: { address: MULTICALL3_ADDRESS, blockCreated: 0 },
  staking: { address: predeploys.STAKING, blockCreated: 0 },
  governance: { address: predeploys.GOVERNANCE, blockCreated: 0 },
  treasury: { address: predeploys.TREASURY, blockCreated: 0 },
  feeConfig: { address: predeploys.FEE_CONFIG, blockCreated: 0 },
  xnsRegistry: { address: predeploys.XNS_REGISTRY, blockCreated: 0 },
  smartAccountImpl: { address: predeploys.SMART_ACCOUNT_IMPL, blockCreated: 0 },
  smartAccountFactory: { address: predeploys.SMART_ACCOUNT_FACTORY, blockCreated: 0 },
  paymaster: { address: predeploys.PAYMASTER, blockCreated: 0 },
  permit2Expiring: { address: predeploys.PERMIT2_EXPIRING, blockCreated: 0 },
  wxds: { address: predeploys.WXDS, blockCreated: 0 },
} as const

const devnetConfig = {
  id: XDEVS_DEVNET_CHAIN_ID,
  name: 'xDEVS Chain Devnet',
  nativeCurrency: testnetXds,
  rpcUrls: {
    default: {
      http: ['http://127.0.0.1:8545'],
      webSocket: ['ws://127.0.0.1:8546'],
    },
  },
  contracts: genesisContracts,
  blockTime: 2_000,
  testnet: true,
} as const

/** Local development node started with `xdevs run --dev` (chain id 7117). */
export const xdevsDevnet: XdevsChain<typeof devnetConfig> = /*#__PURE__*/ defineChain(devnetConfig)

const testnetConfig = {
  id: XDEVS_TESTNET_CHAIN_ID,
  name: 'xDEVS Chain Testnet',
  nativeCurrency: testnetXds,
  rpcUrls: {
    default: {
      http: [`https://rpc.${XDEVS_TESTNET_PLACEHOLDER_DOMAIN}`],
      webSocket: [`wss://ws.${XDEVS_TESTNET_PLACEHOLDER_DOMAIN}`],
    },
  },
  blockExplorers: {
    default: { name: 'xDEVS Explorer', url: `https://explorer.${XDEVS_TESTNET_PLACEHOLDER_DOMAIN}` },
  },
  contracts: genesisContracts,
  blockTime: 2_000,
  testnet: true,
} as const

/**
 * Public xDEVS Chain testnet.
 *
 * TODO: chain id (717171) and all URLs are PLACEHOLDERS — see {@link XDEVS_TESTNET_CHAIN_ID} and
 * {@link XDEVS_TESTNET_PLACEHOLDER_DOMAIN}. Until then, override them, e.g.
 * `defineChain({ ...xdevsTestnet, rpcUrls: { default: { http: ['https://…'] } } })`.
 */
export const xdevsTestnet: XdevsChain<typeof testnetConfig> = /*#__PURE__*/ defineChain(testnetConfig)

/** All xDEVS chains known to this SDK version. */
export const xdevsChains = [xdevsDevnet, xdevsTestnet] as const

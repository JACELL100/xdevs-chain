import { type Chain, defineChain } from 'viem'
import { MULTICALL3_ADDRESS, predeploys } from './predeploys.ts'

/** Native currency of local and devnet xDEVS networks (plan §1.4, §20.1). 1 tXDS = 10^18 dev. */
export const testnetXds = { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 } as const

/** Native currency of the Enclave public testnet: EnclaveXDS, symbol tXDS. No monetary value. */
export const enclaveXds = { name: 'EnclaveXDS', symbol: 'tXDS', decimals: 18 } as const

/** Chain id of a local single-node chain started with `xdevs run --dev` (`xdevs-primitives::DEV_CHAIN_ID`). */
export const XDEVS_LOCAL_CHAIN_ID = 7117

/** Chain id of the xDEVS Chain Devnet (internal multi-node test network, no public endpoint yet). */
export const XDEVS_DEVNET_CHAIN_ID = 717170

/**
 * Chain id of xDEVS Chain Enclave, the public testnet: 717171 = 0xAF173.
 * Free in the chainid.network registry (checked 2026-11-17); registration on ethereum-lists/chains is pending.
 */
export const XDEVS_ENCLAVE_CHAIN_ID = 717171

/** @deprecated Use {@link XDEVS_ENCLAVE_CHAIN_ID}. */
export const XDEVS_TESTNET_CHAIN_ID = XDEVS_ENCLAVE_CHAIN_ID

/** Public endpoints of the Enclave testnet (all under xdevschain.tech). */
export const XDEVS_ENCLAVE_ENDPOINTS = {
  rpc: 'https://rpc.xdevschain.tech',
  ws: 'wss://ws.xdevschain.tech',
  explorer: 'https://explorer.xdevschain.tech',
  explorerApi: 'https://explorer-api.xdevschain.tech',
  wallet: 'https://wallet.xdevschain.tech',
  faucet: 'https://faucet.xdevschain.tech',
  status: 'https://status.xdevschain.tech',
  bundler: 'https://bundler.xdevschain.tech',
  site: 'https://xdevschain.tech',
} as const

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

const localConfig = {
  id: XDEVS_LOCAL_CHAIN_ID,
  name: 'xDEVS Chain Local',
  nativeCurrency: testnetXds,
  rpcUrls: {
    default: {
      http: ['http://127.0.0.1:8545'],
      webSocket: ['ws://127.0.0.1:8545'],
    },
  },
  contracts: genesisContracts,
  blockTime: 2_000,
  testnet: true,
} as const

/** Local development node started with `xdevs run --dev` (chain id 7117, `http://127.0.0.1:8545`). */
export const xdevsLocal: XdevsChain<typeof localConfig> = /*#__PURE__*/ defineChain(localConfig)

const devnetConfig = {
  id: XDEVS_DEVNET_CHAIN_ID,
  name: 'xDEVS Chain Devnet',
  nativeCurrency: testnetXds,
  // No public endpoint yet: pass one with `defineXdevsDevnet({ rpcUrl })`. An empty list makes
  // viem's `http()` transport fail fast instead of calling a wrong host.
  rpcUrls: { default: { http: [] as readonly string[] } },
  contracts: genesisContracts,
  blockTime: 2_000,
  testnet: true,
} as const

/**
 * xDEVS Chain Devnet (chain id 717170). It has no public RPC yet, so this definition carries no URL:
 * use {@link defineXdevsDevnet} to point it at a node.
 */
export const xdevsDevnet: XdevsChain<typeof devnetConfig> = /*#__PURE__*/ defineChain(devnetConfig)

/** Endpoints for {@link defineXdevsDevnet}. */
export type XdevsDevnetEndpoints = { rpcUrl: string; wsUrl?: string; explorerUrl?: string }

/** {@link xdevsDevnet} pointed at the given RPC (and optional WebSocket / explorer) URL. */
export function defineXdevsDevnet({ rpcUrl, wsUrl, explorerUrl }: XdevsDevnetEndpoints): XdevsChain {
  return defineChain({
    ...devnetConfig,
    rpcUrls: { default: { http: [rpcUrl], ...(wsUrl ? { webSocket: [wsUrl] } : {}) } },
    ...(explorerUrl ? { blockExplorers: { default: { name: 'xDEVS Explorer', url: explorerUrl } } } : {}),
  })
}

const enclaveConfig = {
  id: XDEVS_ENCLAVE_CHAIN_ID,
  name: 'xDEVS Chain Enclave',
  nativeCurrency: enclaveXds,
  rpcUrls: {
    default: {
      http: [XDEVS_ENCLAVE_ENDPOINTS.rpc],
      webSocket: [XDEVS_ENCLAVE_ENDPOINTS.ws],
    },
  },
  blockExplorers: {
    default: {
      name: 'xDEVS Explorer',
      url: XDEVS_ENCLAVE_ENDPOINTS.explorer,
      apiUrl: XDEVS_ENCLAVE_ENDPOINTS.explorerApi,
    },
  },
  contracts: genesisContracts,
  blockTime: 2_000,
  testnet: true,
} as const

/** xDEVS Chain Enclave, the public testnet (chain id 717171, coin EnclaveXDS / tXDS). */
export const xdevsEnclave: XdevsChain<typeof enclaveConfig> = /*#__PURE__*/ defineChain(enclaveConfig)

/** @deprecated The public testnet is named Enclave: use {@link xdevsEnclave}. */
export const xdevsTestnet: typeof xdevsEnclave = xdevsEnclave

/** All xDEVS chains known to this SDK version. */
export const xdevsChains = [xdevsLocal, xdevsDevnet, xdevsEnclave] as const

/** Site-wide constants for xdevschain.tech (the product site). Docs live on docs.xdevschain.tech. */

export const SITE_URL = 'https://xdevschain.tech'
export const DOCS_URL = 'https://docs.xdevschain.tech'
export const SITE_NAME = 'xDEVS Chain'
export const TAGLINE = 'The chain that has your back.'
export const SITE_DESCRIPTION =
  'xDEVS Chain is an EVM-equivalent Proof-of-Stake Layer 1 that protects people at the protocol level: passkey accounts, a Guardian Vault, approvals that expire and .xdevs names that cannot be faked. The Enclave public testnet is live.'

/** A page on the docs site. */
export const docs = (path = '/guide/introduction') => `${DOCS_URL}${path}`

export const LINKS = {
  wallet: 'https://wallet.xdevschain.tech',
  walletDemo: 'https://wallet.xdevschain.tech/?demo=1',
  explorer: 'https://explorer.xdevschain.tech',
  faucet: 'https://faucet.xdevschain.tech',
  status: 'https://status.xdevschain.tech',
  rpcInfo: 'https://status.xdevschain.tech/rpc',
  github: 'https://github.com/JACELL100/xdevs-chain',
  releases: 'https://github.com/JACELL100/xdevs-chain/releases',
  securityPolicy: 'https://github.com/JACELL100/xdevs-chain/blob/main/SECURITY.md',
  plan: 'https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md',
  xdevs: 'https://xdevs.in',
} as const

/**
 * Enclave testnet constants. Keep in sync with packages/ui/src/network.ts (ENCLAVE and
 * enclaveAddChainParameters); this app is a standalone build and does not import the workspace.
 */
export const ENCLAVE = {
  chainId: 717171,
  chainIdHex: '0xaf173',
  name: 'xDEVS Chain Enclave',
  nativeCurrency: { name: 'EnclaveXDS', symbol: 'tXDS', decimals: 18 },
  rpcUrl: 'https://rpc.xdevschain.tech',
  wsUrl: 'wss://ws.xdevschain.tech',
  explorerUrl: 'https://explorer.xdevschain.tech',
  explorerApiUrl: 'https://explorer-api.xdevschain.tech',
  bundlerUrl: 'https://bundler.xdevschain.tech',
  iconUrl: 'https://xdevschain.tech/brand/xdevs-chain-mark.svg',
} as const

export function enclaveAddChainParameters() {
  return {
    chainId: ENCLAVE.chainIdHex,
    chainName: ENCLAVE.name,
    nativeCurrency: { ...ENCLAVE.nativeCurrency },
    rpcUrls: [ENCLAVE.rpcUrl],
    blockExplorerUrls: [ENCLAVE.explorerUrl],
    iconUrls: ['https://xdevschain.tech/brand/xdevs-chain-icon-256.png', ENCLAVE.iconUrl],
  }
}

export type NavItem = { label: string; href: string; external?: boolean }

export const NAV: NavItem[] = [
  { label: 'Enclave', href: '/enclave' },
  { label: 'Wallet', href: '/wallet' },
  { label: 'Developers', href: '/developers' },
  { label: 'Security', href: '/security' },
  { label: 'Roadmap', href: '/roadmap' },
  { label: 'Docs', href: DOCS_URL, external: true },
]

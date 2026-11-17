// Wallet configuration. Every value is PUBLIC (inlined into the static bundle at build time) and defaults to the
// public Enclave testnet. Next.js only inlines `process.env.NEXT_PUBLIC_*` when read with a literal name, so each
// variable is read explicitly below. See .env.example.
import { XDEVS_ENCLAVE_CHAIN_ID, XDEVS_ENCLAVE_ENDPOINTS, enclaveXds, xdevsEnclave } from '@xdevs/sdk'
import { type Address, createPublicClient, defineChain, getAddress, http, isAddress } from 'viem'

const ENCLAVE = {
  chainId: XDEVS_ENCLAVE_CHAIN_ID,
  name: xdevsEnclave.name,
  coinName: enclaveXds.name,
  symbol: enclaveXds.symbol,
  rpcUrl: XDEVS_ENCLAVE_ENDPOINTS.rpc,
  bundlerUrl: XDEVS_ENCLAVE_ENDPOINTS.bundler,
  explorerUrl: XDEVS_ENCLAVE_ENDPOINTS.explorer,
  apiUrl: XDEVS_ENCLAVE_ENDPOINTS.explorerApi,
  faucetUrl: XDEVS_ENCLAVE_ENDPOINTS.faucet,
  siteUrl: XDEVS_ENCLAVE_ENDPOINTS.site,
}

const str = (v: string | undefined): string | null => (v && v.trim() ? v.trim() : null)

const list = (v: string | undefined): Address[] =>
  (v ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter((s) => isAddress(s, { strict: false }))
    .map((s) => getAddress(s))

function apiUrl(v: string | undefined): string | null {
  const s = str(v)
  if (s === 'off' || s === 'none') return null
  return s ?? ENCLAVE.apiUrl
}

const chainId = Number(str(process.env.NEXT_PUBLIC_CHAIN_ID) ?? ENCLAVE.chainId)

export const config = {
  rpcUrl: str(process.env.NEXT_PUBLIC_RPC_URL) ?? ENCLAVE.rpcUrl,
  bundlerUrl: str(process.env.NEXT_PUBLIC_BUNDLER_URL) ?? ENCLAVE.bundlerUrl,
  chainId,
  chainName: str(process.env.NEXT_PUBLIC_CHAIN_NAME) ?? (chainId === 7117 ? 'xDEVS Chain Local' : chainId === 717170 ? 'xDEVS Chain Devnet' : ENCLAVE.name),
  coinName: ENCLAVE.coinName,
  symbol: ENCLAVE.symbol,
  /** Explorer API base (apps/indexer) for activity, approvals and the vault queue; "off" = chain reads only. */
  apiUrl: apiUrl(process.env.NEXT_PUBLIC_API_URL),
  explorerUrl: str(process.env.NEXT_PUBLIC_EXPLORER_URL) ?? ENCLAVE.explorerUrl,
  faucetUrl: str(process.env.NEXT_PUBLIC_FAUCET_URL) ?? ENCLAVE.faucetUrl,
  siteUrl: ENCLAVE.siteUrl,
  /** XdevsPaymaster sponsor whose deposit pays the first (deploying) UserOperation, optional. */
  paymasterSponsor: (list(process.env.NEXT_PUBLIC_PAYMASTER_SPONSOR)[0] ?? null) as Address | null,
  knownTokens: list(process.env.NEXT_PUBLIC_KNOWN_TOKENS),
  /**
   * WebAuthn relying-party id. Defaults to the page's hostname (wallet.xdevschain.tech in production, localhost in
   * development). Passkeys are bound to it: changing it later orphans existing passkeys.
   */
  rpIdOverride: str(process.env.NEXT_PUBLIC_RP_ID),
  /** Build-time demo mode: fixture account and data, no chain, no passkey. */
  demo: process.env.NEXT_PUBLIC_DEMO === '1' || process.env.NEXT_PUBLIC_DEMO === 'true',
  /** Dev-only tools (virtual passkey). Never enable on a public deployment. */
  devTools: process.env.NEXT_PUBLIC_DEV_TOOLS === 'true' || process.env.NODE_ENV === 'development',
}

export function rpId(): string {
  return config.rpIdOverride ?? (typeof location !== 'undefined' ? location.hostname : 'localhost')
}

export const chain = defineChain({
  id: config.chainId,
  name: config.chainName,
  nativeCurrency: { name: config.coinName, symbol: config.symbol, decimals: 18 },
  rpcUrls: { default: { http: [config.rpcUrl] } },
  blockExplorers: { default: { name: 'xDEVS Explorer', url: config.explorerUrl } },
  testnet: true,
})

export const publicClient = createPublicClient({ chain, transport: http(config.rpcUrl, { retryCount: 1, timeout: 10_000 }), pollingInterval: 2_000 })

export const explorerAddress = (a: string) => `${config.explorerUrl.replace(/\/$/, '')}/address/${a}`
export const explorerTx = (h: string) => `${config.explorerUrl.replace(/\/$/, '')}/tx/${h}`
export const faucetLink = (a: string) => `${config.faucetUrl}${config.faucetUrl.includes('?') ? '&' : '?'}address=${a}`

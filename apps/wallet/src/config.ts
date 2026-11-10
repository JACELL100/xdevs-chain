// Wallet configuration from Vite env (build-time). See .env.example.
import { type Address, createPublicClient, defineChain, getAddress, http, isAddress } from 'viem'

const env = import.meta.env

const list = (v: string | undefined): Address[] =>
  (v ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter((s) => isAddress(s, { strict: false }))
    .map((s) => getAddress(s))

export const config = {
  rpcUrl: (env.VITE_RPC_URL as string | undefined) ?? 'http://127.0.0.1:8545',
  bundlerUrl: (env.VITE_BUNDLER_URL as string | undefined) ?? 'http://127.0.0.1:4337',
  chainId: Number(env.VITE_CHAIN_ID ?? 7117),
  /** Explorer / indexer API base (apps/indexer), e.g. https://api.explorer.example — optional. */
  apiUrl: ((env.VITE_API_URL as string | undefined) || null) as string | null,
  faucetUrl: ((env.VITE_FAUCET_URL as string | undefined) || null) as string | null,
  explorerUrl: ((env.VITE_EXPLORER_URL as string | undefined) || null) as string | null,
  /** XdevsPaymaster sponsor whose deposit pays for the first (deploying) UserOperation — optional. */
  paymasterSponsor: (list(env.VITE_PAYMASTER_SPONSOR as string | undefined)[0] ?? null) as Address | null,
  knownTokens: list(env.VITE_KNOWN_TOKENS as string | undefined),
  /** Dev-only tools: virtual passkey, raw-key EIP-7702 panel. Never enable on a public deployment. */
  devTools: env.VITE_DEV_TOOLS === 'true' || (env.DEV as boolean),
}

export const chain = defineChain({
  id: config.chainId,
  name: config.chainId === 7117 ? 'xDEVS Devnet' : `xDEVS Chain ${config.chainId}`,
  nativeCurrency: { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 },
  rpcUrls: { default: { http: [config.rpcUrl] } },
})

export const publicClient = createPublicClient({ chain, transport: http(config.rpcUrl), pollingInterval: 1_000 })

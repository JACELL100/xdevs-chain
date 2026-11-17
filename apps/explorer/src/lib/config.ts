// Build-time configuration. Next.js inlines NEXT_PUBLIC_* at build time; every value is public.
// Each variable is read with a literal `process.env.NEXT_PUBLIC_…` expression so the bundler can inline it.

const trim = (s: string) => s.replace(/\/+$/, '')
const env = (v: string | undefined) => (v && v.trim() ? v.trim() : undefined)

import { XDEVS_ENCLAVE_ENDPOINTS, xdevsEnclave } from '@xdevs/sdk'

/** Enclave testnet defaults (from @xdevs/sdk). */
const DEFAULTS = {
  rpcUrl: XDEVS_ENCLAVE_ENDPOINTS.rpc,
  wsUrl: XDEVS_ENCLAVE_ENDPOINTS.ws,
  apiUrl: XDEVS_ENCLAVE_ENDPOINTS.explorerApi,
  chainId: xdevsEnclave.id,
  networkName: xdevsEnclave.name,
  nativeSymbol: xdevsEnclave.nativeCurrency.symbol,
  nativeName: xdevsEnclave.nativeCurrency.name,
}

export const config = {
  rpcUrl: trim(env(process.env.NEXT_PUBLIC_RPC_URL) ?? DEFAULTS.rpcUrl),
  wsUrl: env(process.env.NEXT_PUBLIC_WS_URL) ?? DEFAULTS.wsUrl,
  apiUrl: trim(env(process.env.NEXT_PUBLIC_API_URL) ?? DEFAULTS.apiUrl),
  chainId: Number(env(process.env.NEXT_PUBLIC_CHAIN_ID) ?? DEFAULTS.chainId),
  networkName: env(process.env.NEXT_PUBLIC_NETWORK_NAME) ?? DEFAULTS.networkName,
  nativeSymbol: DEFAULTS.nativeSymbol,
  nativeName: DEFAULTS.nativeName,
  demo: process.env.NEXT_PUBLIC_DEMO === '1' || process.env.NEXT_PUBLIC_DEMO === 'true',
}

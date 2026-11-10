// Build-time configuration (Vite env). Safe to import from node tests, where import.meta.env is undefined.

const env: Record<string, string | undefined> = (import.meta as { env?: Record<string, string | undefined> }).env ?? {}

const trim = (s: string) => s.replace(/\/+$/, '')

export const config = {
  apiUrl: trim(env.VITE_API_URL || 'http://127.0.0.1:8080'),
  rpcUrl: env.VITE_RPC_URL || 'http://127.0.0.1:8545',
  chainId: Number(env.VITE_CHAIN_ID || 7117),
  nativeSymbol: env.VITE_NATIVE_SYMBOL || 'tXDS',
  networkName: env.VITE_NETWORK_NAME || (Number(env.VITE_CHAIN_ID || 7117) === 7117 ? 'xDEVS Dev (local)' : 'xDEVS Chain Testnet'),
}

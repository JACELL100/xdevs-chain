/**
 * Enclave testnet constants for UI use (kept dependency-free; must match `xdevsEnclave` in @xdevs/sdk,
 * which a test in packages/ui checks).
 */

export const ENCLAVE = {
  chainId: 717171,
  chainIdHex: '0xaf173',
  /** Full network name, as wallets show it. */
  name: 'xDEVS Chain Enclave',
  /** Short label for badges. */
  shortName: 'Enclave testnet',
  nativeCurrency: { name: 'EnclaveXDS', symbol: 'tXDS', decimals: 18 },
  rpcUrl: 'https://rpc.xdevschain.tech',
  wsUrl: 'wss://ws.xdevschain.tech',
  explorerUrl: 'https://explorer.xdevschain.tech',
  explorerApiUrl: 'https://explorer-api.xdevschain.tech',
  bundlerUrl: 'https://bundler.xdevschain.tech',
  iconUrl: 'https://xdevschain.tech/brand/xdevs-chain-mark.svg',
} as const

/** Public xDEVS web properties, used for the footer cross-links. */
export const XDEVS_LINKS = {
  site: 'https://xdevschain.tech',
  docs: 'https://xdevschain.tech/guide/introduction',
  endpoints: 'https://xdevschain.tech/network/endpoints',
  explorer: 'https://explorer.xdevschain.tech',
  wallet: 'https://wallet.xdevschain.tech',
  faucet: 'https://faucet.xdevschain.tech',
  status: 'https://status.xdevschain.tech',
  github: 'https://github.com/JACELL100/xdevs-chain',
  xdevs: 'https://xdevs.in',
} as const

export type XdevsAppId = 'explorer' | 'wallet' | 'faucet' | 'status'

/** EIP-3085 `wallet_addEthereumChain` parameter. */
export type AddEthereumChainParameter = {
  chainId: `0x${string}`
  chainName: string
  nativeCurrency: { name: string; symbol: string; decimals: number }
  rpcUrls: string[]
  blockExplorerUrls?: string[]
  iconUrls?: string[]
}

/** The EIP-3085 parameters that add Enclave to MetaMask & co. */
export function enclaveAddChainParameters(): AddEthereumChainParameter {
  return {
    chainId: ENCLAVE.chainIdHex,
    chainName: ENCLAVE.name,
    nativeCurrency: { ...ENCLAVE.nativeCurrency },
    rpcUrls: [ENCLAVE.rpcUrl],
    blockExplorerUrls: [ENCLAVE.explorerUrl],
    iconUrls: [ENCLAVE.iconUrl],
  }
}

export type NetworkStatus = 'live' | 'degraded' | 'down' | 'checking' | 'unknown'

/**
 * Probes a JSON-RPC endpoint with `eth_chainId`: `live` when it answers with the expected chain id,
 * `degraded` when it answers with another id or an error object, `down` on network/HTTP failure or timeout.
 */
export async function probeRpc(
  rpcUrl: string = ENCLAVE.rpcUrl,
  { expectedChainId = ENCLAVE.chainId, timeoutMs = 6000, fetchImpl = globalThis.fetch } = {},
): Promise<NetworkStatus> {
  const ctrl = new AbortController()
  const timer = setTimeout(() => ctrl.abort(), timeoutMs)
  try {
    const res = await fetchImpl(rpcUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_chainId', params: [] }),
      signal: ctrl.signal,
    })
    if (!res.ok) return 'down'
    const body = (await res.json()) as { result?: unknown }
    if (typeof body.result !== 'string') return 'degraded'
    return Number.parseInt(body.result, 16) === expectedChainId ? 'live' : 'degraded'
  } catch {
    return 'down'
  } finally {
    clearTimeout(timer)
  }
}

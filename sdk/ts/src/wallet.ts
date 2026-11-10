import { type Account, type Chain, type Client, type Transport, BaseError, numberToHex } from 'viem'

/** EIP-3085 `wallet_addEthereumChain` parameter, as listed in plan §23.1. */
export type AddEthereumChainParameter = {
  chainId: `0x${string}`
  chainName: string
  nativeCurrency: { name: string; symbol: string; decimals: number }
  rpcUrls: string[]
  blockExplorerUrls?: string[]
  iconUrls?: string[]
}

/** Builds the EIP-3085 parameters for `chain` (plan §23.1). `chainId` is lowercase hex (`0xaf173`). */
export function getAddChainParameters(chain: Chain, options: { iconUrls?: string[] } = {}): AddEthereumChainParameter {
  const explorers = chain.blockExplorers ? Object.values(chain.blockExplorers).map((e) => e.url) : []
  return {
    chainId: numberToHex(chain.id),
    chainName: chain.name,
    nativeCurrency: { ...chain.nativeCurrency },
    rpcUrls: [...chain.rpcUrls.default.http],
    ...(explorers.length ? { blockExplorerUrls: [...new Set(explorers)] } : {}),
    ...(options.iconUrls?.length ? { iconUrls: options.iconUrls } : {}),
  }
}

/**
 * Asks the wallet (MetaMask etc.) to add `chain` via `wallet_addEthereumChain`.
 * Throws before contacting the wallet if the chain still uses a placeholder `.invalid` URL
 * (e.g. the unconfigured `xdevsTestnet`).
 */
export async function addXdevsChainToWallet(
  walletClient: Client<Transport, Chain | undefined, Account | undefined>,
  chain: Chain,
  options: { iconUrls?: string[] } = {},
): Promise<void> {
  const params = getAddChainParameters(chain, options)
  const placeholder = [...params.rpcUrls, ...(params.blockExplorerUrls ?? [])].find((u) => {
    try {
      return new URL(u).hostname.endsWith('.invalid')
    } catch {
      return true
    }
  })
  if (placeholder)
    throw new BaseError(`Chain "${chain.name}" still has a placeholder URL (${placeholder}).`, {
      name: 'XdevsPlaceholderChainError',
      metaMessages: ['Override rpcUrls/blockExplorers with the real endpoints before adding it to a wallet.'],
    })
  const request = walletClient.request as unknown as (
    args: { method: string; params: unknown },
    options?: { retryCount?: number },
  ) => Promise<unknown>
  await request({ method: 'wallet_addEthereumChain', params: [params] }, { retryCount: 0 })
}

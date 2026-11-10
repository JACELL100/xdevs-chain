// EIP-1193 (MetaMask) and dev-key transaction senders for guardian actions and the EIP-7702 panel.
import { type Address, type Hex, type WalletClient, createWalletClient, custom, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { chain, config, publicClient } from '../config.ts'

type Eip1193 = { request(args: { method: string; params?: unknown[] }): Promise<unknown> }

export function injected(): Eip1193 | null {
  return ((globalThis as { ethereum?: Eip1193 }).ethereum ?? null) as Eip1193 | null
}

/** Connects MetaMask (or any EIP-1193 wallet) and switches it to this chain when possible. */
export async function connectInjected(): Promise<{ address: Address; wallet: WalletClient }> {
  const eth = injected()
  if (!eth) throw new Error('No browser wallet (EIP-1193) found')
  const accounts = (await eth.request({ method: 'eth_requestAccounts' })) as Address[]
  if (!accounts[0]) throw new Error('no account authorised')
  try {
    await eth.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: `0x${config.chainId.toString(16)}` }] })
  } catch {
    try {
      await eth.request({
        method: 'wallet_addEthereumChain',
        params: [{ chainId: `0x${config.chainId.toString(16)}`, chainName: chain.name, nativeCurrency: chain.nativeCurrency, rpcUrls: [config.rpcUrl] }],
      })
    } catch {
      /* user declined: reads still work through our RPC */
    }
  }
  return { address: accounts[0], wallet: createWalletClient({ account: accounts[0], chain, transport: custom(eth) }) }
}

export function devKeyWallet(key: Hex): { address: Address; wallet: WalletClient } {
  const account = privateKeyToAccount(key)
  return { address: account.address, wallet: createWalletClient({ account, chain, transport: http(config.rpcUrl) }) }
}

export async function sendAndWait(wallet: WalletClient, to: Address, data: Hex): Promise<Hex> {
  const hash = await wallet.sendTransaction({ account: wallet.account ?? null, chain, to, data } as never)
  const r = await publicClient.waitForTransactionReceipt({ hash, timeout: 90_000 })
  if (r.status !== 'success') throw new Error(`transaction ${hash} reverted`)
  return hash
}

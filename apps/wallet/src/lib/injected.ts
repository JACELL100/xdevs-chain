// EIP-1193 browser wallet (MetaMask & co.) helpers: connect + switch to this chain, send a transaction, and the
// lock-safe fees XIP-0001 requires for self-calls of a LOCKED EOA (max fee <= 2 x base fee).
import { type Address, type Hex, type WalletClient, createWalletClient, custom } from 'viem'
import { chain, config, publicClient } from '../config.ts'

type Eip1193 = { request(args: { method: string; params?: unknown[] }): Promise<unknown> }

export function injected(): Eip1193 | null {
  return ((globalThis as { ethereum?: Eip1193 }).ethereum ?? null) as Eip1193 | null
}

/** Connects the browser wallet and switches it to this chain (adding it when unknown). */
export async function connectInjected(): Promise<{ address: Address; wallet: WalletClient }> {
  const eth = injected()
  if (!eth) throw new Error('No browser wallet found. Install MetaMask (or another EIP-1193 wallet) and reload.')
  const accounts = (await eth.request({ method: 'eth_requestAccounts' })) as Address[]
  if (!accounts[0]) throw new Error('The wallet did not share an account.')
  const chainId = `0x${config.chainId.toString(16)}`
  try {
    await eth.request({ method: 'wallet_switchEthereumChain', params: [{ chainId }] })
  } catch {
    try {
      await eth.request({
        method: 'wallet_addEthereumChain',
        params: [{ chainId, chainName: config.chainName, nativeCurrency: chain.nativeCurrency, rpcUrls: [config.rpcUrl], blockExplorerUrls: [config.explorerUrl] }],
      })
    } catch {
      /* declined: reads still work through the wallet's own RPC */
    }
  }
  return { address: accounts[0], wallet: createWalletClient({ account: accounts[0], chain, transport: custom(eth) }) }
}

/** Fees valid for a self-call of a LOCKED EOA (XIP-0001): max fee <= 2 x base fee, tip <= base fee. */
export async function lockSafeFees() {
  const block = await publicClient.getBlock({ blockTag: 'latest' })
  const baseFee = block.baseFeePerGas ?? 1_000_000_000n
  let tip = 1_000_000n
  try {
    tip = await publicClient.estimateMaxPriorityFeePerGas()
  } catch {
    /* default */
  }
  return { maxFeePerGas: baseFee * 2n, maxPriorityFeePerGas: tip < baseFee ? tip : baseFee }
}

export async function sendAndWait(wallet: WalletClient, to: Address, data: Hex, opts: { lockSafe?: boolean; gas?: bigint } = {}): Promise<Hex> {
  const fees = opts.lockSafe ? await lockSafeFees() : {}
  const hash = await wallet.sendTransaction({ account: wallet.account ?? null, chain, to, data, value: 0n, ...(opts.gas ? { gas: opts.gas } : {}), ...fees } as never)
  const r = await publicClient.waitForTransactionReceipt({ hash, timeout: 90_000 })
  if (r.status !== 'success') throw new Error(`Transaction ${hash} reverted.`)
  return hash
}

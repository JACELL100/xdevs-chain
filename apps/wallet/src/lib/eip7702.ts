// EIP-7702 upgrade of an existing EOA to the xDEVS Smart Account + XIP-0001 EOA lock.
// DEV PATH: uses a raw private key, because MetaMask (and most wallets) only sign 7702 authorizations to their own
// delegator contract, never to an arbitrary address such as 0x…1005.
import { type Address, type Hex, type PublicClient, createWalletClient, encodeFunctionData, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { accountAbi } from './abi.ts'
import { DESIGNATOR, type InitParams, SMART_ACCOUNT_IMPL } from './account.ts'

export async function is7702Delegated(client: PublicClient, address: Address): Promise<boolean> {
  const code = await client.getCode({ address })
  return !!code && code.toLowerCase() === DESIGNATOR
}

/**
 * Fees for a transaction the EOA sends to itself. XIP-0001: a LOCKED EOA may only send value-0 self-calls with
 * max fee <= 2 x base fee (and gas <= 2,000,000), so we never exceed that.
 */
async function lockSafeFees(client: PublicClient) {
  const block = await client.getBlock({ blockTag: 'latest' })
  const baseFee = block.baseFeePerGas ?? 1_000_000_000n
  let tip = 1_000_000n
  try {
    tip = await client.estimateMaxPriorityFeePerGas()
  } catch {
    /* default */
  }
  const maxFeePerGas = baseFee * 2n
  return { maxFeePerGas, maxPriorityFeePerGas: tip < baseFee ? tip : baseFee }
}

/** Type-4 transaction: delegate the EOA to 0x…1005 and `initialize(params)` on itself in the same transaction. */
export async function upgradeEoa(client: PublicClient, rpcUrl: string, privateKey: Hex, params: InitParams): Promise<Hex> {
  const account = privateKeyToAccount(privateKey)
  const wallet = createWalletClient({ account, transport: http(rpcUrl) })
  const authorization = await wallet.signAuthorization({ account, contractAddress: SMART_ACCOUNT_IMPL, executor: 'self' })
  const hash = await wallet.sendTransaction({
    chain: null,
    authorizationList: [authorization],
    to: account.address,
    data: encodeFunctionData({ abi: accountAbi, functionName: 'initialize', args: [params] }),
    gas: 1_500_000n,
    ...(await lockSafeFees(client)),
  })
  const r = await client.waitForTransactionReceipt({ hash, timeout: 60_000 })
  if (r.status !== 'success') throw new Error('7702 upgrade transaction reverted')
  return hash
}

/** The EOA key calls a function of its own account (value 0, gas <= 2M: valid even while XIP-0001 LOCKED). */
export async function eoaSelfCall(client: PublicClient, rpcUrl: string, privateKey: Hex, data: Hex): Promise<Hex> {
  const account = privateKeyToAccount(privateKey)
  const wallet = createWalletClient({ account, transport: http(rpcUrl) })
  const hash = await wallet.sendTransaction({ chain: null, to: account.address, value: 0n, data, gas: 1_000_000n, ...(await lockSafeFees(client)) })
  const r = await client.waitForTransactionReceipt({ hash, timeout: 60_000 })
  if (r.status !== 'success') throw new Error('self-call reverted')
  return hash
}

/** `enableEoaLock()` (instant; needs the vault enabled and a 7702 delegation). */
export function enableEoaLock(client: PublicClient, rpcUrl: string, privateKey: Hex): Promise<Hex> {
  return eoaSelfCall(client, rpcUrl, privateKey, encodeFunctionData({ abi: accountAbi, functionName: 'enableEoaLock' }))
}

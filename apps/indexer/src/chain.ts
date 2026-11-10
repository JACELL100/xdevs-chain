// Chain-facing helpers shared by the indexer and the API: RPC shapes, hex helpers, code classification, token metadata.

import { decodeFunctionResult, encodeFunctionData, getAddress, hexToString, type Hex } from 'viem'
import { predeploys } from '@xdevs/sdk'
import { RpcError, type Rpc, type RpcCall } from './rpc.ts'

export interface RpcTx {
  hash: Hex
  from: Hex
  to: Hex | null
  value: Hex
  nonce: Hex
  gas: Hex
  gasPrice?: Hex
  maxFeePerGas?: Hex
  maxPriorityFeePerGas?: Hex
  type?: Hex
  input: Hex
  transactionIndex: Hex
  authorizationList?: { chainId: Hex; address: Hex; nonce: Hex }[]
}

export interface RpcLog {
  address: Hex
  topics: Hex[]
  data: Hex
  logIndex: Hex
  transactionHash: Hex
  transactionIndex: Hex
  blockHash: Hex
  blockNumber: Hex
}

export interface RpcReceipt {
  transactionHash: Hex
  transactionIndex: Hex
  blockHash: Hex
  status?: Hex
  gasUsed: Hex
  effectiveGasPrice?: Hex
  contractAddress: Hex | null
  logs: RpcLog[]
}

export interface RpcBlock {
  number: Hex
  hash: Hex
  parentHash: Hex
  timestamp: Hex
  miner: Hex
  gasUsed: Hex
  gasLimit: Hex
  baseFeePerGas?: Hex | null
  size?: Hex
  transactions: RpcTx[]
}

export const hexToBig = (h: Hex | string | null | undefined): bigint => (h ? BigInt(h) : 0n)
export const hexToNum = (h: Hex | string | null | undefined): number => (h ? Number(BigInt(h)) : 0)
export const toHex = (n: number | bigint): Hex => `0x${n.toString(16)}`
export const lower = <T extends string | null | undefined>(a: T): T => (a ? (a.toLowerCase() as T) : a)
export const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000'

/** EIP-7702 delegation designator `0xef0100 ‖ address` (23 bytes). */
export function delegationTarget(code: string | null | undefined): string | null {
  if (!code) return null
  const c = code.toLowerCase()
  return c.length === 48 && c.startsWith('0xef0100') ? `0x${c.slice(8)}` : null
}

export type CodeKind = 'eoa' | 'contract' | 'delegated'
export function codeKind(code: string | null | undefined): CodeKind {
  if (!code || code === '0x') return 'eoa'
  return delegationTarget(code) ? 'delegated' : 'contract'
}

/** ERC-1967 implementation slot. */
export const ERC1967_IMPL_SLOT = '0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc'
const ACCOUNT_IMPL = predeploys.SMART_ACCOUNT_IMPL.toLowerCase()

export type AccountKind = 'xdevs-7702' | 'xdevs-proxy' | 'unknown'

/** Classifies an address as an xDEVS smart account (EIP-7702 delegated to 0x…1005, or an ERC-1967 proxy to it). */
export async function accountKinds(rpc: Rpc, accounts: string[], block: Hex | 'latest'): Promise<Map<string, AccountKind>> {
  const out = new Map<string, AccountKind>()
  if (!accounts.length) return out
  const codes = await rpc.batch(accounts.map((a) => ({ method: 'eth_getCode', params: [a, block] })))
  const proxies: string[] = []
  accounts.forEach((a, i) => {
    const code = codes[i]
    if (code instanceof RpcError || typeof code !== 'string') return out.set(a, 'unknown')
    if (delegationTarget(code) === ACCOUNT_IMPL) return out.set(a, 'xdevs-7702')
    if (code !== '0x') proxies.push(a)
    out.set(a, 'unknown')
  })
  if (proxies.length) {
    const slots = await rpc.batch(proxies.map((a) => ({ method: 'eth_getStorageAt', params: [a, ERC1967_IMPL_SLOT, block] })))
    proxies.forEach((a, i) => {
      const s = slots[i]
      if (typeof s === 'string' && `0x${s.slice(-40)}`.toLowerCase() === ACCOUNT_IMPL) out.set(a, 'xdevs-proxy')
    })
  }
  return out
}

const metaAbi = [
  { type: 'function', name: 'name', inputs: [], outputs: [{ type: 'string' }], stateMutability: 'view' },
  { type: 'function', name: 'symbol', inputs: [], outputs: [{ type: 'string' }], stateMutability: 'view' },
  { type: 'function', name: 'decimals', inputs: [], outputs: [{ type: 'uint8' }], stateMutability: 'view' },
] as const

export interface TokenMeta {
  address: string
  name: string | null
  symbol: string | null
  decimals: number | null
}

function decodeStringish(fn: 'name' | 'symbol', data: unknown): string | null {
  if (typeof data !== 'string' || data === '0x') return null
  try {
    return sanitize(decodeFunctionResult({ abi: metaAbi, functionName: fn, data: data as Hex }) as string)
  } catch {
    // Old tokens (MKR-style) return bytes32.
    if (data.length === 66) {
      try {
        return sanitize(hexToString(data as Hex, { size: 32 }).replace(/\0+$/, ''))
      } catch {
        return null
      }
    }
    return null
  }
}

/** Strips control characters and caps length: token metadata is attacker-controlled display text. */
export function sanitize(s: string): string | null {
  // eslint-disable-next-line no-control-regex
  const clean = s.replace(/[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁦-⁩]/g, '').trim().slice(0, 64)
  return clean || null
}

export async function fetchTokenMeta(rpc: Rpc, tokens: string[], block: Hex | 'latest'): Promise<TokenMeta[]> {
  if (!tokens.length) return []
  const calls: RpcCall[] = []
  for (const t of tokens)
    for (const fn of ['name', 'symbol', 'decimals'] as const)
      calls.push({ method: 'eth_call', params: [{ to: t, data: encodeFunctionData({ abi: metaAbi, functionName: fn }) }, block] })
  const res = await rpc.batch(calls)
  return tokens.map((address, i) => {
    let decimals: number | null = null
    const d = res[i * 3 + 2]
    if (typeof d === 'string' && d.length >= 66) {
      const v = BigInt(d.slice(0, 66))
      if (v <= 255n) decimals = Number(v)
    }
    return { address, name: decodeStringish('name', res[i * 3]), symbol: decodeStringish('symbol', res[i * 3 + 1]), decimals }
  })
}

export const checksum = (a: string | null | undefined): string | null => (a ? getAddress(a) : null)

/** Predeploy labels (contracts/src/common/Predeploys.sol). */
export const PREDEPLOY_LABELS: Record<string, string> = Object.fromEntries(
  Object.entries({
    STAKING: 'Staking (predeploy)',
    GOVERNANCE: 'Governance (predeploy)',
    TREASURY: 'Treasury (predeploy)',
    FEE_CONFIG: 'FeeConfig (predeploy)',
    XNS_REGISTRY: 'XNS Registry (predeploy)',
    SMART_ACCOUNT_IMPL: 'Smart Account implementation (predeploy)',
    SMART_ACCOUNT_FACTORY: 'Smart Account factory (predeploy)',
    PAYMASTER: 'Paymaster (predeploy)',
    PERMIT2_EXPIRING: 'Permit2Expiring (predeploy)',
    WXDS: 'Wrapped XDS (predeploy)',
    SYSTEM_ADDRESS: 'System address',
    P256_VERIFY: 'P256VERIFY precompile',
  }).flatMap(([k, label]) => {
    const a = (predeploys as Record<string, string>)[k]
    return a ? [[a.toLowerCase(), label]] : []
  }),
)

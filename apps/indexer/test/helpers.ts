// Test helpers: an in-memory fake chain that answers the eth_ JSON-RPC subset the indexer and API use.

import {
  encodeAbiParameters,
  encodeEventTopics,
  keccak256,
  toHex as vToHex,
  decodeFunctionData,
  type Abi,
  type Hex,
} from 'viem'
import { createPgliteDb, migrate, type Db } from '../src/db.ts'
import { RpcError, type Rpc, type RpcCall } from '../src/rpc.ts'
import type { RpcBlock, RpcLog, RpcReceipt } from '../src/chain.ts'
import { testConfig, type Config } from '../src/config.ts'

export const toHex = (n: number | bigint): Hex => `0x${n.toString(16)}`
export const addr = (n: number | string): Hex =>
  (typeof n === 'string' ? (n.startsWith('0x') ? n : `0x${n}`) : `0x${n.toString(16).padStart(40, '0')}`).toLowerCase() as Hex

export interface FakeLog {
  address: string
  topics: Hex[]
  data: Hex
}

export interface FakeTx {
  from: string
  to: string | null
  value?: bigint
  input?: Hex
  logs?: FakeLog[]
  contractAddress?: string
  status?: 0 | 1
  type?: number
}

export function log(abi: Abi | readonly unknown[], eventName: string, args: Record<string, unknown>, address: string): FakeLog {
  const item = (abi as { type: string; name: string; inputs: { name: string; type: string; indexed?: boolean; components?: unknown }[] }[]).find(
    (x) => x.type === 'event' && x.name === eventName,
  )
  if (!item) throw new Error(`no event ${eventName}`)
  const topics = encodeEventTopics({ abi: [item] as Abi, eventName, args: args as never }) as Hex[]
  const nonIndexed = item.inputs.filter((i) => !i.indexed)
  const data = nonIndexed.length ? encodeAbiParameters(nonIndexed as never, nonIndexed.map((i) => args[i.name]) as never) : '0x'
  return { address: address.toLowerCase(), topics, data }
}

export const erc20MetaAbi = [
  { type: 'function', name: 'name', inputs: [], outputs: [{ type: 'string' }], stateMutability: 'view' },
  { type: 'function', name: 'symbol', inputs: [], outputs: [{ type: 'string' }], stateMutability: 'view' },
  { type: 'function', name: 'decimals', inputs: [], outputs: [{ type: 'uint8' }], stateMutability: 'view' },
  { type: 'function', name: 'allowance', inputs: [{ type: 'address' }, { type: 'address' }], outputs: [{ type: 'uint256' }], stateMutability: 'view' },
] as const

export class FakeChain implements Rpc {
  chainId = 7117
  clientVersion = 'xdevs/0.1.0'
  blocks: { block: RpcBlock; receipts: RpcReceipt[] }[] = []
  codes = new Map<string, Hex>()
  storage = new Map<string, Hex>()
  tokens = new Map<string, { name: string; symbol: string; decimals: number }>()
  allowances = new Map<string, bigint>()
  eoaLock: Record<string, unknown> | null = null
  calls: string[] = []
  baseTimestamp = 1_800_000_000
  salt = 'a'
  private nonces = new Map<string, number>()

  constructor() {
    this.addBlock([]) // genesis
  }

  get head(): number {
    return this.blocks.length - 1
  }

  addBlock(txs: FakeTx[], opts: { timestamp?: number } = {}): RpcBlock {
    const number = this.blocks.length
    const parent = this.blocks[number - 1]?.block.hash ?? (`0x${'0'.repeat(64)}` as Hex)
    const hash = keccak256(vToHex(`block-${number}-${this.salt}`))
    const baseFee = 1_000_000_000n
    const timestamp = opts.timestamp ?? this.baseTimestamp + number * 2
    let logIndex = 0
    const rpcTxs = txs.map((t, i) => {
      const from = t.from.toLowerCase()
      const nonce = this.nonces.get(from) ?? 0
      this.nonces.set(from, nonce + 1)
      return {
        hash: keccak256(vToHex(`tx-${number}-${i}-${this.salt}`)),
        from: from as Hex,
        to: (t.to?.toLowerCase() ?? null) as Hex | null,
        value: toHex(t.value ?? 0n),
        nonce: toHex(nonce),
        gas: toHex(100_000),
        maxFeePerGas: toHex(3_000_000_000n),
        maxPriorityFeePerGas: toHex(1_000_000_000n),
        type: toHex(t.type ?? 2),
        input: t.input ?? '0x',
        transactionIndex: toHex(i),
      }
    })
    const receipts: RpcReceipt[] = txs.map((t, i) => ({
      transactionHash: rpcTxs[i]!.hash,
      transactionIndex: toHex(i),
      blockHash: hash,
      status: toHex(t.status ?? 1),
      gasUsed: toHex(21_000),
      effectiveGasPrice: toHex(2_000_000_000n),
      contractAddress: (t.contractAddress?.toLowerCase() ?? null) as Hex | null,
      logs: (t.status === 0 ? [] : (t.logs ?? [])).map(
        (l): RpcLog => ({
          address: l.address as Hex,
          topics: l.topics,
          data: l.data,
          logIndex: toHex(logIndex++),
          transactionHash: rpcTxs[i]!.hash,
          transactionIndex: toHex(i),
          blockHash: hash,
          blockNumber: toHex(number),
        }),
      ),
    }))
    for (const t of txs) if (t.contractAddress) this.codes.set(t.contractAddress.toLowerCase(), '0x6080')
    const block: RpcBlock = {
      number: toHex(number),
      hash,
      parentHash: parent,
      timestamp: toHex(timestamp),
      miner: addr(0xfee),
      gasUsed: toHex(21_000 * txs.length),
      gasLimit: toHex(30_000_000),
      baseFeePerGas: toHex(baseFee),
      size: toHex(1000),
      transactions: rpcTxs,
    }
    this.blocks.push({ block, receipts })
    return block
  }

  /** Replaces blocks >= n with a different history (same tx shapes, new hashes). */
  fork(n: number, salt: string) {
    const old = this.blocks.slice(n)
    this.blocks = this.blocks.slice(0, n)
    this.salt = salt
    for (const b of old) {
      const txs: FakeTx[] = b.block.transactions.map((t, i) => ({
        from: t.from,
        to: t.to,
        value: BigInt(t.value),
        input: t.input,
        logs: b.receipts[i]!.logs.map((l) => ({ address: l.address, topics: l.topics, data: l.data })),
      }))
      this.addBlock(txs)
    }
  }

  async request<T>(method: string, params: unknown[] = []): Promise<T> {
    this.calls.push(method)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const p = params as any[]
    switch (method) {
      case 'eth_chainId':
        return toHex(this.chainId) as T
      case 'web3_clientVersion':
        return this.clientVersion as T
      case 'eth_blockNumber':
        return toHex(this.head) as T
      case 'eth_getBlockByNumber': {
        const b = this.blocks[Number(BigInt(p[0] as string))]
        if (!b) return null as T
        return (p[1] ? b.block : { ...b.block, transactions: b.block.transactions.map((t) => t.hash) }) as T
      }
      case 'eth_getBlockReceipts': {
        const b = this.blocks[Number(BigInt(p[0] as string))]
        return (b ? b.receipts : null) as T
      }
      case 'eth_getLogs': {
        const f = p[0] as { fromBlock: string; toBlock: string }
        const out: RpcLog[] = []
        for (let n = Number(BigInt(f.fromBlock)); n <= Number(BigInt(f.toBlock)); n++)
          for (const r of this.blocks[n]?.receipts ?? []) out.push(...r.logs)
        return out as T
      }
      case 'eth_getCode':
        return (this.codes.get(String(p[0]).toLowerCase()) ?? '0x') as T
      case 'eth_getStorageAt':
        return (this.storage.get(`${String(p[0]).toLowerCase()}:${String(p[1]).toLowerCase()}`) ?? `0x${'0'.repeat(64)}`) as T
      case 'eth_getBalance':
        return toHex(10n ** 18n) as T
      case 'eth_getTransactionCount':
        return toHex(this.nonces.get(String(p[0]).toLowerCase()) ?? 0) as T
      case 'xdevs_getEoaLock':
        if (!this.eoaLock) throw new RpcError(-32601, 'the method xdevs_getEoaLock does not exist')
        return this.eoaLock as T
      case 'xdevs_getCommitCertificate':
        return null as T
      case 'eth_call': {
        const call = p[0] as { to: string; data: Hex }
        const to = call.to.toLowerCase()
        const meta = this.tokens.get(to)
        if (!meta) return '0x' as T
        const { functionName, args } = decodeFunctionData({ abi: erc20MetaAbi, data: call.data })
        if (functionName === 'name') return encodeAbiParameters([{ type: 'string' }], [meta.name]) as T
        if (functionName === 'symbol') return encodeAbiParameters([{ type: 'string' }], [meta.symbol]) as T
        if (functionName === 'decimals') return encodeAbiParameters([{ type: 'uint8' }], [meta.decimals]) as T
        const key = `${to}:${String(args![0]).toLowerCase()}:${String(args![1]).toLowerCase()}`
        return encodeAbiParameters([{ type: 'uint256' }], [this.allowances.get(key) ?? 0n]) as T
      }
      default:
        throw new RpcError(-32601, `method ${method} not found`)
    }
  }

  async batch(calls: RpcCall[]): Promise<unknown[]> {
    return Promise.all(calls.map((c) => this.request(c.method, c.params).catch((e: unknown) => e)))
  }
}

export async function freshDb(): Promise<Db> {
  const db = await createPgliteDb()
  await migrate(db)
  return db
}

export function cfg(overrides: Partial<Config> = {}): Config {
  return testConfig({ batchSize: 5, verifyLogs: true, ...overrides })
}

export const silent = () => {}

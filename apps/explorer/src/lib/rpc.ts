// Direct node reads (viem). Used for the live head, the commit certificate, validators, EOA lock, XNS reverse lookup,
// token metadata, and as the fallback for every view when the indexer API is down. Results use the API's shapes.

import { createClient, encodeFunctionData, getAddress, http, maxUint256, parseAbi, toHex, type Client, type Hex, type Block, type Transaction, type TransactionReceipt } from 'viem'
import {
  getBalance,
  getBlock,
  getBlockNumber,
  getBlockReceipts,
  getChainId,
  getCode,
  getLogs,
  getTransaction,
  getTransactionCount,
  getTransactionReceipt,
  readContract,
} from 'viem/actions'
import { getApprovals, getCommit, getValidators, getVaultQueue, predeploys, XdevsMethodNotAvailableError, type XdevsApproval } from '@xdevs/sdk'
import { config } from './config.ts'
import { decodeLog, vaultAbi } from './decode.ts'
import type { AddressSummary, Approval, BlockDetail, BlockSummary, CommitDetail, DecodedLog, TokenInfo, TokenPage, TokenTransfer, TxDetail, TxSummary, ValidatorRow, ValidatorSet, VaultAction } from './types.ts'

const BURN_BPS = 8000n
const hx = (v: bigint | number) => `0x${BigInt(v).toString(16)}`

// A bare viem Client plus only the actions the explorer uses (tree-shakeable, unlike createPublicClient, which pulls
// in every public action: ENS, multicall, KZG, ...). Requests made in the same tick go out as one JSON-RPC batch.
function makeClient() {
  const base = createClient({ transport: http(config.rpcUrl, { batch: { batchSize: 50, wait: 8 }, retryCount: 0, timeout: 10_000 }) })
  return {
    base,
    request: base.request,
    getChainId: () => getChainId(base),
    getBlockNumber: (a?: Parameters<typeof getBlockNumber>[1]) => getBlockNumber(base, a),
    getBlock: getBlockBound(base),
    getBlockReceipts: (a: Parameters<typeof getBlockReceipts>[1]) => getBlockReceipts(base, a),
    getTransaction: (a: Parameters<typeof getTransaction>[1]) => getTransaction(base, a),
    getTransactionReceipt: (a: Parameters<typeof getTransactionReceipt>[1]) => getTransactionReceipt(base, a),
    getBalance: (a: Parameters<typeof getBalance>[1]) => getBalance(base, a),
    getTransactionCount: (a: Parameters<typeof getTransactionCount>[1]) => getTransactionCount(base, a),
    getCode: (a: Parameters<typeof getCode>[1]) => getCode(base, a),
    readContract: ((a: Parameters<typeof readContract>[1]) => readContract(base, a)) as unknown as <const abi extends readonly unknown[], fn extends string>(a: {
      address: Hex
      abi: abi
      functionName: fn
      args?: readonly unknown[]
    }) => Promise<ReadResult<abi, fn>>,
    getLogs: ((a: Parameters<typeof getLogs>[1]) => getLogs(base, a)) as typeof getLogsBound,
  }
}

// Typed wrappers (keep the call sites as readable as viem's public client).
function getBlockBound(base: Client) {
  return <const inc extends boolean = false>(a?: { blockNumber?: bigint; blockHash?: Hex; includeTransactions?: inc }) =>
    getBlock(base, a as Parameters<typeof getBlock>[1]) as unknown as Promise<Block<bigint, inc>>
}
declare function getLogsBound<const ev extends import('viem').AbiEvent>(a: { address: Hex; event: ev; fromBlock: bigint; toBlock: bigint }): Promise<import('viem').GetLogsReturnType<ev>>
type ReadResult<abi extends readonly unknown[], fn extends string> = abi extends import('viem').Abi ? import('viem').ContractFunctionReturnType<abi, 'view' | 'pure', fn & import('viem').ContractFunctionName<abi, 'view' | 'pure'>> : unknown

let client: ReturnType<typeof makeClient> | null = null
export function rpcClient() {
  if (!client) client = makeClient()
  return client
}

/** Raw request for methods viem does not type (`xdevs_getEoaLock`, `eth_getBlockReceipts`). */
async function raw<T>(method: string, params: unknown[]): Promise<T> {
  const req = rpcClient().request as unknown as (a: { method: string; params: unknown[] }) => Promise<T>
  return req({ method, params })
}

export async function rpcChainId(): Promise<number> {
  return rpcClient().getChainId()
}

export async function rpcHead(): Promise<number> {
  return Number(await rpcClient().getBlockNumber({ cacheTime: 0 }))
}

// --------------------------------------------------------------------------------------------------------------
// Blocks and transactions
// --------------------------------------------------------------------------------------------------------------

function blockFees(b: Block): BlockSummary['fees'] {
  const base = (b.baseFeePerGas ?? 0n) * b.gasUsed
  const burned = (base * BURN_BPS) / 10000n
  return { burned: hx(burned), treasury: hx(base - burned), priority: null }
}

export function toBlockSummary(b: Block): BlockSummary {
  return {
    number: Number(b.number),
    hash: b.hash ?? '0x',
    parentHash: b.parentHash,
    timestamp: Number(b.timestamp),
    miner: getAddress(b.miner),
    txCount: b.transactions.length,
    gasUsed: hx(b.gasUsed),
    gasLimit: hx(b.gasLimit),
    baseFeePerGas: b.baseFeePerGas !== null && b.baseFeePerGas !== undefined ? hx(b.baseFeePerGas) : null,
    size: b.size !== undefined ? Number(b.size) : null,
    fees: blockFees(b),
  }
}

export function toTxSummary(t: Transaction, ts: number, r?: TransactionReceipt | null): TxSummary {
  const typeNum = { legacy: 0, eip2930: 1, eip1559: 2, eip4844: 3, eip7702: 4 }[t.type as string] ?? 0
  return {
    hash: t.hash,
    blockNumber: Number(t.blockNumber ?? 0n),
    timestamp: ts,
    txIndex: t.transactionIndex ?? 0,
    type: typeNum,
    from: getAddress(t.from),
    to: t.to ? getAddress(t.to) : null,
    contractCreated: r?.contractAddress ? getAddress(r.contractAddress) : null,
    value: hx(t.value),
    status: r ? (r.status === 'success' ? 'success' : 'reverted') : 'unknown',
    methodId: t.input && t.input.length >= 10 ? t.input.slice(0, 10) : null,
    fee: r ? hx(r.gasUsed * r.effectiveGasPrice) : null,
    spam: false,
    spamReasons: [],
  }
}

/** The last `count` blocks ending at `head`, with their transactions (one batched HTTP round trip). */
export async function rpcRecentBlocks(head: number, count: number): Promise<{ blocks: BlockSummary[]; txs: TxSummary[] }> {
  const c = rpcClient()
  const nums = Array.from({ length: Math.min(count, head + 1) }, (_, i) => BigInt(head - i))
  const blocks = await Promise.all(nums.map((n) => c.getBlock({ blockNumber: n, includeTransactions: true })))
  const txs: TxSummary[] = []
  for (const b of blocks) for (const t of [...b.transactions].reverse()) txs.push(toTxSummary(t as Transaction, Number(b.timestamp)))
  return { blocks: blocks.map(toBlockSummary), txs }
}

export async function rpcBlockSummary(n: number): Promise<{ block: BlockSummary; txs: TxSummary[] }> {
  const b = await rpcClient().getBlock({ blockNumber: BigInt(n), includeTransactions: true })
  return { block: toBlockSummary(b), txs: [...b.transactions].reverse().map((t) => toTxSummary(t as Transaction, Number(b.timestamp))) }
}

async function blockReceipts(b: Block): Promise<TransactionReceipt[] | null> {
  if (!b.transactions.length) return []
  try {
    const rs = await rpcClient().getBlockReceipts({ blockNumber: b.number! })
    return rs
  } catch {
    try {
      return await Promise.all(b.transactions.map((t) => rpcClient().getTransactionReceipt({ hash: typeof t === 'string' ? t : t.hash })))
    } catch {
      return null
    }
  }
}

export async function rpcBlock(id: string): Promise<BlockDetail | null> {
  const c = rpcClient()
  let b: Block
  try {
    b = /^0x[0-9a-fA-F]{64}$/.test(id) ? await c.getBlock({ blockHash: id as Hex, includeTransactions: true }) : id === 'latest' ? await c.getBlock({ includeTransactions: true }) : await c.getBlock({ blockNumber: BigInt(id), includeTransactions: true })
  } catch (e) {
    if (/not found|could not be found/i.test(String((e as Error).message))) return null
    throw e
  }
  const receipts = await blockReceipts(b)
  const byHash = new Map((receipts ?? []).map((r) => [r.transactionHash, r]))
  const ts = Number(b.timestamp)
  const summary = toBlockSummary(b)
  if (receipts) {
    let priority = 0n
    for (const r of receipts) priority += r.gasUsed * (r.effectiveGasPrice - (b.baseFeePerGas ?? 0n))
    summary.fees.priority = hx(priority)
  }
  return {
    ...summary,
    transactions: b.transactions.map((t) => toTxSummary(t as Transaction, ts, byHash.get((t as Transaction).hash) ?? null)),
    commit: null,
  }
}

const revokeAbi = parseAbi([
  'function approve(address spender, uint256 amount) returns (bool)',
  'function lockdown((address token, address spender)[] approvals)',
])

type FungibleApproval = XdevsApproval & { kind: 'erc20' | 'xdrc20' | 'permit2' }

/**
 * `xdevs_getApprovals` (Phase 4 node): live fungible approvals found in the node's bounded log window (the indexer has
 * the full history). `null` when the node does not serve it.
 */
export async function rpcApprovals(owner: string): Promise<Approval[] | null> {
  let items: XdevsApproval[]
  try {
    items = await getApprovals(rpcClient().base, { owner: getAddress(owner) })
  } catch (e) {
    if (e instanceof XdevsMethodNotAvailableError) return null
    throw e
  }
  const now = Math.floor(Date.now() / 1000)
  return items
    .filter((i): i is FungibleApproval => i.kind === 'erc20' || i.kind === 'xdrc20' || i.kind === 'permit2')
    .map((i) => {
      const amount = i.rawAmount ?? toHex(i.amount === 'unlimited' ? maxUint256 : BigInt(i.amount))
      const permit2 = i.kind === 'permit2'
      return {
        kind: i.kind,
        token: { address: i.token, name: null, symbol: i.symbol ?? null, decimals: null, known: false },
        spender: i.spender,
        amount,
        unlimited: i.unlimited ?? i.amount === 'unlimited',
        expiry: i.expires,
        expired: i.expires !== null && i.expires <= now,
        remaining: amount,
        blockNumber: i.lastUpdatedBlock ? Number(i.lastUpdatedBlock) : 0,
        txHash: i.transactionHash ?? '',
        revoke: permit2
          ? {
              to: predeploys.PERMIT2_EXPIRING,
              data: encodeFunctionData({ abi: revokeAbi, functionName: 'lockdown', args: [[{ token: i.token, spender: i.spender }]] }),
              description: 'lockdown([(token, spender)])',
            }
          : {
              to: i.token,
              data: encodeFunctionData({ abi: revokeAbi, functionName: 'approve', args: [i.spender, 0n] }),
              description: 'approve(spender, 0)',
            },
      }
    })
}

/** `xdevs_getVaultQueue` (Phase 4 node): pending Guardian Vault actions read from the account. `null` when not served. */
export async function rpcVaultQueue(account: string): Promise<VaultAction[] | null> {
  try {
    const items = await getVaultQueue(rpcClient().base, { vault: getAddress(account) })
    return items.map((v) => ({
      account: v.account ?? getAddress(account),
      id: toHex(v.id),
      target: v.target,
      value: toHex(v.value),
      data: v.data,
      executeAfter: Number(v.executeAfter),
      status: v.status ?? 'queued',
      queuedBlock: 0,
      queuedTx: '',
      cancelledBy: null,
      finalBlock: null,
      finalTx: null,
    }))
  } catch (e) {
    if (e instanceof XdevsMethodNotAvailableError) return null
    throw e
  }
}

/** `xdevs_getCommitCertificate`, `null` when the node does not serve it or has no certificate for the height. */
export async function rpcCommit(height: number): Promise<CommitDetail | null> {
  try {
    const c = await getCommit(rpcClient().base, { height })
    if (!c) return null
    return { height: Number(c.height), round: c.round, blockHash: c.blockHash, signatures: c.signatures.map((s) => ({ validator: s.validator.toLowerCase(), signature: s.signature })) }
  } catch (e) {
    if (e instanceof XdevsMethodNotAvailableError) return null
    throw e
  }
}

const tokenMetaAbi = parseAbi([
  'function name() view returns (string)',
  'function symbol() view returns (string)',
  'function decimals() view returns (uint8)',
  'function totalSupply() view returns (uint256)',
  'function defaultApprovalDuration() view returns (uint64)',
  'function maxApprovalDuration() view returns (uint64)',
])

const metaCache = new Map<string, Promise<{ name: string | null; symbol: string | null; decimals: number | null }>>()
function tokenMeta(address: string) {
  const k = address.toLowerCase()
  let p = metaCache.get(k)
  if (!p) {
    const c = rpcClient()
    const read = <T,>(functionName: 'name' | 'symbol' | 'decimals') => c.readContract({ address: address as Hex, abi: tokenMetaAbi, functionName }).then((v) => v as T, () => null)
    p = Promise.all([read<string>('name'), read<string>('symbol'), read<number>('decimals')]).then(([name, symbol, decimals]) => ({ name, symbol, decimals: decimals === null ? null : Number(decimals) }))
    metaCache.set(k, p)
  }
  return p
}

const KNOWN_TOKENS = new Set([predeploys.WXDS.toLowerCase()])

export async function rpcTx(hash: string): Promise<TxDetail | null> {
  const c = rpcClient()
  let t: Transaction
  try {
    t = await c.getTransaction({ hash: hash as Hex })
  } catch (e) {
    if (/not found|could not be found/i.test(String((e as Error).message))) return null
    throw e
  }
  if (t.blockNumber === null) return null
  const [r, b] = await Promise.all([c.getTransactionReceipt({ hash: hash as Hex }), c.getBlock({ blockNumber: t.blockNumber })])
  const ts = Number(b.timestamp)
  const base = r.gasUsed * (b.baseFeePerGas ?? 0n)
  const total = r.gasUsed * r.effectiveGasPrice
  const burned = (base * BURN_BPS) / 10000n
  const logs: DecodedLog[] = r.logs.map((l) => {
    const d = decodeLog(l.topics, l.data)
    return {
      logIndex: l.logIndex ?? 0,
      address: getAddress(l.address),
      topics: l.topics,
      data: l.data,
      event: d?.event ?? null,
      signature: null,
      standard: d?.standard ?? null,
      args: d ? Object.fromEntries(Object.entries(d.args).map(([k, v]) => [k, typeof v === 'bigint' ? v.toString() : v])) : null,
    }
  })
  const transfers: TokenTransfer[] = []
  if (t.value > 0n && t.to)
    transfers.push({ blockNumber: Number(t.blockNumber), timestamp: ts, txHash: t.hash, logIndex: null, kind: 'native', token: { address: null, name: config.nativeName, symbol: config.nativeSymbol, decimals: 18, known: true }, from: getAddress(t.from), to: getAddress(t.to), value: hx(t.value), spam: false, spamReasons: [], lookAlikeOf: null })
  for (const l of logs) {
    if (l.event !== 'Transfer' || !l.args) continue
    const meta = await tokenMeta(l.address)
    transfers.push({
      blockNumber: Number(t.blockNumber), timestamp: ts, txHash: t.hash, logIndex: l.logIndex, kind: 'erc20',
      token: { address: l.address, ...meta, known: KNOWN_TOKENS.has(l.address.toLowerCase()) },
      from: getAddress(String(l.args.from)), to: getAddress(String(l.args.to)), value: hx(BigInt(String(l.args.value))), spam: false, spamReasons: [], lookAlikeOf: null,
    })
  }
  return {
    ...toTxSummary(t, ts, r),
    blockHash: t.blockHash ?? '0x',
    nonce: t.nonce,
    gas: hx(t.gas),
    gasUsed: hx(r.gasUsed),
    gasPrice: t.gasPrice !== undefined ? hx(t.gasPrice) : null,
    maxFeePerGas: t.maxFeePerGas !== undefined ? hx(t.maxFeePerGas) : null,
    maxPriorityFeePerGas: t.maxPriorityFeePerGas !== undefined ? hx(t.maxPriorityFeePerGas) : null,
    effectiveGasPrice: hx(r.effectiveGasPrice),
    input: t.input,
    authorizationList: t.authorizationList ? t.authorizationList.map((a) => ({ chainId: hx(a.chainId), address: getAddress(a.address), nonce: hx(a.nonce), authority: null })) : null,
    feeBreakdown: { total: hx(total), baseFee: hx(base), burned: hx(burned), treasury: hx(base - burned), priorityFee: hx(total - base), burnBps: Number(BURN_BPS) },
    logs,
    tokenTransfers: transfers,
    approvals: [],
  }
}

// --------------------------------------------------------------------------------------------------------------
// Addresses
// --------------------------------------------------------------------------------------------------------------

const xnsAbi = parseAbi(['function primaryName(address addr) view returns (string)'])

/** Primary `.xdevs` name from the XNS registry predeploy (verified on chain), or null. */
export async function rpcPrimaryName(address: string): Promise<string | null> {
  try {
    const n = await rpcClient().readContract({ address: predeploys.XNS_REGISTRY, abi: xnsAbi, functionName: 'primaryName', args: [address as Hex] })
    return n ? n : null
  } catch {
    return null
  }
}

type EoaLock = { active: boolean; status: 'unlocked' | 'locked' | 'sealed'; delegatedTo: string | null }

/** XIP-0001 EOA lock (`xdevs_getEoaLock`), null when the node does not serve it. */
export async function rpcEoaLock(address: string): Promise<EoaLock | null> {
  try {
    return await raw<EoaLock>('xdevs_getEoaLock', [address, 'latest'])
  } catch {
    return null
  }
}

export async function rpcAddress(a: string): Promise<AddressSummary> {
  const c = rpcClient()
  const address = getAddress(a)
  const [balance, nonce, code, lock, primaryName] = await Promise.all([
    c.getBalance({ address }),
    c.getTransactionCount({ address }),
    c.getCode({ address }).catch(() => undefined),
    rpcEoaLock(address),
    rpcPrimaryName(address),
  ])
  const delegated = code && code.startsWith('0xef0100') && code.length === 48 ? getAddress(`0x${code.slice(8)}`) : null
  const isContract = !!code && code !== '0x' && !delegated
  let token: AddressSummary['token'] = null
  if (isContract) {
    const meta = await tokenMeta(address)
    if (meta.symbol !== null && meta.decimals !== null) token = { address, ...meta, known: KNOWN_TOKENS.has(address.toLowerCase()) }
  }
  const isXdevsAccount = delegated?.toLowerCase() === predeploys.SMART_ACCOUNT_IMPL.toLowerCase()
  return {
    address,
    live: true,
    balance: hx(balance),
    nonce,
    kind: delegated ? 'delegated' : isContract ? 'contract' : 'eoa',
    delegatedTo: delegated,
    isContract,
    firstSeenBlock: null,
    lastSeenBlock: null,
    txCount: nonce,
    xns: { primaryName, names: [] },
    labels: [],
    token,
    security: {
      accountKind: isXdevsAccount ? 'xdevs-7702' : null,
      vaultEnabled: null,
      frozen: false,
      frozenUntil: null,
      eoaLock: lock ? { status: lock.status, active: lock.active, source: 'rpc' } : { status: 'unknown', active: null, source: 'rpc' },
      eoaKeyRevoked: false,
      recovery: null,
      guardians: null,
      openVaultActions: 0,
    },
  }
}

// --------------------------------------------------------------------------------------------------------------
// Tokens
// --------------------------------------------------------------------------------------------------------------

const transferEvent = parseAbi(['event Transfer(address indexed from, address indexed to, uint256 value)'])[0]

export async function rpcToken(a: string, head: number, scan = 5000): Promise<TokenPage | null> {
  const c = rpcClient()
  const address = getAddress(a)
  const meta = await tokenMeta(address)
  if (meta.symbol === null && meta.decimals === null) return null
  const read = <T,>(functionName: 'totalSupply' | 'defaultApprovalDuration' | 'maxApprovalDuration') => c.readContract({ address, abi: tokenMetaAbi, functionName }).then((v) => v as T, () => null)
  const [supply, defDur, maxDur] = await Promise.all([read<bigint>('totalSupply'), read<bigint>('defaultApprovalDuration'), read<bigint>('maxApprovalDuration')])
  const token: TokenInfo = {
    address, ...meta, totalSupply: supply !== null ? hx(supply) : null,
    xdrc20: defDur !== null ? { defaultApprovalDuration: Number(defDur), maxApprovalDuration: maxDur !== null ? Number(maxDur) : null } : null,
    known: KNOWN_TOKENS.has(address.toLowerCase()), label: address.toLowerCase() === predeploys.WXDS.toLowerCase() ? 'Wrapped XDS (predeploy)' : null,
  }
  let transfers: TokenTransfer[] = []
  const from = BigInt(Math.max(0, head - scan))
  try {
    const logs = await c.getLogs({ address, event: transferEvent, fromBlock: from, toBlock: BigInt(head) })
    const recent = logs.slice(-25).reverse()
    const stamps = new Map<bigint, number>()
    await Promise.all([...new Set(recent.map((l) => l.blockNumber!))].map(async (n) => stamps.set(n, Number((await c.getBlock({ blockNumber: n })).timestamp))))
    transfers = recent.map((l) => ({
      blockNumber: Number(l.blockNumber), timestamp: stamps.get(l.blockNumber!) ?? 0, txHash: l.transactionHash!, logIndex: l.logIndex ?? 0, kind: 'erc20' as const,
      token: { address, name: meta.name, symbol: meta.symbol, decimals: meta.decimals, known: token.known },
      from: getAddress(l.args.from!), to: getAddress(l.args.to!), value: hx(l.args.value!), spam: false, spamReasons: [], lookAlikeOf: null,
    }))
  } catch {
    /* log range too wide for the node: show the metadata only */
  }
  return { token, transfers, scannedBlocks: Number(BigInt(head) - from), holdersNote: 'Holder balances need the indexer.' }
}

// --------------------------------------------------------------------------------------------------------------
// Validators
// --------------------------------------------------------------------------------------------------------------

const stakingAbi = parseAbi([
  'function candidates() view returns (address[])',
  'function signingSet() view returns (address[] operators, uint256[] powers)',
  'function currentEpoch() view returns (uint64)',
  'function getValidator(address validator) view returns ((bytes32 consensusKey, uint8 status, uint16 commissionBps, uint64 jailedUntil, uint64 lastCommissionChange, uint256 totalStake, uint256 totalShares, uint256 selfStake, uint256 unbondingStake, uint256 unbondingShares, uint256 commissionAccrued))',
])
const STATUS = ['unknown', 'candidate', 'active', 'jailed', 'tombstoned', 'exited'] as const

/** Validator set: `xdevs_validators` when the node serves it (has uptime), otherwise read from the Staking predeploy. */
export async function rpcValidators(): Promise<ValidatorSet> {
  const c = rpcClient()
  try {
    const vs = await getValidators(c.base)
    return {
      via: 'rpc-validators',
      epoch: null,
      validators: vs.map((v) => ({ operator: getAddress(v.operator), consensusKey: v.consensusKey, status: v.status, totalStake: hx(v.totalStake), selfStake: null, votingPower: hx(v.votingPower), commissionBps: v.commissionBps, uptimeBps: v.uptimeBps, jailedUntil: null, inSigningSet: v.status === 'active', name: null })),
    }
  } catch (e) {
    if (!(e instanceof XdevsMethodNotAvailableError)) {
      // Unreachable node etc.: fall through only for "method not available"; rethrow network errors.
      if (!/method|not.*(found|supported|available)/i.test(String((e as Error).message))) throw e
    }
  }
  const S = predeploys.STAKING
  const [cands, signing, epoch] = await Promise.all([
    c.readContract({ address: S, abi: stakingAbi, functionName: 'candidates' }),
    c.readContract({ address: S, abi: stakingAbi, functionName: 'signingSet' }),
    c.readContract({ address: S, abi: stakingAbi, functionName: 'currentEpoch' }).catch(() => null),
  ])
  const power = new Map(signing[0].map((op, i) => [op.toLowerCase(), signing[1][i]!]))
  const all = [...new Set([...cands, ...signing[0]].map((x) => getAddress(x)))]
  const infos = await Promise.all(all.map((op) => c.readContract({ address: S, abi: stakingAbi, functionName: 'getValidator', args: [op] })))
  const validators: ValidatorRow[] = all.map((op, i) => {
    const v = infos[i]!
    const p = power.get(op.toLowerCase())
    return {
      operator: op, consensusKey: v.consensusKey, status: STATUS[v.status] ?? 'unknown', totalStake: hx(v.totalStake), selfStake: hx(v.selfStake),
      votingPower: p !== undefined ? hx(p) : '0x0', commissionBps: v.commissionBps, uptimeBps: null, jailedUntil: v.jailedUntil ? Number(v.jailedUntil) : null,
      inSigningSet: p !== undefined, name: null,
    }
  })
  return { via: 'staking', epoch: epoch !== null ? Number(epoch) : null, validators }
}

export { vaultAbi }

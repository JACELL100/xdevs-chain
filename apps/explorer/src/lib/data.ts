// One loader per page. Order: demo fixtures (NEXT_PUBLIC_DEMO=1) → indexer API → node RPC. Each result says where
// it came from so the page can label RPC-only views. Transport failures mark the service down (net.ts).

import { getAddress, isAddress, parseAbi, type Hex } from 'viem'
import { normalizeXnsName, predeploys, xnsNamehash } from '@xdevs/sdk'
import { api, ApiError } from './api.ts'
import { config } from './config.ts'
import { decodeCall, vaultActivity, type DecodedCall, type VaultActivity } from './decode.ts'
import * as demo from './fixtures.ts'
import { isUnreachable, setNet } from './net.ts'
import * as rpc from './rpc.ts'
import type { AddressSummary, Approval, BlockDetail, CommitDetail, NameInfo, Page, Source, TokenPage, TokenTransfer, TxDetail, TxSummary, ValidatorSet, VaultAction } from './types.ts'

export class NotFoundError extends Error {
  constructor(what: string) {
    super(what)
    this.name = 'NotFoundError'
  }
}

const DEMO_LATENCY = 220
const pause = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((res, rej) => {
    const t = setTimeout(res, ms)
    signal?.addEventListener('abort', () => (clearTimeout(t), rej(new DOMException('aborted', 'AbortError'))))
  })

/** Runs an API call; `null` when the API is unreachable (or was already found down) so the caller falls back. */
async function viaApi<T>(fn: () => Promise<T>): Promise<T | null | 'notfound'> {
  try {
    return await fn()
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) return 'notfound'
    if (isUnreachable(e) || (e instanceof ApiError && e.status >= 500)) return null
    throw e
  }
}

/** Runs an RPC call, marking the node up or down. */
async function viaRpc<T>(fn: () => Promise<T>): Promise<T> {
  try {
    const v = await fn()
    setNet({ rpc: 'up' })
    return v
  } catch (e) {
    if (isUnreachable(e)) setNet({ rpc: 'down' })
    throw e
  }
}

// --------------------------------------------------------------------------------------------------------------
// Validators (cached: the set changes once per epoch)
// --------------------------------------------------------------------------------------------------------------

let validatorCache: { at: number; p: Promise<ValidatorSet> } | null = null

export function loadValidators(): Promise<ValidatorSet> {
  if (config.demo) return pause(DEMO_LATENCY).then(() => demo.demoValidatorSet())
  if (validatorCache && Date.now() - validatorCache.at < 60_000) return validatorCache.p
  const p = viaRpc(async () => {
    const set = await rpc.rpcValidators()
    // Names via XNS reverse records (one batched round trip).
    const names = await Promise.all(set.validators.map((v) => rpc.rpcPrimaryName(v.operator)))
    set.validators.forEach((v, i) => (v.name = v.name ?? names[i] ?? null))
    return set
  })
  validatorCache = { at: Date.now(), p }
  p.catch(() => (validatorCache = null))
  return p
}

// --------------------------------------------------------------------------------------------------------------
// Blocks
// --------------------------------------------------------------------------------------------------------------

export interface BlockResult {
  block: BlockDetail
  commit: CommitDetail | null
  source: Source
}

export async function loadBlock(id: string, signal?: AbortSignal): Promise<BlockResult> {
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    const n = /^\d+$/.test(id) ? Number(id) : id === 'latest' ? demo.demoHead() : NaN
    if (!Number.isFinite(n) || n > demo.demoHead()) throw new NotFoundError(`Block ${id} has not been produced yet.`)
    return { block: demo.demoBlock(n), commit: demo.demoCommit(n), source: 'demo' }
  }
  const fromApi = await viaApi(() => api.block(id, signal))
  if (fromApi && fromApi !== 'notfound') {
    const commit = await rpc.rpcCommit(fromApi.number).catch(() => null)
    return { block: fromApi, commit, source: 'api' }
  }
  const block = await viaRpc(() => rpc.rpcBlock(id))
  if (!block) throw new NotFoundError(`Block ${id} was not found on this chain.`)
  const commit = await rpc.rpcCommit(block.number).catch(() => null)
  return { block, commit, source: 'rpc' }
}

// --------------------------------------------------------------------------------------------------------------
// Transactions
// --------------------------------------------------------------------------------------------------------------

export interface TxResult {
  tx: TxDetail
  call: DecodedCall | null
  vault: VaultActivity[]
  /** The tx calls a Guardian Vault's cancel/execute: which action it touched. */
  vaultCall: { account: string; kind: 'cancel' | 'execute'; id: bigint } | null
  source: Source
}

function enrichTx(tx: TxDetail, source: Source): TxResult {
  const call = decodeCall(tx.to, tx.input)
  const vault = vaultActivity(tx.logs)
  const vaultCall = call && call.standard === 'vault' && (call.name === 'cancel' || call.name === 'execute') && tx.to ? { account: tx.to, kind: call.name as 'cancel' | 'execute', id: call.args[0]!.value as bigint } : null
  return { tx, call, vault, vaultCall, source }
}

export async function loadTx(hash: string, signal?: AbortSignal): Promise<TxResult> {
  if (!/^0x[0-9a-fA-F]{64}$/.test(hash)) throw new NotFoundError(`"${hash}" is not a transaction hash.`)
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    const tx = demo.demoTx(hash)
    if (!tx) throw new NotFoundError('This transaction is not in the demo chain.')
    return enrichTx(tx, 'demo')
  }
  const fromApi = await viaApi(() => api.tx(hash, signal))
  if (fromApi && fromApi !== 'notfound') return enrichTx(fromApi, 'api')
  const tx = await viaRpc(() => rpc.rpcTx(hash))
  if (!tx) throw new NotFoundError('No transaction with this hash has been included in a block. Pending transactions are not shown.')
  return enrichTx(tx, 'rpc')
}

// --------------------------------------------------------------------------------------------------------------
// Addresses
// --------------------------------------------------------------------------------------------------------------

export interface AddressResult {
  summary: AddressSummary
  source: Source
}

export async function loadAddress(a: string, signal?: AbortSignal): Promise<AddressResult> {
  if (!isAddress(a, { strict: false })) throw new NotFoundError(`"${a}" is not an address.`)
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    return { summary: demo.demoAddress(a), source: 'demo' }
  }
  const fromApi = await viaApi(() => api.address(a, signal))
  if (fromApi && fromApi !== 'notfound') {
    // The API reads the lock from its own node; ask ours too when the API could not (indexer-only answer).
    return { summary: fromApi, source: 'api' }
  }
  return { summary: await viaRpc(() => rpc.rpcAddress(a)), source: 'rpc' }
}

type Paged<T> = Page<T> & { unavailable?: boolean }

export async function loadAddressTxs(a: string, cursor: string | null, includeSpam: boolean, signal?: AbortSignal): Promise<Paged<TxSummary>> {
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    return { items: demo.demoAddressTxs(a), nextCursor: null, hiddenSpam: null }
  }
  const r = await viaApi(() => api.addressTxs(a, { cursor, includeSpam }, signal))
  if (r && r !== 'notfound') return r
  return { items: [], nextCursor: null, unavailable: true }
}

export async function loadAddressTransfers(a: string, cursor: string | null, includeSpam: boolean, signal?: AbortSignal): Promise<Paged<TokenTransfer>> {
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    const items: TokenTransfer[] = []
    for (const t of demo.demoAddressTxs(a, 12)) for (const tr of demo.demoTx(t.hash)?.tokenTransfers ?? []) items.push(tr)
    return { items, nextCursor: null, hiddenSpam: null }
  }
  const r = await viaApi(() => api.addressTransfers(a, { cursor, includeSpam }, signal))
  if (r && r !== 'notfound') return r
  return { items: [], nextCursor: null, unavailable: true }
}

export async function loadApprovals(a: string, includeExpired: boolean, signal?: AbortSignal): Promise<{ items: Approval[]; live: boolean; unavailable?: boolean }> {
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    return { items: demo.demoApprovals(a), live: true }
  }
  const r = await viaApi(() => api.approvals(a, includeExpired, signal))
  if (r && r !== 'notfound') return r
  // Indexer down: the node's xdevs_getApprovals (bounded window, live allowances only).
  const items = await viaRpc(() => rpc.rpcApprovals(a)).catch(() => null)
  if (items) return { items, live: true }
  return { items: [], live: false, unavailable: true }
}

export async function loadVaultQueue(a: string, cursor: string | null, all: boolean, signal?: AbortSignal): Promise<Paged<VaultAction>> {
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    const items = demo.demoVaultQueue(a)
    return { items: all ? items : items.filter((x) => x.status === 'queued' || x.status === 'ready'), nextCursor: null }
  }
  const r = await viaApi(() => api.vaultQueue(a, { cursor, status: all ? 'all' : 'open' }, signal))
  if (r && r !== 'notfound') return r
  // Indexer down: pending actions straight from the account via the node's xdevs_getVaultQueue (no history).
  const items = await viaRpc(() => rpc.rpcVaultQueue(a)).catch(() => null)
  if (items) return { items, nextCursor: null }
  return { items: [], nextCursor: null, unavailable: true }
}

// --------------------------------------------------------------------------------------------------------------
// Tokens, names, search
// --------------------------------------------------------------------------------------------------------------

export async function loadToken(a: string, signal?: AbortSignal): Promise<TokenPage & { source: Source }> {
  if (!isAddress(a, { strict: false })) throw new NotFoundError(`"${a}" is not a token address.`)
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    const p = demo.demoTokenPage(a)
    if (!p) throw new NotFoundError('This address is not a token in the demo chain.')
    return { ...p, source: 'demo' }
  }
  const page = await viaRpc(async () => rpc.rpcToken(a, await rpc.rpcHead()))
  if (!page) throw new NotFoundError('This address does not answer the XDRC-20 / ERC-20 metadata calls.')
  // Prefer the indexer's transfer history when it is up (complete, with spam flags).
  const fromApi = await viaApi(() => api.addressTransfers(a, { token: a, includeSpam: true }, signal)).catch(() => null)
  if (fromApi && fromApi !== 'notfound' && fromApi.items.length) return { ...page, transfers: fromApi.items, scannedBlocks: null, source: 'api' }
  return { ...page, source: 'rpc' }
}

const resolverAbi = parseAbi(['function addr(bytes32 node) view returns (address)'])

export async function loadName(name: string, signal?: AbortSignal): Promise<{ info: NameInfo | null; resolved: string | null; source: Source }> {
  let n: string
  try {
    n = normalizeXnsName(name)
  } catch {
    throw new NotFoundError(`"${name}" is not a valid .xdevs name.`)
  }
  if (config.demo) {
    await pause(DEMO_LATENCY, signal)
    const info = demo.demoName(n)
    return { info, resolved: info?.address ?? null, source: 'demo' }
  }
  const fromApi = await viaApi(() => api.name(n, signal))
  if (fromApi === 'notfound') return { info: null, resolved: null, source: 'api' }
  if (fromApi) return { info: fromApi, resolved: fromApi.address, source: 'api' }
  const resolved = await viaRpc(() =>
    rpc.rpcClient().readContract({ address: predeploys.XNS_REGISTRY, abi: resolverAbi, functionName: 'addr', args: [xnsNamehash(n) as Hex] }).catch((e) => {
      if (isUnreachable(e)) throw e
      return null
    }),
  )
  return { info: null, resolved: resolved && !/^0x0{40}$/.test(resolved) ? getAddress(resolved) : null, source: 'rpc' }
}

// --------------------------------------------------------------------------------------------------------------
// Home
// --------------------------------------------------------------------------------------------------------------

export async function loadApiStats(signal?: AbortSignal) {
  if (config.demo) return { ...demo.DEMO_STATS(demo.demoHead()), source: 'demo' as const }
  const s = await viaApi(() => api.stats(signal))
  if (!s || s === 'notfound') return null
  return { totalTransactions: s.totalTransactions, totalAddresses: s.totalAddresses, burnedFees: s.burnedFees, treasuryFees: s.treasuryFees, source: 'api' as const }
}

export async function loadLatestTxs(limit: number, signal?: AbortSignal): Promise<TxSummary[] | null> {
  if (config.demo) return demo.demoLatestTxs(demo.demoHead(), limit)
  const r = await viaApi(() => api.txs(null, limit, signal))
  return r && r !== 'notfound' ? r.items : null
}

export async function loadVaultAction(account: string, id: bigint): Promise<VaultAction | null> {
  if (config.demo) return demo.demoVaultQueue(account).find((v) => BigInt(v.id) === id) ?? null
  const r = await viaApi(() => api.vaultQueue(account, { status: 'all' }))
  if (!r || r === 'notfound') return null
  return r.items.find((v) => BigInt(v.id) === id) ?? null
}

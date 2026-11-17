// Live backend: the chain (JSON-RPC), the ERC-4337 bundler and, when configured, the explorer API.
import { type Address, type Hex, getAddress } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { checkXnsLabel } from '@xdevs/sdk'
import { config, publicClient, rpId } from '../config.ts'
import { accountAbi, erc20Abi, xnsAbi } from '../lib/abi.ts'
import {
  type AccountState,
  type Call,
  type InitParams,
  SMART_ACCOUNT_FACTORY,
  XNS_REGISTRY,
  callsToCallData,
  counterfactualAddress,
  factoryData,
  readAccountState,
} from '../lib/account.ts'
import { listApprovals } from '../lib/approvals.ts'
import { previewTransaction } from '../lib/preview.ts'
import { type RecipientCheckResult, type ResolvedRecipient, checkRecipient, resolveRecipient } from '../lib/recipient.ts'
import { sendUserOp } from '../lib/send.ts'
import { type OwnerSigner, SoftPasskey, ecdsaSigner, platformPasskeySigner } from '../lib/signers.ts'
import { checkVaultOutcome, listVaultQueue, listVaultQueueFromApi } from '../lib/vault.ts'
import { type AccountRecord, localActivity, recordActivity } from '../store.ts'
import type { ActivityItem, Asset, Backend, ChainStatus, OpInput, OpResult, RecoveryStatus, Review, XnsQuote } from './types.ts'

export const XNS_YEAR = 365n * 86_400n

const nowSec = () => BigInt(Math.floor(Date.now() / 1000))
const firstLine = (e: unknown) => ((e as { shortMessage?: string }).shortMessage ?? (e as Error).message ?? String(e)).split('\n')[0]

function withTimeout<T>(p: Promise<T>, ms: number, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`${what} did not answer within ${ms / 1000}s`)), ms)
    p.then(
      (v) => {
        clearTimeout(t)
        resolve(v)
      },
      (e) => {
        clearTimeout(t)
        reject(e)
      },
    )
  })
}

/** Signer for an account record, or null for watch-only accounts. */
export async function signerFor(rec: AccountRecord): Promise<OwnerSigner | null> {
  if (rec.kind === 'passkey' && rec.passkey) return platformPasskeySigner(rec.passkey)
  if (rec.kind === 'virtual' && rec.virtualJwk) return (await SoftPasskey.fromJwk(rec.virtualJwk, rpId(), location.origin)).signer()
  if (rec.kind === 'dev-key' && rec.devKey) return ecdsaSigner(privateKeyToAccount(rec.devKey), 'Dev key (EIP-7702 EOA)')
  return null
}

type ApiTransfer = {
  timestamp: number
  txHash: Hex
  logIndex: number | null
  kind: 'native' | 'erc20'
  token: { address: Address | null; symbol: string | null; decimals: number | null }
  from: Address
  to: Address
  value: Hex
  spam: boolean
  lookAlikeOf: Address | null
}

async function activityFromApi(apiUrl: string, address: Address): Promise<ActivityItem[]> {
  const res = await fetch(`${apiUrl.replace(/\/$/, '')}/api/v1/addresses/${address}/token-transfers?limit=50`)
  if (!res.ok) throw new Error(`explorer API: HTTP ${res.status}`)
  const j = (await res.json()) as { items: ApiTransfer[] }
  return j.items.map((t) => {
    const out = t.from.toLowerCase() === address.toLowerCase()
    const symbol = t.token.symbol ?? (t.kind === 'native' ? 'tXDS' : 'token')
    return {
      id: `${t.txHash}-${t.logIndex ?? 'n'}`,
      kind: out ? 'sent' : 'received',
      title: out ? `Sent ${symbol}` : `Received ${symbol}`,
      counterparty: getAddress(out ? t.to : t.from),
      amount: BigInt(t.value),
      symbol,
      decimals: t.token.decimals ?? 18,
      timestamp: t.timestamp,
      status: 'confirmed',
      hash: t.txHash,
      spam: t.spam,
      lookAlikeOf: t.lookAlikeOf,
    } satisfies ActivityItem
  })
}

export const liveBackend: Backend = {
  demo: false,

  async chainStatus(): Promise<ChainStatus> {
    try {
      const [chainId, block] = await withTimeout(Promise.all([publicClient.getChainId(), publicClient.getBlockNumber()]), 6_000, 'The RPC')
      if (chainId !== config.chainId) return { kind: 'wrong-chain', chainId }
      return { kind: 'ok', chainId, block }
    } catch (e) {
      return { kind: 'unreachable', reason: firstLine(e) }
    }
  },

  readAccount: (address) => readAccountState(publicClient, address),

  async assets(address, nativeBalance) {
    const out: Asset[] = [{ token: null, symbol: config.symbol, name: config.coinName, decimals: 18, balance: nativeBalance }]
    const rows = await Promise.allSettled(
      config.knownTokens.map(async (token) => {
        const [symbol, name, decimals, balance] = await Promise.all([
          publicClient.readContract({ address: token, abi: erc20Abi, functionName: 'symbol' }),
          publicClient.readContract({ address: token, abi: erc20Abi, functionName: 'name' }),
          publicClient.readContract({ address: token, abi: erc20Abi, functionName: 'decimals' }),
          publicClient.readContract({ address: token, abi: erc20Abi, functionName: 'balanceOf', args: [address] }),
        ])
        return { token, symbol, name, decimals: Number(decimals), balance } satisfies Asset
      }),
    )
    for (const r of rows) if (r.status === 'fulfilled') out.push(r.value)
    return out
  },

  async activity(address) {
    const local = localActivity(address)
    if (!config.apiUrl) return local
    try {
      const api = await activityFromApi(config.apiUrl, address)
      const hashes = new Set(api.map((a) => a.hash))
      return [...api, ...local.filter((l) => !l.hash || !hashes.has(l.hash))].sort((a, b) => b.timestamp - a.timestamp)
    } catch {
      return local
    }
  },

  async vaultQueue(address) {
    if (config.apiUrl) {
      try {
        return await listVaultQueueFromApi(config.apiUrl, address)
      } catch {
        /* fall back to the chain */
      }
    }
    return listVaultQueue(publicClient, address, nowSec())
  },

  async approvals(address) {
    if (config.apiUrl) {
      try {
        return await listApprovals(publicClient, address, { apiUrl: config.apiUrl, knownTokens: config.knownTokens })
      } catch {
        /* explorer API down: fall back to the node / a local scan */
      }
    }
    return listApprovals(publicClient, address, { apiUrl: null, knownTokens: config.knownTokens })
  },

  // Social recovery status from the account's events (bounded window: the node caps eth_getLogs at 10k blocks).
  async recoveryStatus(address): Promise<RecoveryStatus> {
    try {
      const latest = await publicClient.getBlockNumber()
      const fromBlock = latest > 9_000n ? latest - 9_000n : 0n
      const logs = await publicClient.getContractEvents({ address, abi: accountAbi, fromBlock, toBlock: latest })
      const rel = logs.filter((l) => ['RecoveryApproved', 'RecoveryReady', 'RecoveryExecuted', 'RecoveryCancelled'].includes(l.eventName))
      const last = rel[rel.length - 1]
      if (!last) return { kind: 'none', text: 'No recovery in progress.' }
      if (last.eventName === 'RecoveryApproved')
        return { kind: 'proposed', text: `A recovery was proposed: ${(last.args as { approvals: number }).approvals} guardian approval(s) so far.` }
      if (last.eventName === 'RecoveryReady')
        return {
          kind: 'ready',
          text: `A recovery reached the quorum and can execute from ${new Date(Number((last.args as { readyAt: bigint }).readyAt) * 1000).toLocaleString()}. If this is not you, cancel it now.`,
        }
      if (last.eventName === 'RecoveryExecuted') return { kind: 'executed', text: 'A recovery was executed: the signer set was replaced.' }
      return { kind: 'cancelled', text: 'The last recovery was cancelled by the owner.' }
    } catch (e) {
      return { kind: 'unknown', text: `Recovery status unavailable (${firstLine(e)}).` }
    }
  },

  resolveRecipient: (input) => resolveRecipient(publicClient, input),
  checkRecipient: (from, to, history) => checkRecipient(publicClient, from, to, history),

  async review(state: AccountState, call: Call, recipient: ResolvedRecipient | null, check: RecipientCheckResult | null): Promise<Review> {
    const [preview, vault] = await Promise.all([
      previewTransaction(publicClient, { from: state.address, to: call.target, value: call.value, data: call.data }, { recipient: check, balance: state.balance, nativeSymbol: config.symbol }),
      checkVaultOutcome(publicClient, state, [call]),
    ])
    return { call, recipient, check, preview, vault }
  },

  async tokenDecimals(token) {
    return Number(await publicClient.readContract({ address: token, abi: erc20Abi, functionName: 'decimals' }))
  },

  counterfactualAddress: (params: InitParams, salt: Hex) => counterfactualAddress(publicClient, params, salt),

  async primaryName(address) {
    try {
      const n = await publicClient.readContract({ address: XNS_REGISTRY, abi: xnsAbi, functionName: 'primaryName', args: [address] })
      return n || null
    } catch {
      return null
    }
  },

  async xnsQuote(label): Promise<XnsQuote> {
    const name = `${label}.xdevs`
    const err = checkXnsLabel(label)
    if (err) return { name, available: false, price: 0n, reason: err }
    const [available, price] = await Promise.all([
      publicClient.readContract({ address: XNS_REGISTRY, abi: xnsAbi, functionName: 'available', args: [label] }),
      publicClient.readContract({ address: XNS_REGISTRY, abi: xnsAbi, functionName: 'rentPrice', args: [XNS_YEAR] }),
    ])
    return { name, available, price, reason: available ? undefined : 'taken, reserved, or too similar to an existing name' }
  },

  async sendOp(rec: AccountRecord, input: OpInput, onStatus): Promise<OpResult> {
    const signer = await signerFor(rec)
    if (!signer) throw new Error('This account is watch-only in this browser: there is no passkey to sign with.')
    const r = await sendUserOp({
      client: publicClient,
      bundlerUrl: config.bundlerUrl,
      chainId: config.chainId,
      sender: rec.address,
      callData: input.callData ?? callsToCallData(input.calls ?? []),
      signer,
      factory: rec.params && rec.salt ? { address: SMART_ACCOUNT_FACTORY, data: factoryData(rec.params, rec.salt) } : null,
      sponsor: input.sponsored ? config.paymasterSponsor : null,
      onStatus,
    })
    const queued = r.queuedIds.length > 0
    recordActivity(rec.address, {
      ...input.describe,
      id: r.userOpHash,
      timestamp: Math.floor(Date.now() / 1000),
      status: queued ? 'queued' : 'confirmed',
      kind: queued ? 'queued' : input.describe.kind,
      hash: r.receipt.receipt.transactionHash,
    })
    return { userOpHash: r.userOpHash, txHash: r.receipt.receipt.transactionHash, queuedIds: r.queuedIds }
  },
}

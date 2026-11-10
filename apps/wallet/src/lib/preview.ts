// Pre-sign preview (plan §16.4): `xdevs_simulateTransaction` when the node has it, else a local preview
// (eth_call + decoding of approvals / transfers). Either way the result is ADVICE, NOT FACT.
import {
  type Address,
  type Hex,
  type PublicClient,
  decodeFunctionData,
  formatUnits,
  isAddressEqual,
  maxUint256,
} from 'viem'
import {
  type SimulationResult,
  type XdevsApprovalChange,
  type XdevsAssetChange,
  type XdevsWarning,
  XdevsMethodNotAvailableError,
  isMethodNotFoundError,
  simulateTransaction,
} from '@xdevs/sdk'
import { erc20Abi, nftAbi, permit2Abi, xdrc20Abi } from './abi.ts'
import { PERMIT2_EXPIRING, UNLIMITED_APPROVAL } from './account.ts'
import type { RecipientCheckResult } from './recipient.ts'

export const ADVICE_DISCLAIMER =
  'Advice, not fact: this preview is a best-effort simulation against the current state. The result can differ if ' +
  'state changes before inclusion, and warnings are heuristics. Always check the recipient and the amount yourself.'

export type PreviewTx = { from: Address; to: Address; value: bigint; data: Hex }

export type Preview = SimulationResult & {
  source: 'node' | 'local'
  /** What the call does, in one line per decoded action. */
  summary: string[]
}

type TokenMeta = { symbol: string; decimals: number }

async function tokenMeta(client: PublicClient, token: Address): Promise<TokenMeta> {
  try {
    const [symbol, decimals] = await Promise.all([
      client.readContract({ address: token, abi: erc20Abi, functionName: 'symbol' }),
      client.readContract({ address: token, abi: erc20Abi, functionName: 'decimals' }).catch(() => 18),
    ])
    return { symbol, decimals: Number(decimals) }
  } catch {
    return { symbol: `${token.slice(0, 8)}…`, decimals: 18 }
  }
}

const isUnlimited = (amount: bigint) => amount >= UNLIMITED_APPROVAL || amount === maxUint256

/**
 * Local preview of one call: decodes ERC-20 / XDRC-20 / Permit2 / NFT approvals and transfers, flags unlimited
 * approvals and operator approvals, and checks with eth_call whether the call would revert.
 */
export async function localPreview(
  client: PublicClient,
  tx: PreviewTx,
  opts: { recipient?: RecipientCheckResult | null; balance?: bigint; nativeSymbol?: string } = {},
): Promise<Preview> {
  const sym = opts.nativeSymbol ?? 'tXDS'
  const warnings: XdevsWarning[] = []
  const assetChanges: XdevsAssetChange[] = []
  const approvals: XdevsApprovalChange[] = []
  const summary: string[] = []

  if (tx.value > 0n) {
    assetChanges.push({ token: 'native', symbol: sym, from: tx.from, to: tx.to, amount: formatUnits(tx.value, 18) })
    summary.push(`Send ${formatUnits(tx.value, 18)} ${sym} to ${tx.to}`)
    if (opts.balance !== undefined && opts.balance > 0n && tx.value * 2n > opts.balance) {
      warnings.push({ code: 'HIGH_VALUE_TRANSFER', severity: 'medium', message: `This sends more than half of your ${sym} balance.` })
    }
  }

  if (tx.data.length >= 10) {
    let decoded: { functionName: string; args?: readonly unknown[] } | null = null
    try {
      decoded = decodeFunctionData({ abi: [...erc20Abi, ...xdrc20Abi, ...nftAbi, ...permit2Abi], data: tx.data })
    } catch {
      decoded = null
    }
    const args = (decoded?.args ?? []) as readonly unknown[]
    const fn = decoded?.functionName
    if (fn === 'approve' && args.length === 4 && isAddressEqual(tx.to, PERMIT2_EXPIRING)) {
      const [token, spender, amount, expiration] = args as [Address, Address, bigint, number]
      const m = await tokenMeta(client, token)
      const unlimited = amount >= UNLIMITED_APPROVAL
      approvals.push({ token, spender, amount: unlimited ? 'unlimited' : formatUnits(amount, m.decimals), expires: Number(expiration) || null })
      summary.push(`Permit2Expiring: let ${spender} spend ${unlimited ? 'ALL' : formatUnits(amount, m.decimals)} ${m.symbol} until ${new Date(Number(expiration) * 1000).toISOString()}`)
      if (unlimited && amount > 0n) {
        warnings.push({ code: 'UNLIMITED_APPROVAL', severity: 'high', message: `This lets ${spender} spend ALL your ${m.symbol} through Permit2 until it expires.` })
      }
    } else if (fn === 'approve' || fn === 'increaseAllowance') {
      const [spender, amount] = args as [Address, bigint]
      let expiry = args.length === 3 ? Number(args[2]) : null
      const m = await tokenMeta(client, tx.to)
      if (expiry === null) {
        // XDRC-20 (xns-and-approvals.md §3): even the legacy approve expires after defaultApprovalDuration.
        const d = await client.readContract({ address: tx.to, abi: xdrc20Abi, functionName: 'defaultApprovalDuration' }).catch(() => null)
        if (d) expiry = Math.floor(Date.now() / 1000) + Number(d)
      }
      if (amount === 0n) {
        summary.push(`Revoke ${spender}'s allowance on ${m.symbol}`)
        approvals.push({ token: tx.to, spender, amount: '0', expires: null })
      } else {
        const unlimited = isUnlimited(amount)
        approvals.push({ token: tx.to, spender, amount: unlimited ? 'unlimited' : formatUnits(amount, m.decimals), expires: expiry })
        summary.push(`${fn === 'increaseAllowance' ? 'Increase' : 'Allow'} ${spender} to spend ${unlimited ? 'ALL your' : formatUnits(amount, m.decimals)} ${m.symbol}${expiry ? ` until ${new Date(expiry * 1000).toISOString()}` : ''}`)
        if (unlimited) {
          warnings.push({
            code: 'UNLIMITED_APPROVAL',
            severity: 'high',
            message: `This gives ${spender} permission to spend ALL your ${m.symbol}${expiry ? ` until ${new Date(expiry * 1000).toISOString().slice(0, 10)} (the approval expires)` : ' (on a plain ERC-20: forever, until you revoke it)'}.`,
          })
        }
      }
    } else if (fn === 'setApprovalForAll') {
      const [operator, approved] = args as [Address, boolean]
      summary.push(approved ? `Let ${operator} move ALL your NFTs of ${tx.to}` : `Revoke operator ${operator} on ${tx.to}`)
      if (approved) {
        approvals.push({ token: tx.to, spender: operator, amount: 'unlimited', expires: null })
        warnings.push({ code: 'SET_APPROVAL_FOR_ALL', severity: 'high', message: `This gives ${operator} control of ALL your tokens in collection ${tx.to}. Scammers use this to drain NFTs.` })
      }
    } else if (fn === 'transfer') {
      const [to, amount] = args as [Address, bigint]
      const m = await tokenMeta(client, tx.to)
      assetChanges.push({ token: tx.to, symbol: m.symbol, from: tx.from, to, amount: formatUnits(amount, m.decimals) })
      summary.push(`Send ${formatUnits(amount, m.decimals)} ${m.symbol} to ${to}`)
    } else if (fn === 'transferFrom' || fn === 'safeTransferFrom') {
      const [from, to, amount] = args as [Address, Address, bigint]
      summary.push(`Move token ${String(amount)} of ${tx.to} from ${from} to ${to}`)
      if (isAddressEqual(from, tx.from)) assetChanges.push({ token: tx.to, symbol: (await tokenMeta(client, tx.to)).symbol, from, to, amount: String(amount) })
    } else if (fn === 'transferOwnership') {
      summary.push(`Transfer ownership of ${tx.to} to ${String(args[0])}`)
      warnings.push({ code: 'OWNERSHIP_TRANSFER', severity: 'high', message: `This hands ownership of contract ${tx.to} to ${String(args[0])}.` })
    } else if (fn === 'revokeAll') {
      summary.push(`Revoke every approval you granted on ${tx.to}`)
    } else {
      summary.push(`Call ${tx.data.slice(0, 10)} on ${tx.to} (not decoded)`)
      warnings.push({ code: 'UNVERIFIED_CONTRACT', severity: 'low', message: 'The wallet cannot decode this call. Only continue if you trust the app that built it.' })
    }
  }

  if (opts.recipient) {
    if (opts.recipient.lookAlikeOf.length > 0) {
      warnings.push({ code: 'LOOKALIKE_RECIPIENT', severity: 'high', message: `The recipient looks like an address you used before (${opts.recipient.lookAlikeOf.join(', ')}) but is different. This is how address poisoning works.` })
    }
    if (opts.recipient.firstTime) {
      warnings.push({ code: 'FIRST_TIME_RECIPIENT', severity: 'low', message: 'You have never sent to this address before.' })
    }
  }

  let success = true
  let revertReason: string | null = null
  let gasUsed = 0n
  try {
    await client.call({ account: tx.from, to: tx.to, value: tx.value, data: tx.data })
    gasUsed = await client.estimateGas({ account: tx.from, to: tx.to, value: tx.value, data: tx.data }).catch(() => 0n)
  } catch (e) {
    success = false
    revertReason = ((e as { shortMessage?: string }).shortMessage ?? (e as Error).message).split('\n')[0]
  }
  return { success, gasUsed, assetChanges, approvals, warnings, disclaimer: ADVICE_DISCLAIMER, revertReason, source: 'local', summary }
}

/** Node simulation when available (`xdevs_simulateTransaction`), otherwise the local preview. */
export async function previewTransaction(
  client: PublicClient,
  tx: PreviewTx,
  opts: { recipient?: RecipientCheckResult | null; balance?: bigint; nativeSymbol?: string } = {},
): Promise<Preview> {
  const local = await localPreview(client, tx, opts)
  try {
    const sim = await simulateTransaction(client, { account: tx.from, to: tx.to, value: tx.value, data: tx.data })
    // Keep the local recipient warnings (the node does not know the wallet's address book).
    const codes = new Set(sim.warnings.map((w) => w.code))
    const extra = local.warnings.filter((w) => !codes.has(w.code) && (w.code === 'LOOKALIKE_RECIPIENT' || w.code === 'FIRST_TIME_RECIPIENT'))
    return { ...sim, warnings: [...sim.warnings, ...extra], disclaimer: `${sim.disclaimer} ${ADVICE_DISCLAIMER}`, source: 'node', summary: local.summary }
  } catch (e) {
    if (e instanceof XdevsMethodNotAvailableError || isMethodNotFoundError(e)) return local
    return local
  }
}

export const hasHighSeverity = (p: Pick<Preview, 'warnings'>) => p.warnings.some((w) => w.severity === 'high')

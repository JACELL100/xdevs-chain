// Approvals manager (plan §16.5): list live approvals and build revoke calls.
// Sources, in order: explorer API (VITE_API_URL), node `xdevs_getApprovals`, local log scan of known tokens.
import { type Address, type Hex, type PublicClient, encodeFunctionData, getAddress, parseAbiItem } from 'viem'
import { XdevsMethodNotAvailableError, getApprovals as sdkGetApprovals, isMethodNotFoundError } from '@xdevs/sdk'
import { erc20Abi, permit2Abi, xdrc20Abi } from './abi.ts'
import { type Call, PERMIT2_EXPIRING, UNLIMITED_APPROVAL } from './account.ts'

export type ApprovalItem = {
  kind: 'erc20' | 'xdrc20' | 'permit2'
  token: Address
  symbol: string | null
  spender: Address
  /** Remaining allowance (live read when possible). */
  amount: bigint
  unlimited: boolean
  /** Unix seconds, or null = never expires (plain ERC-20). */
  expiry: number | null
  source: 'api' | 'node' | 'local'
}

const notAvailable = (e: unknown) => e instanceof XdevsMethodNotAvailableError || isMethodNotFoundError(e)

/** Explorer API (docs/specs/explorer-api.md: GET /api/v1/addresses/:address/approvals). */
export async function approvalsFromApi(apiUrl: string, owner: Address): Promise<ApprovalItem[]> {
  const res = await fetch(`${apiUrl.replace(/\/$/, '')}/api/v1/addresses/${owner}/approvals`)
  if (!res.ok) throw new Error(`explorer API: HTTP ${res.status}`)
  const j = (await res.json()) as {
    items: {
      kind: ApprovalItem['kind']
      token: { address: Address | null; symbol: string | null }
      spender: Address
      amount: Hex
      unlimited: boolean
      expiry: number | null
      remaining: Hex | null
    }[]
  }
  return j.items
    .filter((i) => i.token.address)
    .map((i) => ({
      kind: i.kind,
      token: getAddress(i.token.address as Address),
      symbol: i.token.symbol,
      spender: getAddress(i.spender),
      amount: BigInt(i.remaining ?? i.amount),
      unlimited: i.unlimited,
      expiry: i.expiry,
      source: 'api' as const,
    }))
}

const approvalEvent = parseAbiItem('event Approval(address indexed owner, address indexed spender, uint256 value)')
const permit2ApprovalEvent = parseAbiItem(
  'event Approval(address indexed owner, address indexed token, address indexed spender, uint160 amount, uint48 expiration)',
)
const permit2PermitEvent = parseAbiItem(
  'event Permit(address indexed owner, address indexed token, address indexed spender, uint160 amount, uint48 expiration, uint48 nonce)',
)

async function isXdrc20(client: PublicClient, token: Address, owner: Address): Promise<boolean> {
  try {
    await client.readContract({ address: token, abi: xdrc20Abi, functionName: 'approvalEpoch', args: [owner] })
    return true
  } catch {
    return false
  }
}

/**
 * Local scan: Approval logs of `knownTokens` and Permit2Expiring for `owner` over the last `lookback` blocks (the node
 * caps eth_getLogs at 10,000 blocks), then live `allowance()` reads. Misses approvals older than the window and
 * tokens the wallet does not know — use the explorer API for completeness.
 */
export async function approvalsFromLogs(
  client: PublicClient,
  owner: Address,
  knownTokens: Address[],
  lookback = 9_000n,
): Promise<ApprovalItem[]> {
  const latest = await client.getBlockNumber()
  const fromBlock = latest > lookback ? latest - lookback : 0n
  const out: ApprovalItem[] = []
  for (const token of knownTokens) {
    const logs = await client.getLogs({ address: token, event: approvalEvent, args: { owner }, fromBlock, toBlock: latest })
    const spenders = [...new Set(logs.map((l) => getAddress(l.args.spender as Address)))]
    if (spenders.length === 0) continue
    const [symbol, xdrc] = await Promise.all([
      client.readContract({ address: token, abi: erc20Abi, functionName: 'symbol' }).catch(() => null),
      isXdrc20(client, token, owner),
    ])
    for (const spender of spenders) {
      const amount = await client.readContract({ address: token, abi: erc20Abi, functionName: 'allowance', args: [owner, spender] })
      if (amount === 0n) continue
      const expiry = xdrc
        ? Number(await client.readContract({ address: token, abi: xdrc20Abi, functionName: 'allowanceExpiry', args: [owner, spender] }))
        : null
      out.push({ kind: xdrc ? 'xdrc20' : 'erc20', token, symbol, spender, amount, unlimited: amount >= UNLIMITED_APPROVAL, expiry, source: 'local' })
    }
  }
  const p2 = [
    ...(await client.getLogs({ address: PERMIT2_EXPIRING, event: permit2ApprovalEvent, args: { owner }, fromBlock, toBlock: latest })),
    ...(await client.getLogs({ address: PERMIT2_EXPIRING, event: permit2PermitEvent, args: { owner }, fromBlock, toBlock: latest })),
  ]
  const pairs = new Map<string, [Address, Address]>()
  for (const l of p2) {
    const t = getAddress(l.args.token as Address)
    const s = getAddress(l.args.spender as Address)
    pairs.set(`${t}-${s}`, [t, s])
  }
  for (const [token, spender] of pairs.values()) {
    const [amount, expiration] = await client.readContract({ address: PERMIT2_EXPIRING, abi: permit2Abi, functionName: 'allowance', args: [owner, token, spender] })
    if (amount === 0n) continue
    const symbol = await client.readContract({ address: token, abi: erc20Abi, functionName: 'symbol' }).catch(() => null)
    out.push({ kind: 'permit2', token, symbol, spender, amount, unlimited: amount >= UNLIMITED_APPROVAL, expiry: Number(expiration), source: 'local' })
  }
  return out
}

/** Explorer API if configured, else node `xdevs_getApprovals`, else the local log scan. */
export async function listApprovals(
  client: PublicClient,
  owner: Address,
  o: { apiUrl?: string | null; knownTokens: Address[] },
): Promise<ApprovalItem[]> {
  if (o.apiUrl) return approvalsFromApi(o.apiUrl, owner)
  try {
    const items = await sdkGetApprovals(client, { owner })
    return items.map((i) => ({
      kind: i.kind === 'permit2' ? 'permit2' : i.kind === 'xdrc20' ? 'xdrc20' : 'erc20',
      token: i.token,
      symbol: i.symbol ?? null,
      spender: i.spender,
      amount: i.amount === 'unlimited' ? UNLIMITED_APPROVAL : BigInt(i.amount),
      unlimited: i.amount === 'unlimited',
      expiry: i.expires,
      source: 'node' as const,
    })) as ApprovalItem[]
  } catch (e) {
    if (!notAvailable(e)) throw e
  }
  return approvalsFromLogs(client, owner, o.knownTokens)
}

/** Revokes one approval. Always instant under the Guardian Vault (amount 0 is a revocation). */
export function revokeCall(a: ApprovalItem): Call {
  if (a.kind === 'permit2') {
    return {
      target: PERMIT2_EXPIRING,
      value: 0n,
      data: encodeFunctionData({ abi: permit2Abi, functionName: 'approve', args: [a.token, a.spender, 0n, 0] }),
    }
  }
  return { target: a.token, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [a.spender, 0n] }) }
}

/**
 * "Revoke all". With the vault on, per-pair revocations (approve 0) are used because they are on the vault's
 * instant path; the O(1) epoch bumps (XDRC-20 `revokeAll()`, Permit2Expiring `revokeAll()` / `lockdown`) are not
 * recognised as revocations by the account's CallClassifier and would be QUEUED for the vault delay.
 */
export function revokeAllCalls(items: ApprovalItem[], vaultEnabled: boolean): Call[] {
  if (vaultEnabled) return items.map(revokeCall)
  const calls: Call[] = []
  const xdrcTokens = new Set<Address>()
  let permit2 = false
  for (const a of items) {
    if (a.kind === 'xdrc20') xdrcTokens.add(a.token)
    else if (a.kind === 'permit2') permit2 = true
    else calls.push(revokeCall(a))
  }
  for (const t of xdrcTokens) calls.push({ target: t, value: 0n, data: encodeFunctionData({ abi: xdrc20Abi, functionName: 'revokeAll' }) })
  if (permit2) calls.push({ target: PERMIT2_EXPIRING, value: 0n, data: encodeFunctionData({ abi: permit2Abi, functionName: 'revokeAll' }) })
  return calls
}

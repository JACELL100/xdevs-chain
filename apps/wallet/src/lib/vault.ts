// Guardian Vault: will a call be queued? queue listing, guardian votes, policy-change helpers.
import {
  type Address,
  type Hex,
  type PublicClient,
  decodeErrorResult,
  decodeFunctionData,
  encodeFunctionData,
  isAddressEqual,
  toFunctionSelector,
} from 'viem'
import { accountAbi, erc20Abi, nftAbi, permit2Abi, xdrc20Abi } from './abi.ts'
import { type AccountState, type Call, ENTRY_POINT, UNLIMITED_APPROVAL, VAULT_EXECUTION_WINDOW, type VaultPolicy, callsToCallData } from './account.ts'

export type VaultOutcome =
  | { kind: 'instant'; reason: string }
  | { kind: 'queued'; reason: string; delay: bigint }
  | { kind: 'rejected'; reason: string }
  | { kind: 'unknown'; reason: string }

/** Instant self-call selectors (XdevsSmartAccount._isInstantSelfSelector). */
const INSTANT_SELF = new Set(
  [
    'addSigner((uint8,bytes32,bytes32))',
    'addSessionKey((address,uint48,uint48,uint128,(address,bytes4)[],(address,uint128)[]))',
    'revokeSessionKey(address)',
    'revokeAllSessionKeys()',
    'cancel(uint256)',
    'cancelRecovery()',
    'revokeHash(bytes32)',
    'enableEoaLock()',
    'pokeGasBudget()',
  ].map((sig) => toFunctionSelector(sig)),
)

function classifyOne(self: Address, policy: VaultPolicy, whitelist: Address[], c: Call): { queue: boolean; reject?: string; why: string; native: bigint } {
  const wl = (a: Address) => whitelist.some((w) => isAddressEqual(w, a))
  if (isAddressEqual(c.target, self)) {
    const sel = c.data.slice(0, 10) as Hex
    const instant = c.value === 0n && INSTANT_SELF.has(sel)
    return { queue: !instant, why: instant ? 'account management (instant list)' : 'account setting change (always delayed)', native: 0n }
  }
  const trusted = wl(c.target)
  const native = c.value !== 0n && !trusted ? c.value : 0n
  if (c.data.length >= 10) {
    try {
      const d = decodeFunctionData({ abi: [...erc20Abi, ...xdrc20Abi, ...nftAbi, ...permit2Abi], data: c.data })
      const args = (d.args ?? []) as readonly unknown[]
      if (d.functionName === 'approve' || d.functionName === 'increaseAllowance') {
        const permit2 = args.length === 4
        const spender = (permit2 ? args[1] : args[0]) as Address
        const amount = (permit2 ? args[2] : args[1]) as bigint
        if (amount === 0n) return { queue: false, why: 'revocation (always instant)', native }
        if (wl(spender)) return { queue: false, why: 'approval to a whitelisted spender', native }
        if (amount >= UNLIMITED_APPROVAL && policy.blockUnlimitedApprovals) {
          return { queue: true, reject: `unlimited approval to ${spender} is blocked by your allowance policy`, why: '', native }
        }
        return { queue: true, why: 'approval to a non-whitelisted spender', native }
      }
      if (d.functionName === 'setApprovalForAll') {
        if (args[1] === false) return { queue: false, why: 'revocation (always instant)', native }
        if (wl(args[0] as Address)) return { queue: false, why: 'operator is whitelisted', native }
        if (policy.blockUnlimitedApprovals) return { queue: true, reject: 'setApprovalForAll(true) is blocked by your allowance policy', why: '', native }
        return { queue: true, why: 'operator approval for ALL tokens', native }
      }
      if (d.functionName === 'transfer') {
        if (wl(args[0] as Address)) return { queue: false, why: 'recipient is whitelisted', native }
        return { queue: true, why: 'token transfer to a non-whitelisted recipient (instant only within a per-token limit)', native }
      }
    } catch {
      /* unknown selector */
    }
    if (!trusted) return { queue: true, why: 'call to a non-whitelisted contract', native }
    return { queue: false, why: 'whitelisted contract', native }
  }
  return { queue: false, why: trusted ? 'whitelisted recipient' : 'native transfer', native }
}

/**
 * Local explanation of the vault decision (mirror of VaultDispatcher, simplified: per-token limits are not modelled).
 * The authoritative answer is `checkVaultOutcome` (an eth_call of the real dispatcher).
 */
export function explainVault(state: Pick<AccountState, 'address' | 'policy' | 'whitelist' | 'spentToday'>, calls: Call[]): VaultOutcome {
  const p = state.policy
  if (!p.enabled) return { kind: 'instant', reason: 'Guardian Vault is off: everything executes instantly' }
  let native = 0n
  const reasons: string[] = []
  let queue = false
  for (const c of calls) {
    const r = classifyOne(state.address, p, state.whitelist, c)
    if (r.reject) return { kind: 'rejected', reason: r.reject }
    native += r.native
    if (r.queue) {
      queue = true
      reasons.push(r.why)
    }
  }
  if (!queue && native > 0n) {
    if (native > p.nativeThreshold) {
      queue = true
      reasons.push('amount is above your instant threshold')
    } else if (state.spentToday + native > p.dailyLimit) {
      queue = true
      reasons.push("today's instant limit would be exceeded")
    }
  }
  if (queue) return { kind: 'queued', reason: reasons.join('; '), delay: p.delay }
  return { kind: 'instant', reason: native > 0n ? 'within your instant threshold and daily limit' : 'nothing the vault delays' }
}

/**
 * Exact answer for a deployed account: runs the account's own dispatcher in an eth_call FROM the EntryPoint (owner
 * authority), which returns the queued action id (0 = instant) or reverts (e.g. UnlimitedApprovalBlocked).
 */
export async function checkVaultOutcome(client: PublicClient, state: AccountState, calls: Call[]): Promise<VaultOutcome> {
  const local = explainVault(state, calls)
  if (!state.deployed) return local
  try {
    const res = await client.call({ account: ENTRY_POINT, to: state.address, data: callsToCallData(calls) })
    const id = res.data && res.data !== '0x' ? BigInt(res.data) : 0n
    if (id > 0n) {
      return { kind: 'queued', delay: state.policy.delay, reason: local.kind === 'queued' ? local.reason : 'the vault policy delays this call' }
    }
    return { kind: 'instant', reason: local.kind === 'instant' ? local.reason : 'allowed instantly by your policy' }
  } catch (e) {
    const data = revertData(e)
    if (data) {
      try {
        const d = decodeErrorResult({ abi: accountAbi, data })
        if (d.errorName === 'UnlimitedApprovalBlocked') {
          return { kind: 'rejected', reason: `unlimited approval to ${String((d.args as readonly unknown[])[0])} is blocked by your allowance policy` }
        }
        if (d.errorName === 'AccountIsFrozen') return { kind: 'rejected', reason: 'the account is frozen by its guardians' }
        return { kind: 'rejected', reason: `the call would revert: ${d.errorName}` }
      } catch {
        return { kind: 'rejected', reason: `the call would revert (${data.slice(0, 10)})` }
      }
    }
    return { kind: 'unknown', reason: `${local.reason} (simulation unavailable: ${(e as Error).message.split('\n')[0]})` }
  }
}

export function revertData(e: unknown): Hex | undefined {
  let x: unknown = e
  for (let i = 0; i < 8 && x && typeof x === 'object'; i++) {
    const o = x as Record<string, unknown>
    if (typeof o.data === 'string' && o.data.startsWith('0x') && o.data.length >= 10) return o.data as Hex
    if (o.data && typeof o.data === 'object' && typeof (o.data as Record<string, unknown>).data === 'string') {
      return (o.data as Record<string, string>).data as Hex
    }
    x = o.cause
  }
  return undefined
}

// ------------------------------------------------------------------------------------------- queue
export type QueuedAction = {
  id: bigint
  executeAfter: bigint
  status: 'queued' | 'ready' | 'cancelled' | 'executed' | 'expired' | 'invalidated'
  calls: Call[]
  /** Guardian cancel votes so far / threshold. */
  cancelVotes: number
  totalValue: bigint
}

const STATUS = ['none', 'queued', 'cancelled', 'executed'] as const

/** Reads the last `limit` vault actions from the account (`getAction`), newest first. */
export async function listVaultQueue(client: PublicClient, account: Address, now: bigint, limit = 30): Promise<QueuedAction[]> {
  const last = await client.readContract({ address: account, abi: accountAbi, functionName: 'lastActionId' })
  const out: QueuedAction[] = []
  for (let id = last; id >= 1n && out.length < limit; id--) {
    const [executeAfter, status, valid, calls] = await client.readContract({ address: account, abi: accountAbi, functionName: 'getAction', args: [id] })
    let s: QueuedAction['status'] = STATUS[status] === 'none' ? 'cancelled' : (STATUS[status] as QueuedAction['status'])
    if (s === 'queued') {
      if (!valid) s = 'invalidated'
      else if (now > executeAfter + VAULT_EXECUTION_WINDOW) s = 'expired'
      else if (now >= executeAfter) s = 'ready'
    }
    let votes = 0
    if (s === 'queued' || s === 'ready') {
      const [, v] = await client.readContract({ address: account, abi: accountAbi, functionName: 'guardianVotes', args: [2, id] })
      votes = Number(v)
    }
    const cs = calls.map((c) => ({ target: c.target, value: c.value, data: c.data }))
    out.push({ id, executeAfter, status: s, calls: cs, cancelVotes: votes, totalValue: cs.reduce((t, c) => t + c.value, 0n) })
  }
  return out
}

/** Explorer API vault queue (docs/specs/explorer-api.md), if VITE_API_URL is configured. */
export async function listVaultQueueFromApi(apiUrl: string, account: Address): Promise<QueuedAction[]> {
  const res = await fetch(`${apiUrl.replace(/\/$/, '')}/api/v1/addresses/${account}/vault-queue?status=all&limit=50`)
  if (!res.ok) throw new Error(`explorer API: HTTP ${res.status}`)
  const j = (await res.json()) as { items: { id: Hex; target: Address; value: Hex; data: Hex; executeAfter: number; status: QueuedAction['status'] }[] }
  return j.items.map((i) => ({
    id: BigInt(i.id),
    executeAfter: BigInt(i.executeAfter),
    status: i.status,
    calls: [{ target: i.target, value: BigInt(i.value), data: i.data }],
    cancelVotes: 0,
    totalValue: BigInt(i.value),
  }))
}

export function formatCountdown(seconds: bigint): string {
  if (seconds <= 0n) return 'now'
  const s = Number(seconds)
  const d = Math.floor(s / 86_400)
  const h = Math.floor((s % 86_400) / 3_600)
  const m = Math.floor((s % 3_600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  return `${m}m ${s % 60}s`
}

/** Callable on the account: owner (or the guardian threshold) cancels a queued action. */
export function cancelCallData(id: bigint): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'cancel', args: [id] })
}

export function executeQueuedCallData(id: bigint): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'execute', args: [id] })
}

export function setVaultPolicyCallData(p: VaultPolicy): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'setVaultPolicy', args: [p] })
}

export function setGuardiansCallData(guardians: Address[], threshold: number): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'setGuardians', args: [guardians, threshold] })
}

export function setWhitelistedCallData(who: Address, trusted: boolean): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'setWhitelisted', args: [who, trusted] })
}

// JSON-RPC errors (ERC-7769 codes) and revert-data decoding.
import { type Hex, decodeErrorResult, parseAbi } from 'viem'
import { entryPointAbi, gasMeterAbi } from './abi.ts'

export const RpcCodes = {
  parseError: -32700,
  invalidRequest: -32600,
  methodNotFound: -32601,
  invalidParams: -32602,
  internal: -32603,
  /** Rejected by the EntryPoint / account during validation (AA1x, AA2x). */
  rejectedByEntryPointOrAccount: -32500,
  /** Rejected by the paymaster (AA3x). */
  rejectedByPaymaster: -32501,
  /** Outside its validity time range (AA22, AA32). */
  outOfTimeRange: -32503,
  /** Signature check failed (AA24, AA34). */
  invalidSignature: -32507,
  /** Execution (callData) reverted during estimation. */
  executionReverted: -32521,
} as const

export class RpcError extends Error {
  readonly code: number
  readonly data?: unknown
  constructor(code: number, message: string, data?: unknown) {
    super(message)
    this.code = code
    this.data = data
  }
}

/** Errors of the xDEVS account / paymaster and common Solidity errors, for human-readable revert reasons. */
const knownErrorsAbi = parseAbi([
  'error Error(string)',
  'error Panic(uint256)',
  'error Unauthorized()',
  'error InvalidParameter()',
  'error AlreadyInitialized()',
  'error InvalidSigner()',
  'error ActionNotQueued(uint256 id)',
  'error ActionNotReady(uint256 id, uint64 executeAfter)',
  'error ActionExpired(uint256 id)',
  'error ActionInvalidated(uint256 id)',
  'error UnlimitedApprovalBlocked(address spender)',
  'error SessionActionNeedsDelay()',
  'error NotGuardian()',
  'error AlreadyApproved()',
  'error AccountIsFrozen()',
  'error NotFrozen()',
  'error FreezeCooldown(uint64 until)',
  'error NotDelegatedEoa()',
  'error VaultDisabled()',
  'error EoaKeyInactive()',
  'error SponsorRejected(uint8 reason)',
  'error InsufficientBalance()',
  'error InvalidPaymasterData()',
  'error NotFromEntryPoint()',
  'error ERC20InsufficientBalance(address sender, uint256 balance, uint256 needed)',
  'error ERC20InsufficientAllowance(address spender, uint256 allowance, uint256 needed)',
])

const allAbis = [...entryPointAbi, ...gasMeterAbi, ...knownErrorsAbi]

const SPONSOR_REASONS: Record<number, string> = {
  1: 'sponsor inactive',
  2: 'maxCost above the sponsor maxCostPerOp',
  3: 'sponsor balance too low',
  4: 'per-account cap reached',
  5: 'account not allowlisted by the sponsor',
}

/** Best-effort human-readable rendering of revert data. */
export function describeRevert(data: Hex | undefined | null, depth = 0): string {
  if (!data || data === '0x') return 'reverted without data'
  try {
    const d = decodeErrorResult({ abi: allAbis, data })
    const args = (d.args ?? []) as readonly unknown[]
    if (d.errorName === 'Error') return String(args[0])
    if (d.errorName === 'SponsorRejected') {
      return `SponsorRejected(${String(args[0])}: ${SPONSOR_REASONS[Number(args[0])] ?? 'unknown'})`
    }
    if ((d.errorName === 'FailedOpWithRevert' || d.errorName === 'StepFailed') && depth < 3) {
      const inner = args[d.errorName === 'StepFailed' ? 1 : 2] as Hex
      const prefix = d.errorName === 'StepFailed' ? `step ${String(args[0])}` : String(args[1])
      return `${prefix}: ${describeRevert(inner, depth + 1)}`
    }
    if (d.errorName === 'FailedOp') return String(args[1])
    if (d.errorName === 'DelegateAndRevert' && depth < 3) return describeRevert(args[1] as Hex, depth + 1)
    return `${d.errorName}(${args.map((a) => String(a)).join(', ')})`
  } catch {
    return `revert ${data.slice(0, 10)}${data.length > 10 ? '…' : ''}`
  }
}

/** Maps an EntryPoint `AAxx` reason to the ERC-7769 error code. */
export function codeForReason(reason: string): number {
  const m = /^AA(\d)(\d)/.exec(reason)
  if (!m) return RpcCodes.rejectedByEntryPointOrAccount
  if (reason.startsWith('AA24') || reason.startsWith('AA34')) return RpcCodes.invalidSignature
  if (reason.startsWith('AA22') || reason.startsWith('AA32')) return RpcCodes.outOfTimeRange
  if (m[1] === '3') return RpcCodes.rejectedByPaymaster
  return RpcCodes.rejectedByEntryPointOrAccount
}

/** Extracts revert data from a JSON-RPC error object (anvil / geth: `data` hex; some nodes: `data.data`). */
export function revertDataOf(error: unknown): Hex | undefined {
  let e: unknown = error
  for (let i = 0; i < 6 && e && typeof e === 'object'; i++) {
    const o = e as Record<string, unknown>
    if (typeof o.data === 'string' && o.data.startsWith('0x')) return o.data as Hex
    if (o.data && typeof o.data === 'object') {
      const inner = (o.data as Record<string, unknown>).data
      if (typeof inner === 'string' && inner.startsWith('0x')) return inner as Hex
    }
    e = o.cause ?? o.error
  }
  return undefined
}

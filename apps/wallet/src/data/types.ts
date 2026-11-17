// The wallet's data layer. The UI only talks to a `Backend`: `live.ts` (the chain, the bundler and the explorer API)
// or `demo.ts` (fixtures, for reviewing every screen without a chain or a real passkey).
import type { Address, Hex } from 'viem'
import type { AccountState, Call, InitParams } from '../lib/account.ts'
import type { ApprovalItem } from '../lib/approvals.ts'
import type { Preview } from '../lib/preview.ts'
import type { RecipientCheckResult, ResolvedRecipient } from '../lib/recipient.ts'
import type { QueuedAction, VaultOutcome } from '../lib/vault.ts'
import type { AccountRecord } from '../store.ts'

export type ChainStatus =
  | { kind: 'checking' }
  | { kind: 'ok'; chainId: number; block: bigint }
  | { kind: 'wrong-chain'; chainId: number }
  | { kind: 'unreachable'; reason: string }

export type Asset = {
  /** null = native tXDS */
  token: Address | null
  symbol: string
  name: string
  decimals: number
  balance: bigint
}

export type ActivityItem = {
  id: string
  kind: 'sent' | 'received' | 'queued' | 'cancelled' | 'approval' | 'revoke' | 'account' | 'name' | 'settings'
  title: string
  counterparty?: Address | null
  counterpartyName?: string | null
  amount?: bigint
  symbol?: string
  decimals?: number
  timestamp: number
  status: 'confirmed' | 'queued' | 'failed' | 'cancelled'
  hash?: Hex | null
  /** Explorer spam / poisoning flags (look-alike of a past counterparty). */
  spam?: boolean
  lookAlikeOf?: Address | null
}

export type Review = {
  call: Call
  recipient: ResolvedRecipient | null
  check: RecipientCheckResult | null
  preview: Preview
  vault: VaultOutcome
}

export type OpInput = { calls?: Call[]; callData?: Hex; sponsored?: boolean; describe: Omit<ActivityItem, 'id' | 'timestamp' | 'status'> }

export type OpResult = { userOpHash: Hex; txHash: Hex | null; queuedIds: bigint[] }

export type XnsQuote = { name: string; available: boolean; price: bigint; reason?: string }

export type RecoveryStatus = { kind: 'none' | 'proposed' | 'ready' | 'executed' | 'cancelled' | 'unknown'; text: string }

export interface Backend {
  readonly demo: boolean
  chainStatus(): Promise<ChainStatus>
  readAccount(address: Address): Promise<AccountState>
  assets(address: Address, nativeBalance: bigint): Promise<Asset[]>
  activity(address: Address): Promise<ActivityItem[]>
  vaultQueue(address: Address): Promise<QueuedAction[]>
  approvals(address: Address): Promise<ApprovalItem[]>
  recoveryStatus(address: Address): Promise<RecoveryStatus>
  resolveRecipient(input: string): Promise<ResolvedRecipient>
  checkRecipient(from: Address, to: Address, history: readonly Address[]): Promise<RecipientCheckResult>
  review(state: AccountState, call: Call, recipient: ResolvedRecipient | null, check: RecipientCheckResult | null): Promise<Review>
  tokenDecimals(token: Address): Promise<number>
  counterfactualAddress(params: InitParams, salt: Hex): Promise<Address>
  primaryName(address: Address): Promise<string | null>
  xnsQuote(label: string): Promise<XnsQuote>
  /** Signs (one passkey prompt) and submits a UserOperation for `rec`, then waits for inclusion. */
  sendOp(rec: AccountRecord, input: OpInput, onStatus: (s: string) => void): Promise<OpResult>
}

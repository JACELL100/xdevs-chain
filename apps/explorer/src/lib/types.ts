// Response shapes of the explorer API (docs/specs/explorer-api.md). Quantities are 0x hex strings.

export type Hex = `0x${string}`

export interface Page<T> {
  items: T[]
  nextCursor: string | null
  hiddenSpam?: number | null
}

export interface TokenRef {
  address: string | null
  name: string | null
  symbol: string | null
  decimals: number | null
  known: boolean
}

export interface BlockSummary {
  number: number
  hash: string
  parentHash: string
  timestamp: number
  miner: string
  txCount: number
  gasUsed: string
  gasLimit: string
  baseFeePerGas: string | null
  size: number | null
  fees: { burned: string; treasury: string; priority: string }
}

export interface Commit {
  round: number
  signatures: number
  validators: string[]
}

export interface BlockDetail extends BlockSummary {
  transactions: TxSummary[]
  commit: Commit | null
}

export interface TxSummary {
  hash: string
  blockNumber: number
  timestamp: number
  txIndex: number
  type: number
  from: string
  to: string | null
  contractCreated: string | null
  value: string
  status: 'success' | 'reverted'
  methodId: string | null
  fee: string
  spam: boolean
  spamReasons: string[]
}

export interface TokenTransfer {
  blockNumber: number
  timestamp: number
  txHash: string
  logIndex: number | null
  kind: 'native' | 'erc20'
  token: TokenRef
  from: string
  to: string
  value: string
  spam: boolean
  spamReasons: string[]
  lookAlikeOf: string | null
}

export interface DecodedLog {
  logIndex: number
  address: string
  topics: string[]
  data: string
  event: string | null
  signature: string | null
  standard: string | null
  args: Record<string, unknown> | null
}

export interface ApprovalChange {
  kind: 'erc20' | 'xdrc20' | 'permit2'
  event: string
  token: TokenRef | null
  owner: string | null
  spender: string | null
  amount: string | null
  unlimited: boolean
  expiry: number | null
  revokesAll: boolean
}

export interface FeeBreakdown {
  total: string
  baseFee: string
  burned: string
  treasury: string
  priorityFee: string
  burnBps: number
}

export interface TxDetail extends TxSummary {
  blockHash: string
  nonce: number
  gas: string
  gasUsed: string
  gasPrice: string | null
  maxFeePerGas: string | null
  maxPriorityFeePerGas: string | null
  effectiveGasPrice: string
  input: string
  authorizationList: { chainId: string; address: string; nonce: string; authority: string | null }[] | null
  feeBreakdown: FeeBreakdown
  logs: DecodedLog[]
  tokenTransfers: TokenTransfer[]
  approvals: ApprovalChange[]
}

export interface Stats {
  chainId: number | null
  latestBlock: number | null
  indexedBlock: number | null
  lagBlocks: number | null
  avgBlockTimeMs: number | null
  totalTransactions: number
  totalAddresses: number
  totalTokenTransfers: number
  burnedFees: string
  treasuryFees: string
  latestBaseFee: string | null
  nativeCurrency: { name: string; symbol: string; decimals: number }
}

export interface AddressSummary {
  address: string
  live: boolean
  balance: string | null
  nonce: number | null
  kind: 'eoa' | 'contract' | 'delegated' | null
  delegatedTo: string | null
  isContract: boolean
  firstSeenBlock: number | null
  lastSeenBlock: number | null
  txCount: number
  xns: { primaryName: string | null; names: { name: string; expires: number; status: NameStatus }[] }
  labels: string[]
  token: TokenRef | null
  security: {
    accountKind: 'xdevs-7702' | 'xdevs-proxy' | 'unknown' | null
    vaultEnabled: boolean | null
    frozen: boolean
    frozenUntil: number | null
    eoaLock: { status: 'unlocked' | 'locked' | 'sealed' | 'unknown'; active: boolean | null; source: 'rpc' | 'indexer' }
    eoaKeyRevoked: boolean
    recovery: { id: string; approvals: number | null; readyAt: number | null } | null
    guardians: { count: number; threshold: number | null } | null
    openVaultActions: number
  }
}

export interface Approval {
  kind: 'erc20' | 'xdrc20' | 'permit2'
  token: TokenRef
  spender: string
  amount: string
  unlimited: boolean
  expiry: number | null
  expired: boolean
  remaining: string | null
  blockNumber: number
  txHash: string
  revoke: { to: string; data: string; description: string }
}

export type VaultStatus = 'queued' | 'ready' | 'cancelled' | 'executed' | 'expired' | 'invalidated'

export interface VaultAction {
  account: string
  id: string
  target: string
  value: string
  data: string
  executeAfter: number
  status: VaultStatus
  queuedBlock: number
  queuedTx: string
  cancelledBy: string | null
  finalBlock: number | null
  finalTx: string | null
}

export type NameStatus = 'active' | 'grace' | 'released'

export interface NameInfo {
  name: string
  label: string
  node: string
  labelhash: string
  owner: string
  address: string | null
  expires: number
  graceEndsAt: number
  status: NameStatus
  registeredBlock: number
  registeredTx: string
  isPrimaryFor: string | null
}

export interface SearchResult {
  type: 'block' | 'tx' | 'address' | 'name'
  id: string
  label: string | null
}

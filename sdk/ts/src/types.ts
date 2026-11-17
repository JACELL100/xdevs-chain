import type { Address, BlockTag, Hash, Hex, RpcTransactionRequest } from 'viem'

// =====================================================================================================
// Shared
// =====================================================================================================

/** Hex-encoded quantity as sent on the wire (`"0x5208"`). */
export type Quantity = Hex

/** `blockNumber` or `blockTag` (default `"latest"`), like viem's read actions. */
export type BlockParameter =
  | { blockNumber?: bigint | undefined; blockTag?: undefined }
  | { blockNumber?: undefined; blockTag?: BlockTag | undefined }

/** Wire form of a block parameter: a hex block number or a tag. */
export type RpcBlockParameter = Quantity | BlockTag

/** Statuses of the Staking predeploy (`IStaking.ValidatorStatus`, plan §11.4), lowercased. */
export type ValidatorStatus = 'candidate' | 'active' | 'jailed' | 'tombstoned' | 'exited'

// =====================================================================================================
// Safety warnings (plan §16.4 catalogue v1)
// =====================================================================================================

/**
 * Warning codes of the §16.4 v1 catalogue:
 * - `UNLIMITED_APPROVAL` — unlimited ERC-20 approval
 * - `SET_APPROVAL_FOR_ALL` — `setApprovalForAll(operator, true)` on an NFT collection
 * - `OWNERSHIP_TRANSFER` — transfers ownership/admin of a contract or account
 * - `NEW_CONTRACT` — target contract was deployed recently
 * - `UNVERIFIED_CONTRACT` — target contract source is not verified
 * - `FIRST_TIME_RECIPIENT` — sender never sent to this address before
 * - `LOOKALIKE_RECIPIENT` — recipient looks like a past recipient (address poisoning, §16.3)
 * - `SELF_DESTRUCT_PROXY` — call to a proxy whose implementation can self-destruct
 * - `HIGH_VALUE_TRANSFER` — value exceeds X% of the sender's balance
 * - `PERMIT_SIGNATURE` — EIP-2612 Permit typed-data signature request
 * - `PERMIT2_SIGNATURE` — Permit2 typed-data signature request
 *
 * The node (Phase 4) also emits: `REVERT`, `NEVER_EXPIRING_APPROVAL`, `CONTRACT_CREATION`, `SELF_DESTRUCT`,
 * `VAULT_QUEUED`, `BURN`, `INSUFFICIENT_FUNDS`, `TRACE_TRUNCATED` (simulation) and `POISONING_PATTERN`,
 * `ZERO_ADDRESS`, `PRECOMPILE`, `SYSTEM_CONTRACT`, `CONTRACT_RECIPIENT`, `NEW_ADDRESS`, `NAME_MISMATCH`
 * (recipient flags). Codes are open-ended: treat unknown ones by severity.
 */
export type XdevsWarningCode =
  | 'UNLIMITED_APPROVAL'
  | 'SET_APPROVAL_FOR_ALL'
  | 'OWNERSHIP_TRANSFER'
  | 'NEW_CONTRACT'
  | 'UNVERIFIED_CONTRACT'
  | 'FIRST_TIME_RECIPIENT'
  | 'LOOKALIKE_RECIPIENT'
  | 'SELF_DESTRUCT_PROXY'
  | 'HIGH_VALUE_TRANSFER'
  | 'PERMIT_SIGNATURE'
  | 'PERMIT2_SIGNATURE'

/** Every v1 warning code, for exhaustive UI mapping. */
export const XDEVS_WARNING_CODES = [
  'UNLIMITED_APPROVAL',
  'SET_APPROVAL_FOR_ALL',
  'OWNERSHIP_TRANSFER',
  'NEW_CONTRACT',
  'UNVERIFIED_CONTRACT',
  'FIRST_TIME_RECIPIENT',
  'LOOKALIKE_RECIPIENT',
  'SELF_DESTRUCT_PROXY',
  'HIGH_VALUE_TRANSFER',
  'PERMIT_SIGNATURE',
  'PERMIT2_SIGNATURE',
] as const satisfies readonly XdevsWarningCode[]

export type XdevsWarningSeverity = 'info' | 'low' | 'medium' | 'high'

/**
 * A risk warning. **Warnings are advice, not facts** (plan §16.4) — always present them as such.
 * `code` is open-ended (`string & {}`) so a newer node can add codes without breaking older SDKs.
 */
export type XdevsWarning = {
  code: XdevsWarningCode | (string & {})
  severity: XdevsWarningSeverity
  message: string
}

// =====================================================================================================
// xdevs_simulateTransaction (§16.4)
// =====================================================================================================

export type XdevsAssetStandard = 'native' | 'erc20' | 'erc721' | 'erc1155'

export type XdevsAssetChange = {
  /** `"native"` for tXDS/XDS, otherwise the token contract. */
  token: 'native' | Address
  /** Self-reported by the token contract (`null` if it has no `symbol()`). */
  symbol: string | null
  from: Address
  to: Address
  /** Human-readable decimal amount, already scaled by token decimals (e.g. `"12.5"`); raw if decimals are unknown. */
  amount: string
  /** Token standard (node simulation). */
  standard?: XdevsAssetStandard
  /** Self-reported decimals, `null` if unknown (node simulation). */
  decimals?: number | null
  /** Raw amount in base units (node simulation). */
  rawAmount?: Quantity
  /** NFT id (ERC-721 / ERC-1155). */
  tokenId?: Quantity
}

/** Net change of one address in one asset over the whole simulated transaction. */
export type XdevsBalanceChange = {
  address: Address
  token: 'native' | Address
  symbol: string | null
  /** Signed, scaled amount (`"-12.5"`). */
  delta: string
  /** Signed raw amount in base units (`"-12500000"`). */
  rawDelta: string
  tokenId?: Quantity
}

export type XdevsApprovalChange = {
  token: Address
  spender: Address
  /** Scaled amount, `"unlimited"`, `"all"` (operator approval) or `"0"` (revocation). */
  amount: string
  /** Unix seconds when the approval expires, `null` = never (plan §16.5). */
  expires: number | null
  /** Approval kind (node simulation). */
  kind?: ApprovalKind | 'erc721'
  owner?: Address
  symbol?: string | null
  rawAmount?: Quantity
  unlimited?: boolean
  tokenId?: Quantity
}

/** Guardian Vault outcome of a simulated transaction to an xDEVS Smart Account. */
export type XdevsVaultOutcome = {
  account: Address
  /** `true`: queued with a cancel window; `false`: executed instantly. */
  queued: boolean
  actionId?: Quantity
  /** Unix seconds from which the queued action can be executed. */
  releaseAt?: number
  executed?: Quantity[]
  cancelled?: Quantity[]
}

/** Wire response of `xdevs_simulateTransaction` (plan §16.4). */
export type RpcSimulationResult = {
  success: boolean
  gasUsed: Quantity
  assetChanges: XdevsAssetChange[]
  approvals: XdevsApprovalChange[]
  warnings: XdevsWarning[]
  /** Always present: "advice, not fact: …". */
  disclaimer: string
  /** Decoded revert reason when `success` is false. */
  revertReason?: string | null
  /** Raw revert data. */
  revertData?: Hex
  /** Net change per address and asset. */
  balanceChanges?: XdevsBalanceChange[]
  contractsCreated?: { address: Address; creator: Address }[]
  vault?: XdevsVaultOutcome
  /** Block whose state the simulation ran on. */
  blockNumber?: Quantity
}

export type SimulationResult = Omit<RpcSimulationResult, 'gasUsed'> & { gasUsed: bigint }

// =====================================================================================================
// xdevs_checkRecipient (§16.3)
// =====================================================================================================

/** On-chain facts about a recipient (`xdevs_checkRecipient` → `facts`). */
export type RecipientFacts = {
  address: Address
  /** The XNS name that was entered (normalized), if any. */
  resolvedFrom: string | null
  isZeroAddress: boolean
  isPrecompile: boolean
  isSystemContract: boolean
  isContract: boolean
  codeSize: number
  /** xDEVS Smart Account (EIP-7702 delegation to, or ERC-1967 proxy of, `0x…1005`). */
  isSmartAccount: boolean
  delegatedTo: Address | null
  /** XIP-0001 lock status, `null` when the node does not serve it. */
  eoaLock: 'unlocked' | 'locked' | 'sealed' | null
  nonce: number
  balance: Quantity
  primaryName: string | null
}

/** Advisory part of `xdevs_checkRecipient` (heuristics — advice, not fact). */
export type RecipientAdvice = {
  /** `null` when no `from` was given. */
  firstTime: boolean | null
  lookAlikeOf: Address[]
  flags: XdevsWarning[]
  /** Block range of the bounded `Transfer` log scan behind `firstTime` / `lookAlikeOf`. */
  history: { fromBlock: number; toBlock: number; complete: boolean } | null
  disclaimer: string
}

/** Wire response of `xdevs_checkRecipient(from, to)`. */
export type RecipientCheck = {
  /** `from` has never sent to `to` before. */
  firstTime: boolean
  /** Past recipients of `from` that share the first/last 4–6 hex chars with `to` (address poisoning). */
  lookAlikeOf: Address[]
  /** Primary XNS name of `to` (forward-verified by the node), or `null`. */
  name: string | null
  /** `to` has code (contract or EIP-7702 delegated account). */
  isContract: boolean
  /** Optional advisory warnings (same catalogue as simulation). */
  warnings?: XdevsWarning[]
  /** Resolved recipient address (node ≥ Phase 4). */
  address?: Address
  /** On-chain facts (node ≥ Phase 4). */
  facts?: RecipientFacts
  /** Advice, kept apart from the facts (node ≥ Phase 4). */
  advice?: RecipientAdvice
  blockNumber?: Quantity
}

// =====================================================================================================
// xdevs_getApprovals (§16.5)
// =====================================================================================================

export type ApprovalKind = 'erc20' | 'xdrc20' | 'permit2' | 'erc721-all' | 'erc1155-all'

/** One live approval of `owner` (wire and parsed shape are identical). */
export type XdevsApproval = {
  kind: ApprovalKind
  token: Address
  symbol?: string | null
  spender: Address
  /** Remaining raw amount in base units (decimal integer string), `"unlimited"`, or `"all"` for operator approvals. */
  amount: string
  /** Unix seconds, `null` = never expires. */
  expires: number | null
  owner?: Address
  rawAmount?: Quantity
  unlimited?: boolean
  /** Block of the last approval event seen for this pair. */
  lastUpdatedBlock?: Quantity
  transactionHash?: Hash | null
}

// =====================================================================================================
// xdevs_getVaultQueue (§16.2)
// =====================================================================================================

/** One call of a queued Guardian Vault action. */
export type RpcVaultCall = { target: Address; value: Quantity; data: Hex }

/** Wire form of a queued Guardian Vault action (mirrors the `VaultActionQueued` event). */
export type RpcVaultAction = {
  id: Quantity
  target: Address
  value: Quantity
  data: Hex
  /** Unix seconds after which anyone may `execute(id)`. */
  executeAfter: Quantity
  /** Smart account (node ≥ Phase 4). */
  account?: Address
  /** Unix seconds after which the action can no longer be executed. */
  expiresAt?: Quantity
  /** `queued` (cancel window) or `ready` (executable). */
  status?: 'queued' | 'ready'
  calls?: RpcVaultCall[]
  cancelVotes?: number
}

export type VaultAction = {
  id: bigint
  target: Address
  value: bigint
  data: Hex
  executeAfter: bigint
  account?: Address
  expiresAt?: bigint
  status?: 'queued' | 'ready'
  calls?: { target: Address; value: bigint; data: Hex }[]
  cancelVotes?: number
}

// =====================================================================================================
// eth_subscribe("vaultEvents") (§16.2)
// =====================================================================================================

export type VaultEventType = 'queued' | 'cancelled' | 'executed' | 'invalidated' | 'frozen' | 'unfrozen'

/** Wire form of a `vaultEvents` notification. */
export type RpcVaultEvent = {
  type: VaultEventType
  account: Address
  id?: Quantity
  target?: Address
  value?: Quantity
  data?: Hex
  executeAfter?: Quantity
  by?: Address
  frozenUntil?: Quantity
  epoch?: number
  blockNumber?: Quantity
  blockHash?: Hash
  transactionHash?: Hash
  logIndex?: Quantity
}

export type VaultEvent = {
  type: VaultEventType
  account: Address
  id?: bigint
  target?: Address
  value?: bigint
  data?: Hex
  executeAfter?: bigint
  by?: Address
  frozenUntil?: bigint
  epoch?: number
  blockNumber?: bigint
  blockHash?: Hash
  transactionHash?: Hash
  logIndex?: number
}

// =====================================================================================================
// xdevs_localFee (§15.2)
// =====================================================================================================

export type RpcLocalFee = {
  contract: Address
  globalBaseFee: Quantity
  localBaseFee: Quantity
}

export type LocalFee = {
  contract: Address
  globalBaseFee: bigint
  localBaseFee: bigint
  /** `globalBaseFee + localBaseFee` — the base fee a tx to `contract` must pay. */
  totalBaseFee: bigint
}

// =====================================================================================================
// xdevs_validators (§11)
// =====================================================================================================

export type RpcValidator = {
  operator: Address
  /** Ed25519 consensus public key (32 bytes). */
  consensusKey: Hex
  status: ValidatorStatus
  votingPower: Quantity
  totalStake: Quantity
  commissionBps: number
  /** Signed-block ratio over the downtime window, basis points (10000 = 100%). */
  uptimeBps: number
}

export type Validator = Omit<RpcValidator, 'votingPower' | 'totalStake'> & {
  votingPower: bigint
  totalStake: bigint
}

// =====================================================================================================
// xdevs_getCommitCertificate (§8.1)
// =====================================================================================================

export type CommitSignature = {
  /** Validator address (20 bytes, `sha256(ed25519 pubkey)[..20]`). */
  validator: Hex
  /** Ed25519 signature over the precommit (64 bytes). BLS aggregate later. */
  signature: Hex
}

/** Shape served by the node (research doc §6.5): signatures sorted by validator address. */
export type RpcCommitCertificate = {
  height: Quantity
  round: Quantity
  blockHash: Hash
  signatures: CommitSignature[]
  /** Canonical RLP of the certificate, verifiable against the validator set. */
  raw: Hex
}

export type CommitCertificate = Omit<RpcCommitCertificate, 'height' | 'round'> & {
  height: bigint
  round: number
}

// =====================================================================================================
// xdevs_epochInfo (§11.5)
// =====================================================================================================

export type RpcEpochInfo = {
  epoch: Quantity
  epochLength: Quantity
  startBlock: Quantity
  /** First block of the next epoch (the `onEpoch` transition happens there). */
  nextTransitionBlock: Quantity
  validatorSetHash: Hash
}

export type EpochInfo = {
  epoch: bigint
  epochLength: bigint
  startBlock: bigint
  nextTransitionBlock: bigint
  validatorSetHash: Hash
}

// =====================================================================================================
// RPC schema (usable with viem's `rpcSchema<XdevsRpcSchema>()`)
// =====================================================================================================

export type XdevsRpcSchema = [
  {
    Method: 'xdevs_simulateTransaction'
    Parameters: [transaction: RpcTransactionRequest, block: RpcBlockParameter]
    ReturnType: RpcSimulationResult
  },
  {
    Method: 'xdevs_checkRecipient'
    Parameters: [from: Address, to: Address] | [to: Address | string] | [from: Address | null, to: Address | string, block: RpcBlockParameter]
    ReturnType: RecipientCheck
  },
  {
    Method: 'xdevs_getApprovals'
    Parameters:
      | [owner: Address, block: RpcBlockParameter]
      | [owner: Address, options: { tokens?: Address[]; fromBlock?: Quantity }, block: RpcBlockParameter]
    ReturnType: XdevsApproval[]
  },
  {
    Method: 'xdevs_getVaultQueue'
    Parameters: [vault: Address, block: RpcBlockParameter]
    ReturnType: RpcVaultAction[]
  },
  {
    Method: 'xdevs_resolveName'
    Parameters: [name: string]
    ReturnType: Address | null
  },
  {
    Method: 'xdevs_reverseName'
    Parameters: [address: Address]
    ReturnType: string | null
  },
  {
    Method: 'xdevs_localFee'
    Parameters: [contract: Address, block: RpcBlockParameter]
    ReturnType: RpcLocalFee
  },
  {
    Method: 'xdevs_validators'
    Parameters: [block: RpcBlockParameter]
    ReturnType: RpcValidator[]
  },
  {
    Method: 'xdevs_getCommitCertificate'
    Parameters: [height: Quantity]
    ReturnType: RpcCommitCertificate | null
  },
  {
    Method: 'xdevs_epochInfo'
    Parameters: [block: RpcBlockParameter]
    ReturnType: RpcEpochInfo
  },
]

export type XdevsRpcMethod = XdevsRpcSchema[number]['Method']

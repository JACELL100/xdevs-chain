import {
  type AccessList,
  type Account,
  type Address,
  type Chain,
  type Client,
  type Hex,
  type SignedAuthorizationList,
  type Transport,
  formatTransactionRequest,
  getAddress,
  isAddress,
  numberToHex,
} from 'viem'
import { XdevsMethodNotAvailableError, isMethodNotFoundError } from './errors.ts'
import type {
  BlockParameter,
  CommitCertificate,
  EpochInfo,
  LocalFee,
  RecipientCheck,
  RpcBlockParameter,
  SimulationResult,
  Validator,
  VaultAction,
  XdevsApproval,
  XdevsRpcSchema,
} from './types.ts'
import { normalizeXnsName } from './xns.ts'

type AnyClient = Client<Transport, Chain | undefined, Account | undefined>

type Method = XdevsRpcSchema[number]['Method']
type ParamsOf<M extends Method> = Extract<XdevsRpcSchema[number], { Method: M }>['Parameters']
type ReturnOf<M extends Method> = Extract<XdevsRpcSchema[number], { Method: M }>['ReturnType']

/**
 * Sends one `xdevs_` request and maps "method not found" to {@link XdevsMethodNotAvailableError}.
 * Retries are disabled for that error by viem already; other errors propagate unchanged.
 */
export async function xdevsRequest<M extends Method>(
  client: AnyClient,
  method: M,
  params: ParamsOf<M>,
): Promise<ReturnOf<M>> {
  const request = client.request as unknown as (args: { method: string; params: unknown }) => Promise<unknown>
  try {
    return (await request({ method, params })) as ReturnOf<M>
  } catch (error) {
    if (isMethodNotFoundError(error)) throw new XdevsMethodNotAvailableError(method, error as Error)
    throw error
  }
}

// -----------------------------------------------------------------------------------------------------
// Parsing helpers (tolerate hex quantities, decimal strings and JSON numbers)
// -----------------------------------------------------------------------------------------------------

function toBigInt(value: unknown, field: string): bigint {
  if (typeof value === 'bigint') return value
  if (typeof value === 'number' && Number.isSafeInteger(value)) return BigInt(value)
  if (typeof value === 'string' && /^(0x[0-9a-fA-F]+|\d+)$/.test(value)) return BigInt(value)
  throw new TypeError(`xDEVS RPC: field "${field}" is not a quantity: ${JSON.stringify(value)}`)
}

function blockParam(args: BlockParameter | undefined): RpcBlockParameter {
  if (args?.blockNumber !== undefined) return numberToHex(args.blockNumber)
  return args?.blockTag ?? 'latest'
}

function checkedAddress(value: string, field: string): Address {
  if (!isAddress(value, { strict: false })) throw new TypeError(`${field} is not a valid address: ${value}`)
  return value
}

// -----------------------------------------------------------------------------------------------------
// xdevs_simulateTransaction — Phase 4 (§16.4)
// -----------------------------------------------------------------------------------------------------

export type SimulateTransactionParameters = {
  /** Sender. Defaults to the client's account. */
  account?: Address | Account | undefined
  to?: Address | null | undefined
  data?: Hex | undefined
  value?: bigint | undefined
  gas?: bigint | undefined
  gasPrice?: bigint | undefined
  maxFeePerGas?: bigint | undefined
  maxPriorityFeePerGas?: bigint | undefined
  nonce?: number | undefined
  accessList?: AccessList | undefined
  authorizationList?: SignedAuthorizationList | undefined
} & BlockParameter

/**
 * Simulates a transaction on the node (revm + inspector) and returns asset changes, approvals and
 * advisory warnings. Node support: **Phase 4**. Results are advisory (see `disclaimer`).
 */
export async function simulateTransaction(
  client: AnyClient,
  args: SimulateTransactionParameters,
): Promise<SimulationResult> {
  const { account = client.account, blockNumber, blockTag, ...tx } = args
  const from = typeof account === 'string' ? account : account?.address
  const request = formatTransactionRequest({ ...tx, ...(from ? { from } : {}) } as Parameters<
    typeof formatTransactionRequest
  >[0])
  const res = await xdevsRequest(client, 'xdevs_simulateTransaction', [
    request,
    blockParam({ blockNumber, blockTag } as BlockParameter),
  ])
  return { ...res, gasUsed: toBigInt(res.gasUsed, 'gasUsed') }
}

// -----------------------------------------------------------------------------------------------------
// xdevs_checkRecipient — Phase 4 (§16.3)
// -----------------------------------------------------------------------------------------------------

export type CheckRecipientParameters = { from: Address; to: Address }

/** Address-poisoning / first-time-recipient checks for a transfer. Node support: **Phase 4**. */
export async function checkRecipient(client: AnyClient, { from, to }: CheckRecipientParameters): Promise<RecipientCheck> {
  return xdevsRequest(client, 'xdevs_checkRecipient', [checkedAddress(from, 'from'), checkedAddress(to, 'to')])
}

// -----------------------------------------------------------------------------------------------------
// xdevs_getApprovals — Phase 4 (§16.5)
// -----------------------------------------------------------------------------------------------------

export type GetApprovalsParameters = { owner: Address } & BlockParameter

/** All live approvals of `owner` (for a one-click revoke screen). Node support: **Phase 4**. */
export async function getApprovals(client: AnyClient, args: GetApprovalsParameters): Promise<XdevsApproval[]> {
  return xdevsRequest(client, 'xdevs_getApprovals', [checkedAddress(args.owner, 'owner'), blockParam(args)])
}

// -----------------------------------------------------------------------------------------------------
// xdevs_getVaultQueue — Phase 4 (§16.2)
// -----------------------------------------------------------------------------------------------------

export type GetVaultQueueParameters = { vault: Address } & BlockParameter

/** Pending (not executed, not cancelled) Guardian Vault actions. Node support: **Phase 4**. */
export async function getVaultQueue(client: AnyClient, args: GetVaultQueueParameters): Promise<VaultAction[]> {
  const res = await xdevsRequest(client, 'xdevs_getVaultQueue', [checkedAddress(args.vault, 'vault'), blockParam(args)])
  return res.map((a) => ({
    id: toBigInt(a.id, 'id'),
    target: a.target,
    value: toBigInt(a.value, 'value'),
    data: a.data,
    executeAfter: toBigInt(a.executeAfter, 'executeAfter'),
  }))
}

// -----------------------------------------------------------------------------------------------------
// xdevs_resolveName / xdevs_reverseName — Phase 4 (§16.3)
// -----------------------------------------------------------------------------------------------------

export type ResolveXnsNameParameters = { name: string }

/**
 * Resolves an XNS name (`alice` or `alice.xdevs`) to an address, or `null` if unregistered.
 * The name is normalized and validated locally first (throws `InvalidXnsNameError`). Node support: **Phase 4**.
 */
export async function resolveXnsName(client: AnyClient, { name }: ResolveXnsNameParameters): Promise<Address | null> {
  const res = await xdevsRequest(client, 'xdevs_resolveName', [normalizeXnsName(name)])
  return res ? getAddress(res) : null
}

export type ReverseXnsNameParameters = { address: Address }

/** Primary XNS name of `address`, or `null`. Node support: **Phase 4**. */
export async function reverseXnsName(client: AnyClient, { address }: ReverseXnsNameParameters): Promise<string | null> {
  const res = await xdevsRequest(client, 'xdevs_reverseName', [checkedAddress(address, 'address')])
  return res ?? null
}

// -----------------------------------------------------------------------------------------------------
// xdevs_localFee — Phase 5 (§15.2)
// -----------------------------------------------------------------------------------------------------

export type GetLocalFeeParameters = { contract: Address } & BlockParameter

/** Global + local base fee for txs whose top-level `to` is `contract`. Node support: **Phase 5**. */
export async function getLocalFee(client: AnyClient, args: GetLocalFeeParameters): Promise<LocalFee> {
  const res = await xdevsRequest(client, 'xdevs_localFee', [checkedAddress(args.contract, 'contract'), blockParam(args)])
  const globalBaseFee = toBigInt(res.globalBaseFee, 'globalBaseFee')
  const localBaseFee = toBigInt(res.localBaseFee, 'localBaseFee')
  return { contract: res.contract, globalBaseFee, localBaseFee, totalBaseFee: globalBaseFee + localBaseFee }
}

// -----------------------------------------------------------------------------------------------------
// xdevs_validators — Phase 3 (§11)
// -----------------------------------------------------------------------------------------------------

export type GetValidatorsParameters = BlockParameter

/** Active validator set with voting power and uptime. Node support: **Phase 3**. */
export async function getValidators(client: AnyClient, args?: GetValidatorsParameters): Promise<Validator[]> {
  const res = await xdevsRequest(client, 'xdevs_validators', [blockParam(args)])
  return res.map((v) => ({
    ...v,
    votingPower: toBigInt(v.votingPower, 'votingPower'),
    totalStake: toBigInt(v.totalStake, 'totalStake'),
  }))
}

// -----------------------------------------------------------------------------------------------------
// xdevs_getCommitCertificate — Phase 2 (§8.1)
// -----------------------------------------------------------------------------------------------------

export type GetCommitParameters = { height: bigint | number }

/** BFT commit certificate (2/3+ precommits) for `height`, or `null` if unknown. Node support: **Phase 2**. */
export async function getCommit(client: AnyClient, { height }: GetCommitParameters): Promise<CommitCertificate | null> {
  const res = await xdevsRequest(client, 'xdevs_getCommitCertificate', [numberToHex(height)])
  if (!res) return null
  return { ...res, height: toBigInt(res.height, 'height'), round: Number(toBigInt(res.round, 'round')) }
}

// -----------------------------------------------------------------------------------------------------
// xdevs_epochInfo — Phase 3 (§11.5)
// -----------------------------------------------------------------------------------------------------

export type GetEpochInfoParameters = BlockParameter

/** Current epoch and the block of the next validator-set transition. Node support: **Phase 3**. */
export async function getEpochInfo(client: AnyClient, args?: GetEpochInfoParameters): Promise<EpochInfo> {
  const res = await xdevsRequest(client, 'xdevs_epochInfo', [blockParam(args)])
  return {
    epoch: toBigInt(res.epoch, 'epoch'),
    epochLength: toBigInt(res.epochLength, 'epochLength'),
    startBlock: toBigInt(res.startBlock, 'startBlock'),
    nextTransitionBlock: toBigInt(res.nextTransitionBlock, 'nextTransitionBlock'),
    validatorSetHash: res.validatorSetHash,
  }
}

// -----------------------------------------------------------------------------------------------------
// Decorator
// -----------------------------------------------------------------------------------------------------

export type XdevsActions = {
  /** `xdevs_simulateTransaction` (Phase 4). */
  simulateTransaction: (args: SimulateTransactionParameters) => Promise<SimulationResult>
  /** `xdevs_checkRecipient` (Phase 4). */
  checkRecipient: (args: CheckRecipientParameters) => Promise<RecipientCheck>
  /** `xdevs_getApprovals` (Phase 4). */
  getApprovals: (args: GetApprovalsParameters) => Promise<XdevsApproval[]>
  /** `xdevs_getVaultQueue` (Phase 4). */
  getVaultQueue: (args: GetVaultQueueParameters) => Promise<VaultAction[]>
  /** `xdevs_resolveName` (Phase 4). */
  resolveXnsName: (args: ResolveXnsNameParameters) => Promise<Address | null>
  /** `xdevs_reverseName` (Phase 4). */
  reverseXnsName: (args: ReverseXnsNameParameters) => Promise<string | null>
  /** `xdevs_localFee` (Phase 5). */
  getLocalFee: (args: GetLocalFeeParameters) => Promise<LocalFee>
  /** `xdevs_validators` (Phase 3). */
  getValidators: (args?: GetValidatorsParameters) => Promise<Validator[]>
  /** `xdevs_getCommitCertificate` (Phase 2). */
  getCommit: (args: GetCommitParameters) => Promise<CommitCertificate | null>
  /** `xdevs_epochInfo` (Phase 3). */
  getEpochInfo: (args?: GetEpochInfoParameters) => Promise<EpochInfo>
}

/**
 * Client decorator adding every `xdevs_` action:
 *
 * ```ts
 * const client = createPublicClient({ chain: xdevsDevnet, transport: http() }).extend(xdevsActions)
 * const sim = await client.simulateTransaction({ account, to, value: parseXds('1') })
 * ```
 */
export function xdevsActions<
  transport extends Transport = Transport,
  chain extends Chain | undefined = Chain | undefined,
  account extends Account | undefined = Account | undefined,
>(client: Client<transport, chain, account>): XdevsActions {
  const c = client as unknown as AnyClient
  return {
    simulateTransaction: (args) => simulateTransaction(c, args),
    checkRecipient: (args) => checkRecipient(c, args),
    getApprovals: (args) => getApprovals(c, args),
    getVaultQueue: (args) => getVaultQueue(c, args),
    resolveXnsName: (args) => resolveXnsName(c, args),
    reverseXnsName: (args) => reverseXnsName(c, args),
    getLocalFee: (args) => getLocalFee(c, args),
    getValidators: (args) => getValidators(c, args),
    getCommit: (args) => getCommit(c, args),
    getEpochInfo: (args) => getEpochInfo(c, args),
  }
}

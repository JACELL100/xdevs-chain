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
  RpcVaultEvent,
  SimulationResult,
  Validator,
  VaultAction,
  VaultEvent,
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

export type CheckRecipientParameters = {
  /** Sender (enables the first-time / look-alike / poisoning checks). */
  from?: Address | null | undefined
  /** Recipient: an address or an XNS name (`alice` / `alice.xdevs`). */
  to: Address | string
} & BlockParameter

/**
 * Recipient checks for a transfer (plan §16.3): on-chain `facts` (contract / smart account / EOA lock / XNS name)
 * kept apart from `advice` (first-time recipient, look-alike of a past recipient, address-poisoning pattern,
 * zero / precompile / system address). Node support: **Phase 4**. Advice, not fact.
 */
export async function checkRecipient(client: AnyClient, args: CheckRecipientParameters): Promise<RecipientCheck> {
  const to = /^0x/i.test(args.to) ? checkedAddress(args.to, 'to') : normalizeXnsName(args.to)
  const from = args.from ? checkedAddress(args.from, 'from') : null
  const hasBlock = args.blockNumber !== undefined || args.blockTag !== undefined
  if (hasBlock) return xdevsRequest(client, 'xdevs_checkRecipient', [from, to, blockParam(args)])
  if (from) return xdevsRequest(client, 'xdevs_checkRecipient', [from, to as Address])
  return xdevsRequest(client, 'xdevs_checkRecipient', [to])
}

// -----------------------------------------------------------------------------------------------------
// xdevs_getApprovals — Phase 4 (§16.5)
// -----------------------------------------------------------------------------------------------------

export type GetApprovalsParameters = {
  owner: Address
  /** Only these token contracts (Permit2 allowances on them are included). */
  tokens?: readonly Address[] | undefined
  /** First block of the log scan (default: the node's `max_logs_range` window ending at the block). */
  fromBlock?: bigint | undefined
} & BlockParameter

/**
 * Live approvals of `owner` (for a one-click revoke screen): the node scans approval events of the last
 * `max_logs_range` blocks and checks each pair's live allowance. The indexer API is the full-history source.
 * Node support: **Phase 4**.
 */
export async function getApprovals(client: AnyClient, args: GetApprovalsParameters): Promise<XdevsApproval[]> {
  const owner = checkedAddress(args.owner, 'owner')
  if (args.tokens === undefined && args.fromBlock === undefined) {
    return xdevsRequest(client, 'xdevs_getApprovals', [owner, blockParam(args)])
  }
  const options: { tokens?: Address[]; fromBlock?: Hex } = {}
  if (args.tokens) options.tokens = args.tokens.map((t) => checkedAddress(t, 'token'))
  if (args.fromBlock !== undefined) options.fromBlock = numberToHex(args.fromBlock)
  return xdevsRequest(client, 'xdevs_getApprovals', [owner, options, blockParam(args)])
}

// -----------------------------------------------------------------------------------------------------
// xdevs_getVaultQueue — Phase 4 (§16.2)
// -----------------------------------------------------------------------------------------------------

export type GetVaultQueueParameters = { vault: Address } & BlockParameter

/** Pending (not executed, not cancelled) Guardian Vault actions. Node support: **Phase 4**. */
export async function getVaultQueue(client: AnyClient, args: GetVaultQueueParameters): Promise<VaultAction[]> {
  const res = await xdevsRequest(client, 'xdevs_getVaultQueue', [checkedAddress(args.vault, 'vault'), blockParam(args)])
  return res.map((a) => {
    const action: VaultAction = {
      id: toBigInt(a.id, 'id'),
      target: a.target,
      value: toBigInt(a.value, 'value'),
      data: a.data,
      executeAfter: toBigInt(a.executeAfter, 'executeAfter'),
    }
    if (a.account !== undefined) action.account = a.account
    if (a.expiresAt !== undefined) action.expiresAt = toBigInt(a.expiresAt, 'expiresAt')
    if (a.status !== undefined) action.status = a.status
    if (a.calls !== undefined) {
      action.calls = a.calls.map((c) => ({ target: c.target, value: toBigInt(c.value, 'calls.value'), data: c.data }))
    }
    if (a.cancelVotes !== undefined) action.cancelVotes = a.cancelVotes
    return action
  })
}

// -----------------------------------------------------------------------------------------------------
// eth_subscribe("vaultEvents") — Phase 4 (§16.2)
// -----------------------------------------------------------------------------------------------------

/** Parses a `vaultEvents` notification (quantities to bigint). */
export function parseVaultEvent(raw: RpcVaultEvent): VaultEvent {
  const event: VaultEvent = { type: raw.type, account: raw.account }
  if (raw.id !== undefined) event.id = toBigInt(raw.id, 'id')
  if (raw.target !== undefined) event.target = raw.target
  if (raw.value !== undefined) event.value = toBigInt(raw.value, 'value')
  if (raw.data !== undefined) event.data = raw.data
  if (raw.executeAfter !== undefined) event.executeAfter = toBigInt(raw.executeAfter, 'executeAfter')
  if (raw.by !== undefined) event.by = raw.by
  if (raw.frozenUntil !== undefined) event.frozenUntil = toBigInt(raw.frozenUntil, 'frozenUntil')
  if (raw.epoch !== undefined) event.epoch = raw.epoch
  if (raw.blockNumber !== undefined) event.blockNumber = toBigInt(raw.blockNumber, 'blockNumber')
  if (raw.blockHash !== undefined) event.blockHash = raw.blockHash
  if (raw.transactionHash !== undefined) event.transactionHash = raw.transactionHash
  if (raw.logIndex !== undefined) event.logIndex = Number(toBigInt(raw.logIndex, 'logIndex'))
  return event
}

export type WatchVaultEventsParameters = {
  /** Only events emitted by this smart account (recommended: without it any contract's events are delivered). */
  account?: Address | undefined
  onEvent: (event: VaultEvent) => void
  onError?: ((error: Error) => void) | undefined
}

type SubscribingTransport = {
  subscribe?: (args: {
    params: unknown[]
    onData: (data: unknown) => void
    onError?: (error: unknown) => void
  }) => Promise<{ unsubscribe: () => Promise<unknown> }>
}

/**
 * Streams Guardian Vault alerts (`queued`, `cancelled`, `executed`, `invalidated`, `frozen`, `unfrozen`) through
 * `eth_subscribe("vaultEvents", {account})`. Needs a WebSocket transport (viem `webSocket()`). Returns an unsubscribe
 * function. Node support: **Phase 4**.
 */
export async function watchVaultEvents(client: AnyClient, args: WatchVaultEventsParameters): Promise<() => Promise<void>> {
  const transport = client.transport as unknown as SubscribingTransport
  if (typeof transport.subscribe !== 'function') {
    throw new TypeError('watchVaultEvents needs a WebSocket transport (viem webSocket())')
  }
  const params: unknown[] = ['vaultEvents']
  if (args.account) params.push({ account: checkedAddress(args.account, 'account') })
  const subscription = await transport.subscribe({
    params,
    onData: (data) => {
      const result = (data as { result?: RpcVaultEvent } | undefined)?.result
      if (!result) return
      try {
        args.onEvent(parseVaultEvent(result))
      } catch (e) {
        args.onError?.(e as Error)
      }
    },
    onError: (e) => args.onError?.(e as Error),
  })
  return async () => {
    await subscription.unsubscribe()
  }
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
  /** `eth_subscribe("vaultEvents")` (Phase 4, WebSocket transport). */
  watchVaultEvents: (args: WatchVaultEventsParameters) => Promise<() => Promise<void>>
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
 * const client = createPublicClient({ chain: xdevsEnclave, transport: http() }).extend(xdevsActions)
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
    watchVaultEvents: (args) => watchVaultEvents(c, args),
    resolveXnsName: (args) => resolveXnsName(c, args),
    reverseXnsName: (args) => reverseXnsName(c, args),
    getLocalFee: (args) => getLocalFee(c, args),
    getValidators: (args) => getValidators(c, args),
    getCommit: (args) => getCommit(c, args),
    getEpochInfo: (args) => getEpochInfo(c, args),
  }
}

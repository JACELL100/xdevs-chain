// ERC-4337 v0.9 bundler core for xDEVS Chain: validated mempool, ERC-7562 reputation and limits, multi-op bundles,
// pipelined bundles for entity-disjoint ops, persistence, metrics. Single operator key, no P2P mempool.
import { join } from 'node:path'
import {
  type Address,
  type Hex,
  type PublicClient,
  type TransactionReceipt,
  type WalletClient,
  concat,
  createPublicClient,
  createWalletClient,
  decodeErrorResult,
  decodeEventLog,
  decodeFunctionData,
  encodeFunctionData,
  getAddress,
  getContractAddress,
  http,
  isAddressEqual,
  pad,
  parseAbiItem,
  toHex,
} from 'viem'
import { type PrivateKeyAccount, privateKeyToAccount } from 'viem/accounts'
import { CREATE2_DEPLOYER, entryPointAbi, gasMeterAbi } from './abi.ts'
import { type BundlerConfig, DEFAULT_CONFIG } from './config.ts'
import { RpcCodes, RpcError, codeForReason, describeRevert, revertDataOf } from './errors.ts'
import { GAS_METER_BYTECODE } from './gas-meter-artifact.ts'
import { type Logger, createLogger, loggerFromFn } from './log.ts'
import { Mempool, type PoolEntry, nonceParts } from './mempool.ts'
import { Metrics } from './metrics.ts'
import { TokenBuckets } from './ratelimit.ts'
import { Reputation, type ReputationSnapshot } from './reputation.ts'
import {
  bundleFees,
  checkFees,
  checkPreVerificationGas,
  checkStatic,
  estimatePreVerificationGas,
  isFactory,
  opMaxCost,
  parseValidationData,
  sponsorOf,
} from './rules.ts'
import { JsonStore } from './store.ts'
import {
  ENTRY_POINT_V09,
  type PackedUserOperation,
  type RpcUserOperation,
  type UserOperation,
  fromRpcUserOp,
  hashPacked,
  packUserOp,
  toRpcUserOp,
  unpackUserOp,
} from './userop.ts'

export type BundlerOptions = {
  rpcUrl: string
  privateKey: Hex
  entryPoint?: Address
  /** Receives the bundle fees (default: the operator). */
  beneficiary?: Address
  /** Overrides of the defaults in `config.ts` (the env loader fills all of them). */
  config?: Partial<BundlerConfig>
  logger?: Logger
  /** Legacy: plain message sink (tests pass `() => {}`). */
  log?: (msg: string) => void
  /** How far back (blocks) receipt / by-hash lookups scan logs for ops this process did not submit. */
  logLookback?: bigint
  receiptTimeoutMs?: number
  pollingIntervalMs?: number
}

export type RequestContext = { ip?: string }

export type UserOperationReceipt = {
  userOpHash: Hex
  entryPoint: Address
  sender: Address
  nonce: Hex
  paymaster: Address
  actualGasCost: Hex
  actualGasUsed: Hex
  success: boolean
  reason: Hex
  logs: unknown[]
  receipt: unknown
}

type Included = { op: UserOperation; receipt: UserOperationReceipt; txHash: Hex; blockHash: Hex; blockNumber: bigint }
type Inflight = { txHash: Hex; entries: PoolEntry[]; entities: Set<string>; sentAt: number }
type Head = { number: bigint; baseFee: bigint; timestamp: number; at: number }
type StakeInfo = { staked: boolean; deposit: bigint; at: number }

type PersistedOp = Omit<PoolEntry, 'op'> & { op: RpcUserOperation }
type PersistedState = {
  v: 1
  chainId: number
  entryPoint: Address
  savedAt: number
  ops: PersistedOp[]
  reputation: ReputationSnapshot
  sponsorCounts: Record<string, number>
}

const USER_OPERATION_EVENT = parseAbiItem(
  'event UserOperationEvent(bytes32 indexed userOpHash, address indexed sender, address indexed paymaster, uint256 nonce, bool success, uint256 actualGasCost, uint256 actualGasUsed)',
)

const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000' as Address
const MAX_RECENT = 10_000
const RPC_TIMEOUT_MS = 15_000
const HEARTBEAT_MS = 10_000

const mul = (v: bigint, pct: bigint) => (v * pct + 99n) / 100n
const lc = (a: string) => a.toLowerCase()

class Semaphore {
  private active = 0
  private readonly waiters: (() => void)[] = []
  private readonly max: number
  constructor(max: number) {
    this.max = max
  }
  async run<T>(fn: () => Promise<T>): Promise<T> {
    if (this.active >= this.max) await new Promise<void>((r) => this.waiters.push(r))
    this.active++
    try {
      return await fn()
    } finally {
      this.active--
      this.waiters.shift()?.()
    }
  }
}

export class Bundler {
  readonly entryPoint: Address
  readonly account: PrivateKeyAccount
  readonly beneficiary: Address
  readonly client: PublicClient
  readonly rpcUrl: string
  readonly cfg: BundlerConfig
  readonly metrics = new Metrics()
  readonly mempool = new Mempool()
  readonly reputation: Reputation
  chainId = 0
  meter: Address = ZERO_ADDRESS

  private readonly wallet: WalletClient
  private readonly log: Logger
  private readonly store?: JsonStore<PersistedState>
  private readonly opts: { logLookback: bigint; receiptTimeoutMs: number; pollingIntervalMs: number }
  private readonly included = new Map<Hex, Included>()
  private readonly dropped = new Map<Hex, string>()
  private readonly inflight = new Map<Hex, Inflight>()
  private readonly sponsorCounts = new Map<string, number>()
  private readonly sponsorIpBuckets?: TokenBuckets
  private readonly stakeCache = new Map<string, StakeInfo>()
  private readonly sims: Semaphore
  private readonly tracking = new Set<Promise<void>>()
  private headCache?: Head
  private balanceCache?: { value: bigint; at: number }
  private lastRpcOk = 0
  private nonce: number | undefined
  private building: Promise<Hex[]> | undefined
  private kickTimer: NodeJS.Timeout | undefined
  private timers: NodeJS.Timeout[] = []
  private stopping = false
  private consecutiveFailures = 0
  private pausedUntil = 0
  private lastLowBalanceWarn = 0
  private rpcId = 0

  constructor(o: BundlerOptions) {
    this.rpcUrl = o.rpcUrl
    this.cfg = { ...DEFAULT_CONFIG, ...o.config }
    if (o.entryPoint) this.cfg.entryPoint = o.entryPoint
    if (o.beneficiary) this.cfg.beneficiary = o.beneficiary
    if (o.receiptTimeoutMs) this.cfg.receiptTimeoutMs = o.receiptTimeoutMs
    this.entryPoint = this.cfg.entryPoint ?? ENTRY_POINT_V09
    this.account = privateKeyToAccount(o.privateKey)
    this.beneficiary = this.cfg.beneficiary ?? this.account.address
    this.opts = {
      logLookback: o.logLookback ?? 9_000n,
      receiptTimeoutMs: this.cfg.receiptTimeoutMs,
      pollingIntervalMs: o.pollingIntervalMs ?? 250,
    }
    this.log = o.logger ?? (o.log ? loggerFromFn(o.log) : createLogger(this.cfg.logLevel))
    const transport = http(o.rpcUrl, { timeout: RPC_TIMEOUT_MS, retryCount: 1 })
    this.client = createPublicClient({ transport, pollingInterval: this.opts.pollingIntervalMs })
    this.wallet = createWalletClient({ account: this.account, transport })
    this.reputation = new Reputation({
      minInclusionRateDenominator: this.cfg.minInclusionRateDenominator,
      throttlingSlack: this.cfg.throttlingSlack,
      banSlack: this.cfg.banSlack,
      allowlist: this.cfg.reputationAllowlist,
    })
    this.sims = new Semaphore(this.cfg.maxConcurrentSimulations)
    if (this.cfg.sponsoredOpsPerIpPerHour > 0) {
      this.sponsorIpBuckets = new TokenBuckets(this.cfg.sponsoredOpsPerIpPerHour / 3600, this.cfg.sponsoredOpsPerIpPerHour)
    }
    if (this.cfg.dataDir) {
      this.store = new JsonStore<PersistedState>(join(this.cfg.dataDir, 'bundler-state.json'), 250, (e) =>
        this.log.error('state file', { error: e.message }),
      )
    }
    this.declareMetrics()
  }

  // ------------------------------------------------------------------------------------------- lifecycle
  /** Checks the chain and the EntryPoint, deploys the gas meter if needed, restores the persisted state. */
  async init(): Promise<void> {
    this.chainId = await this.client.getChainId()
    if (this.cfg.expectedChainId && this.chainId !== this.cfg.expectedChainId) {
      throw new Error(`RPC is chain ${this.chainId}, expected CHAIN_ID=${this.cfg.expectedChainId}`)
    }
    const epCode = await this.client.getCode({ address: this.entryPoint })
    if (!epCode || epCode === '0x') throw new Error(`no EntryPoint code at ${this.entryPoint} (chain ${this.chainId})`)
    await this.ensureMeter()
    const bal = await this.balance(true)
    this.log.info('bundler ready', {
      chainId: this.chainId,
      entryPoint: this.entryPoint,
      operator: this.account.address,
      beneficiary: this.beneficiary,
      balanceWei: bal,
      meter: this.meter,
      persistence: this.store?.path ?? 'off',
    })
    if (bal < this.cfg.operatorMinBalance) this.log.warn('operator balance below OPERATOR_MIN_BALANCE: bundling paused until funded')
    await this.restore()
  }

  /** Starts background bundling (every `bundleIntervalMs`) and the heartbeat (balance, head, housekeeping). */
  start(): void {
    const every = (ms: number, fn: () => Promise<unknown>) => {
      const t = setInterval(() => void fn().catch((e) => this.log.warn('background task failed', { error: (e as Error).message })), ms)
      t.unref()
      this.timers.push(t)
    }
    if (this.cfg.bundleIntervalMs > 0) every(this.cfg.bundleIntervalMs, () => this.bundleNow())
    every(HEARTBEAT_MS, () => this.heartbeat())
  }

  /** Graceful stop: no new ops or bundles, waits for a bundle being submitted, persists the mempool. */
  async stop(): Promise<void> {
    this.stopping = true
    for (const t of this.timers) clearInterval(t)
    this.timers = []
    if (this.kickTimer) clearTimeout(this.kickTimer)
    await this.building?.catch(() => {})
    this.persist()
    this.store?.flush()
  }

  /** Waits until no bundle is being built or tracked (tests). */
  async idle(): Promise<void> {
    for (let i = 0; i < 1000; i++) {
      await this.building?.catch(() => {})
      if (this.tracking.size === 0 && !this.building) return
      await Promise.allSettled([...this.tracking])
    }
  }

  private async ensureMeter(): Promise<void> {
    const salt = pad('0x00', { size: 32 })
    const predicted = getContractAddress({ opcode: 'CREATE2', from: CREATE2_DEPLOYER, salt, bytecode: GAS_METER_BYTECODE })
    const code = await this.client.getCode({ address: predicted })
    if (code && code !== '0x') {
      this.meter = predicted
      return
    }
    const deployerCode = await this.client.getCode({ address: CREATE2_DEPLOYER })
    const hasDeployer = !!deployerCode && deployerCode !== '0x'
    const hash = await this.sendTx({
      to: hasDeployer ? CREATE2_DEPLOYER : undefined,
      data: hasDeployer ? concat([salt, GAS_METER_BYTECODE]) : GAS_METER_BYTECODE,
      gas: 1_500_000n,
    })
    const r = await this.client.waitForTransactionReceipt({ hash, timeout: this.opts.receiptTimeoutMs })
    if (r.status !== 'success') throw new Error('gas meter deployment failed')
    this.meter = hasDeployer ? predicted : (r.contractAddress as Address)
    this.log.info('deployed gas meter', { meter: this.meter, tx: hash })
  }

  // ------------------------------------------------------------------------------------------- JSON-RPC
  async handle(method: string, params: unknown[], ctx: RequestContext = {}): Promise<unknown> {
    switch (method) {
      case 'eth_chainId':
        return toHex(this.chainId)
      case 'eth_supportedEntryPoints':
        return [this.entryPoint]
      case 'eth_sendUserOperation': {
        this.checkEntryPoint(params[1])
        try {
          const h = await this.sendUserOperation(this.parseOp(params[0], false), ctx)
          this.metrics.inc('bundler_ops_received_total', 'eth_sendUserOperation calls by result', { result: 'accepted' })
          return h
        } catch (e) {
          const code = e instanceof RpcError ? String(e.code) : 'internal'
          this.metrics.inc('bundler_ops_received_total', 'eth_sendUserOperation calls by result', { result: 'rejected' })
          this.metrics.inc('bundler_ops_rejected_total', 'Rejected UserOperations by JSON-RPC error code', { code })
          throw e
        }
      }
      case 'eth_estimateUserOperationGas':
        this.checkEntryPoint(params[1])
        if (params[2] !== undefined && params[2] !== null) {
          throw new RpcError(RpcCodes.invalidParams, 'state overrides are not supported (the xDEVS node rejects them)')
        }
        return this.estimateUserOperationGas(this.parseOp(params[0], true))
      case 'eth_getUserOperationReceipt':
        return this.getUserOperationReceipt(this.parseHash(params[0]))
      case 'eth_getUserOperationByHash':
        return this.getUserOperationByHash(this.parseHash(params[0]))
      default:
        throw new RpcError(RpcCodes.methodNotFound, `method ${method} not supported by the xDEVS bundler`)
    }
  }

  private parseOp(raw: unknown, partial: boolean): UserOperation {
    try {
      return fromRpcUserOp(raw as Partial<RpcUserOperation>, partial)
    } catch (e) {
      throw new RpcError(RpcCodes.invalidParams, `invalid UserOperation: ${(e as Error).message}`)
    }
  }

  private parseHash(v: unknown): Hex {
    if (typeof v !== 'string' || !/^0x[0-9a-fA-F]{64}$/.test(v)) {
      throw new RpcError(RpcCodes.invalidParams, 'expected a 32-byte userOpHash')
    }
    return v.toLowerCase() as Hex
  }

  private checkEntryPoint(ep: unknown): void {
    if (typeof ep !== 'string' || !/^0x[0-9a-fA-F]{40}$/.test(ep) || !isAddressEqual(ep as Address, this.entryPoint)) {
      throw new RpcError(RpcCodes.invalidParams, `unsupported EntryPoint ${String(ep)} (supported: ${this.entryPoint})`)
    }
  }

  // ------------------------------------------------------------------------------------------- chain reads
  /** eth_call returning `{ result }` or `{ revert }` (raw JSON-RPC, so revert data is never lost). */
  private async rawCall(to: Address, data: Hex): Promise<{ result?: Hex; revert?: Hex; error?: string }> {
    const body = JSON.stringify({
      jsonrpc: '2.0',
      id: ++this.rpcId,
      method: 'eth_call',
      params: [{ from: this.account.address, to, data }, 'latest'],
    })
    const res = await fetch(this.rpcUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body,
      signal: AbortSignal.timeout(RPC_TIMEOUT_MS),
    })
    const json = (await res.json()) as { result?: Hex; error?: { code: number; message: string; data?: unknown } }
    this.lastRpcOk = Date.now()
    if (json.error) return { revert: revertDataOf(json.error), error: json.error.message }
    return { result: json.result }
  }

  private async head(force = false): Promise<Head> {
    const now = Date.now()
    if (!force && this.headCache && now - this.headCache.at < 1_000) return this.headCache
    const b = await this.client.getBlock({ blockTag: 'latest' })
    this.lastRpcOk = now
    this.headCache = { number: b.number, baseFee: b.baseFeePerGas ?? 0n, timestamp: Number(b.timestamp), at: now }
    this.metrics.set('bundler_head_block', 'Latest block seen by the bundler', b.number)
    return this.headCache
  }

  /** Operator balance (cached 5 s); updates the balance gauges. */
  private async balance(force = false): Promise<bigint> {
    const now = Date.now()
    if (!force && this.balanceCache && now - this.balanceCache.at < 5_000) return this.balanceCache.value
    const value = await this.client.getBalance({ address: this.account.address })
    this.lastRpcOk = now
    this.balanceCache = { value, at: now }
    this.metrics.set('bundler_operator_balance_wei', 'Operator (bundle signer) balance in wei', value)
    this.metrics.set('bundler_operator_balance_low', '1 when the operator balance is below OPERATOR_LOW_BALANCE', value < this.cfg.operatorLowBalance ? 1 : 0)
    this.metrics.set('bundler_operator_balance_below_floor', '1 when bundling is paused (balance below OPERATOR_MIN_BALANCE)', value < this.cfg.operatorMinBalance ? 1 : 0)
    return value
  }

  /** EntryPoint stake / deposit of an entity (cached 30 s). */
  private async stakeInfo(a: string): Promise<StakeInfo> {
    const c = this.stakeCache.get(a)
    if (c && Date.now() - c.at < 30_000) return c
    const info = await this.client.readContract({
      address: this.entryPoint,
      abi: entryPointAbi,
      functionName: 'getDepositInfo',
      args: [a as Address],
    })
    const s: StakeInfo = {
      staked: info.staked && info.stake >= this.cfg.minStake && info.unstakeDelaySec >= this.cfg.minUnstakeDelay,
      deposit: info.deposit,
      at: Date.now(),
    }
    this.stakeCache.set(a, s)
    if (this.stakeCache.size > 5_000) this.stakeCache.delete(this.stakeCache.keys().next().value as string)
    return s
  }

  private userOpHash(op: UserOperation): Hex | Promise<Hex> {
    const p = packUserOp(op)
    if (p.initCode.toLowerCase().startsWith('0x7702')) {
      return this.client.readContract({ address: this.entryPoint, abi: entryPointAbi, functionName: 'getUserOpHash', args: [p] }) as Promise<Hex>
    }
    return hashPacked(p, this.chainId, this.entryPoint).toLowerCase() as Hex
  }

  private async sendTx(tx: { to?: Address; data: Hex; gas: bigint; maxFeePerGas?: bigint; maxPriorityFeePerGas?: bigint }): Promise<Hex> {
    if (this.nonce === undefined) {
      this.nonce = await this.client.getTransactionCount({ address: this.account.address, blockTag: 'pending' })
    }
    let maxFeePerGas = tx.maxFeePerGas
    let maxPriorityFeePerGas = tx.maxPriorityFeePerGas
    if (maxFeePerGas === undefined || maxPriorityFeePerGas === undefined) {
      const f = await this.client.estimateFeesPerGas()
      maxFeePerGas = f.maxFeePerGas
      maxPriorityFeePerGas = f.maxPriorityFeePerGas
    }
    try {
      const hash = await this.wallet.sendTransaction({
        account: this.account,
        chain: null,
        to: tx.to ?? null,
        data: tx.data,
        gas: tx.gas,
        maxFeePerGas,
        maxPriorityFeePerGas,
        nonce: this.nonce,
      })
      this.nonce++
      return hash
    } catch (e) {
      this.nonce = undefined // resync next time
      throw e
    }
  }

  // ------------------------------------------------------------------------------------------- send
  async sendUserOperation(op: UserOperation, ctx: RequestContext = {}): Promise<Hex> {
    if (this.stopping) throw new RpcError(RpcCodes.unavailable, 'bundler is shutting down, retry shortly')
    checkStatic(op, this.cfg)
    checkPreVerificationGas(op, this.beneficiary)
    const bal = await this.balance()
    if (bal < this.cfg.operatorMinBalance) {
      throw new RpcError(RpcCodes.unavailable, 'bundler temporarily unavailable (operator balance below its floor), retry later')
    }
    const head = await this.head()
    checkFees(op, head.baseFee, this.cfg)
    const hash = await this.userOpHash(op)
    if (this.included.has(hash)) throw new RpcError(RpcCodes.invalidParams, `UserOperation ${hash} already included`)

    const sender = lc(op.sender)
    const { key, seq } = nonceParts(op.nonce)
    const factory = isFactory(op.factory) ? lc(op.factory) : undefined
    const paymaster = op.paymaster ? lc(op.paymaster) : undefined

    // Sponsorship policy (XdevsPaymaster open / allowlist mode): activation first, then a per-account cap.
    const sp = sponsorOf(op, this.cfg.sponsorPaymaster)
    const policy =
      this.cfg.sponsorPolicy === 'activation' &&
      !!sp?.openMode &&
      (this.cfg.sponsorPolicySponsors.length === 0 || this.cfg.sponsorPolicySponsors.some((s) => isAddressEqual(s, sp.sponsor)))
    const ipKey = ctx.ip ?? 'local'
    if (policy) {
      const used =
        (this.sponsorCounts.get(sender) ?? 0) +
        this.mempool.bySender(sender).filter((e) => e.sponsored && e.op.nonce !== op.nonce).length
      if (used >= this.cfg.sponsorMaxOpsPerSender) {
        throw new RpcError(
          RpcCodes.rejectedByPaymaster,
          `sponsorship policy: this account already used its ${this.cfg.sponsorMaxOpsPerSender} sponsored operation(s); pay gas from the account`,
        )
      }
      if (used === 0 && !factory) {
        throw new RpcError(RpcCodes.rejectedByPaymaster, 'sponsorship policy: the first sponsored operation must deploy the account (activation)')
      }
      if (this.sponsorIpBuckets && !this.sponsorIpBuckets.check(ipKey)) {
        throw new RpcError(RpcCodes.limitExceeded, 'too many sponsored operations from this address, retry later')
      }
    }

    const [senderStake, factoryStake, pmStake, onchainNonce] = await Promise.all([
      this.stakeInfo(sender),
      factory ? this.stakeInfo(factory) : undefined,
      paymaster ? this.stakeInfo(paymaster) : undefined,
      this.client.readContract({ address: this.entryPoint, abi: entryPointAbi, functionName: 'getNonce', args: [op.sender, key] }),
    ])
    if (paymaster && this.cfg.requirePaymasterStake && !pmStake?.staked) {
      throw new RpcError(
        RpcCodes.stakeTooLow,
        `paymaster ${op.paymaster} is not staked in the EntryPoint (needs >= ${this.cfg.minStake} wei with an unstake delay >= ${this.cfg.minUnstakeDelay} s)`,
      )
    }
    const stakes = new Map<string, StakeInfo | undefined>([
      [sender, senderStake],
      ...(factory ? [[factory, factoryStake] as const] : []),
      ...(paymaster ? [[paymaster, pmStake] as const] : []),
    ])
    const actx = {
      onchainSeq: nonceParts(onchainNonce).seq,
      status: (a: string) => this.reputation.status(a as Address),
      isStaked: (a: string) => !!stakes.get(a)?.staked,
      stakedAllowance: (a: string, base: number) => this.reputation.stakedAllowance(a as Address, base),
      paymasterDeposit: pmStake?.deposit,
      limits: this.cfg,
    }
    const cand: PoolEntry = {
      hash,
      op,
      nonceKey: key,
      nonceSeq: seq,
      factory,
      paymaster,
      maxCost: opMaxCost(op),
      addedAt: Date.now(),
      addedBlock: head.number,
      validUntil: 0,
      validAfter: 0,
      status: 'pending',
      attempts: 0,
      sponsored: policy,
    }
    const replacing = this.mempool.checkAdmission(cand, actx)

    // Simulate on current state: handleOps([...pending lower nonces of this sender, op]) is authoritative (nonce,
    // signature, prefund, paymaster, time range); then the gas meter extracts validUntil / validAfter / aggregator.
    const prefix = this.mempool
      .bySender(sender, key)
      .filter((e) => e !== replacing && e.nonceSeq >= actx.onchainSeq && e.nonceSeq < seq)
      .sort((a, b) => (a.nonceSeq < b.nonceSeq ? -1 : 1))
    await this.sims.run(async () => {
      const ops = [...prefix.map((e) => e.op), op]
      const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'handleOps', args: [ops.map(packUserOp), this.beneficiary] })
      const sim = await this.rawCall(this.entryPoint, data)
      if (sim.revert !== undefined || sim.error) {
        const err = this.validationError(sim.revert, sim.error)
        const f = this.failedOp(sim.revert)
        if (f && f.index < prefix.length) {
          throw new RpcError(err.code, `an earlier pending op of this sender is no longer valid (${err.message})`, err.data)
        }
        throw err
      }
      if (prefix.length === 0) this.applyValidationData(cand, await this.measureValidation(op), head.timestamp)
    })

    // Re-check against the mempool as it is now (concurrent requests), then admit.
    const replaced = this.mempool.checkAdmission(cand, actx)
    if (replaced) {
      this.mempool.remove(replaced.hash)
      this.remember(this.dropped, replaced.hash, 'replaced by a higher-fee op')
    }
    this.mempool.put(cand)
    this.reputation.seen(op.sender)
    if (factory) this.reputation.seen(factory as Address)
    if (paymaster) this.reputation.seen(paymaster as Address)
    if (policy) this.sponsorIpBuckets?.take(ipKey)
    this.log.info('op accepted', { userOpHash: hash, sender: op.sender, nonce: op.nonce, paymaster: op.paymaster ?? undefined, factory })
    this.persist()
    this.kick()
    return hash
  }

  /** Runs only the validation part through the gas meter; best effort (handleOps already passed). */
  private async measureValidation(op: UserOperation): Promise<{ account: bigint; paymaster: bigint } | null> {
    try {
      const inner = encodeFunctionData({ abi: gasMeterAbi, functionName: 'measure', args: [packUserOp(op), opMaxCost(op)] })
      const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'delegateAndRevert', args: [this.meter, inner] })
      const res = await this.rawCall(this.entryPoint, data)
      if (!res.revert) return null
      const d = decodeErrorResult({ abi: entryPointAbi, data: res.revert })
      if (d.errorName !== 'DelegateAndRevert') return null
      const m = decodeErrorResult({ abi: gasMeterAbi, data: (d.args as readonly [boolean, Hex])[1] })
      if (m.errorName !== 'Measurement') return null
      const a = m.args as readonly bigint[]
      return { account: a[2]!, paymaster: a[4]! }
    } catch (e) {
      this.log.debug('gas meter validation read failed', { error: (e as Error).message })
      return null
    }
  }

  private applyValidationData(c: PoolEntry, vd: { account: bigint; paymaster: bigint } | null, chainTime: number): void {
    if (!vd) return
    const a = parseValidationData(vd.account)
    const p = parseValidationData(vd.paymaster)
    for (const [who, v] of [['account', a], ['paymaster', p]] as const) {
      if (!v.sigFailed && v.aggregator !== ZERO_ADDRESS) {
        throw new RpcError(RpcCodes.unsupportedAggregator, `${who} returned aggregator ${v.aggregator}: aggregators are not supported`)
      }
    }
    const untils = [a.validUntil, p.validUntil].filter((x) => x > 0)
    c.validUntil = untils.length ? Math.min(...untils) : 0
    c.validAfter = Math.max(a.validAfter, p.validAfter)
    const now = Math.max(chainTime, Math.floor(Date.now() / 1000))
    if (c.validUntil !== 0 && c.validUntil < now + this.cfg.minValidUntilSeconds) {
      throw new RpcError(RpcCodes.outOfTimeRange, `op expires too soon (validUntil ${c.validUntil}; needs >= ${this.cfg.minValidUntilSeconds} s)`)
    }
    if (c.validAfter > now + 300) throw new RpcError(RpcCodes.outOfTimeRange, `op not valid before ${c.validAfter}`)
  }

  private failedOp(revert: Hex | undefined): { index: number; reason: string } | undefined {
    if (!revert) return undefined
    try {
      const d = decodeErrorResult({ abi: entryPointAbi, data: revert })
      if (d.errorName === 'FailedOp' || d.errorName === 'FailedOpWithRevert') {
        const args = d.args as readonly unknown[]
        const aa = String(args[1])
        const reason = d.errorName === 'FailedOpWithRevert' ? `${aa}: ${describeRevert(args[2] as Hex)}` : aa
        return { index: Number(args[0] as bigint), reason }
      }
    } catch {
      /* not an EntryPoint error */
    }
    return undefined
  }

  private validationError(revert: Hex | undefined, message?: string): RpcError {
    if (!revert) return new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `simulation failed: ${message ?? 'unknown'}`)
    const f = this.failedOp(revert)
    if (f) return new RpcError(codeForReason(f.reason), f.reason, { revertData: revert })
    return new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `handleOps simulation reverted: ${describeRevert(revert)}`, {
      revertData: revert,
    })
  }

  // ------------------------------------------------------------------------------------------- bundling
  /** Schedules a bundle attempt shortly (batches ops that arrive together). */
  private kick(): void {
    if (this.cfg.bundleIntervalMs === 0 || this.kickTimer || this.stopping) return
    this.kickTimer = setTimeout(() => {
      this.kickTimer = undefined
      void this.bundleNow().catch((e) => this.log.warn('bundling failed', { error: (e as Error).message }))
    }, this.cfg.bundleCollectMs)
    this.kickTimer.unref()
  }

  /** Builds and submits bundles until the mempool has nothing eligible or the in-flight limit is reached. */
  bundleNow(): Promise<Hex[]> {
    if (this.building) return this.building.then(() => [])
    this.building = this.buildAndSubmit().finally(() => {
      this.building = undefined
      this.updateGauges()
    })
    return this.building
  }

  private async buildAndSubmit(): Promise<Hex[]> {
    const sent: Hex[] = []
    while (!this.stopping && this.inflight.size < this.cfg.maxInflightBundles && Date.now() >= this.pausedUntil) {
      const head = await this.head(true)
      this.housekeeping(head)
      if (!this.mempool.all().some((e) => e.status === 'pending')) break
      const bal = await this.balance(true)
      if (bal < this.cfg.operatorMinBalance) {
        if (Date.now() - this.lastLowBalanceWarn > 60_000) {
          this.lastLowBalanceWarn = Date.now()
          this.log.warn('operator balance below OPERATOR_MIN_BALANCE: not submitting bundles', { balanceWei: bal })
        }
        break
      }
      const busy = new Set<string>()
      for (const b of this.inflight.values()) for (const x of b.entities) busy.add(x)
      let bundle = this.mempool.select({
        baseFee: head.baseFee,
        busy,
        maxOps: this.cfg.maxBundleOps,
        maxGas: this.cfg.maxBundleGas,
        nowSec: Math.max(head.timestamp, Math.floor(Date.now() / 1000)),
        minValidSeconds: Math.ceil(this.cfg.minValidUntilSeconds / 2),
        status: (a) => this.reputation.status(a as Address),
        throttledBundleCount: this.cfg.throttledEntityBundleCount,
      })
      if (bundle.length === 0) break
      bundle = await this.resimulate(bundle)
      if (bundle.length === 0) continue
      const ops = bundle.map((e) => e.op)
      const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'handleOps', args: [ops.map(packUserOp), this.beneficiary] })
      let gas: bigint
      try {
        gas = await this.client.estimateGas({ account: this.account.address, to: this.entryPoint, data })
      } catch (e) {
        // Simulation passed but estimation failed: retry later, give up on the first op after a few tries.
        const first = bundle[0]!
        first.attempts++
        if (first.attempts >= 3) this.drop(first, `bundle gas estimation failed: ${(e as Error).message.split('\n')[0]}`)
        this.log.warn('bundle gas estimation failed', { error: (e as Error).message.split('\n')[0], ops: bundle.length })
        break
      }
      const gasLimit = mul(gas, 120n) + 20_000n
      const fees = bundleFees(ops)
      if (bal < gasLimit * fees.maxFeePerGas) {
        this.log.warn('operator balance too low for the next bundle', { balanceWei: bal, needWei: gasLimit * fees.maxFeePerGas })
        break
      }
      let txHash: Hex
      try {
        txHash = await this.sendTx({ to: this.entryPoint, data, gas: gasLimit, ...fees })
      } catch (e) {
        this.log.error('bundle submission failed', { error: (e as Error).message.split('\n')[0], ops: bundle.length })
        this.metrics.inc('bundler_bundles_failed_total', 'Bundles that failed (submission error or on-chain revert)', { stage: 'submit' })
        break
      }
      for (const e of bundle) {
        e.status = 'submitted'
        e.txHash = txHash
      }
      const b: Inflight = { txHash, entries: bundle, entities: Mempool.entities(bundle), sentAt: Date.now() }
      this.inflight.set(txHash, b)
      this.metrics.inc('bundler_bundles_submitted_total', 'Bundles submitted')
      this.metrics.inc('bundler_bundle_ops_total', 'UserOperations put into submitted bundles', {}, bundle.length)
      this.log.info('bundle submitted', { tx: txHash, ops: bundle.length, gasLimit, maxFeePerGas: fees.maxFeePerGas, userOpHashes: bundle.map((e) => e.hash) })
      sent.push(txHash)
      this.persist()
      this.trackInBackground(b)
    }
    return sent
  }

  /** Simulates the whole bundle; drops ops invalidated by earlier ones (or by new state) until it passes. */
  private async resimulate(bundle: PoolEntry[]): Promise<PoolEntry[]> {
    let list = [...bundle]
    for (let i = 0; i < bundle.length + 2 && list.length > 0; i++) {
      const data = encodeFunctionData({
        abi: entryPointAbi,
        functionName: 'handleOps',
        args: [list.map((e) => packUserOp(e.op)), this.beneficiary],
      })
      const sim = await this.sims.run(() => this.rawCall(this.entryPoint, data))
      if (sim.revert === undefined && !sim.error) return list
      const f = this.failedOp(sim.revert)
      if (f && list[f.index]) {
        this.drop(list[f.index]!, `invalid at bundle time: ${f.reason}`)
        list.splice(f.index, 1)
        continue
      }
      if (!sim.revert) throw new Error(`bundle simulation failed: ${sim.error ?? 'unknown'}`) // RPC trouble: retry later
      if (list.length > 1) {
        list = [list[0]!]
        continue
      }
      this.drop(list[0]!, `bundle simulation reverted: ${describeRevert(sim.revert)}`)
      list = []
    }
    return list
  }

  private trackInBackground(b: Inflight): void {
    const p = this.track(b)
      .catch((e) => this.log.error('bundle tracking failed', { tx: b.txHash, error: (e as Error).message }))
      .finally(() => {
        this.tracking.delete(p)
        this.updateGauges()
        this.kick()
      })
    this.tracking.add(p)
  }

  private async track(b: Inflight): Promise<void> {
    let r: TransactionReceipt | undefined
    for (let round = 0; !r; round++) {
      try {
        r = await this.client.waitForTransactionReceipt({
          hash: b.txHash,
          timeout: this.opts.receiptTimeoutMs,
          pollingInterval: this.opts.pollingIntervalMs,
        })
      } catch (e) {
        // Still known to the node: keep waiting (re-bundling its ops would double-spend their nonces).
        const known = await this.client.getTransaction({ hash: b.txHash }).catch(() => null)
        if (known && round < 10 && !this.stopping) {
          this.log.warn('bundle not mined yet', { tx: b.txHash, waitedMs: Date.now() - b.sentAt })
          continue
        }
        if (this.stopping) return // persisted as submitted; resolved at the next start
        this.log.error('bundle dropped by the node; ops go back to the mempool', { tx: b.txHash, error: (e as Error).message.split('\n')[0] })
        this.nonce = undefined
        this.inflight.delete(b.txHash)
        this.metrics.inc('bundler_bundles_failed_total', 'Bundles that failed (submission error or on-chain revert)', { stage: 'dropped' })
        for (const e of b.entries) {
          e.status = 'pending'
          e.txHash = undefined
          e.attempts++
          if (e.attempts >= 3) this.drop(e, 'bundle not mined three times')
        }
        this.persist()
        return
      }
    }
    this.inflight.delete(b.txHash)
    if (r.status === 'success') this.onIncluded(b, r)
    else await this.onReverted(b, r)
    this.persist()
  }

  private onIncluded(b: Inflight, r: TransactionReceipt): void {
    let collected = 0n
    for (const e of b.entries) {
      const receipt = this.buildReceipt(e.hash, r)
      this.mempool.remove(e.hash)
      if (!receipt) {
        this.remember(this.dropped, e.hash, 'missing from its bundle receipt')
        continue
      }
      this.remember(this.included, e.hash, { op: e.op, receipt, txHash: r.transactionHash, blockHash: r.blockHash, blockNumber: r.blockNumber })
      this.reputation.included(e.op.sender)
      if (e.factory) this.reputation.included(e.factory as Address)
      if (e.paymaster) this.reputation.included(e.paymaster as Address)
      if (e.sponsored) this.sponsorCounts.set(lc(e.op.sender), (this.sponsorCounts.get(lc(e.op.sender)) ?? 0) + 1)
      collected += BigInt(receipt.actualGasCost)
    }
    const spent = r.gasUsed * r.effectiveGasPrice
    const net = collected - spent
    this.consecutiveFailures = 0
    this.metrics.inc('bundler_bundles_included_total', 'Bundles mined successfully')
    this.metrics.inc('bundler_ops_included_total', 'UserOperations included on chain', {}, b.entries.length)
    this.metrics.inc('bundler_fees_collected_wei_total', 'Fees paid to the beneficiary by included ops (wei)', {}, collected)
    this.metrics.inc('bundler_gas_spent_wei_total', 'Gas paid by the operator for bundles (wei)', {}, spent)
    const level = net < 0n ? 'warn' : 'info'
    this.log[level]('bundle included', { tx: r.transactionHash, block: r.blockNumber, ops: b.entries.length, gasUsed: r.gasUsed, netWei: net })
  }

  private async onReverted(b: Inflight, r: TransactionReceipt): Promise<void> {
    const spent = r.gasUsed * r.effectiveGasPrice
    this.consecutiveFailures++
    this.metrics.inc('bundler_bundles_failed_total', 'Bundles that failed (submission error or on-chain revert)', { stage: 'reverted' })
    this.metrics.inc('bundler_gas_spent_wei_total', 'Gas paid by the operator for bundles (wei)', {}, spent)
    this.log.error('bundle reverted on chain', { tx: r.transactionHash, block: r.blockNumber, ops: b.entries.length, lostWei: spent })
    // Find who broke it: re-simulate each op alone (with its same-sender predecessors from this bundle).
    for (const e of b.entries) {
      e.status = 'pending'
      e.txHash = undefined
      e.attempts++
    }
    for (const e of b.entries) {
      if (!this.mempool.get(e.hash)) continue
      const prior = b.entries.filter((x) => lc(x.op.sender) === lc(e.op.sender) && x.nonceKey === e.nonceKey && x.nonceSeq < e.nonceSeq)
      if (prior.length) continue // judged together with its predecessor
      const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'handleOps', args: [[packUserOp(e.op)], this.beneficiary] })
      const sim = await this.rawCall(this.entryPoint, data).catch(() => ({ revert: undefined, error: 'rpc' }) as { revert?: Hex; error?: string })
      const f = this.failedOp(sim.revert)
      if (f) {
        const culprit = /^AA1/.test(f.reason) ? e.factory : /^AA3/.test(f.reason) ? e.paymaster : /^AA25/.test(f.reason) ? undefined : lc(e.op.sender)
        if (culprit) {
          this.reputation.crashed(culprit as Address)
          this.log.warn('entity banned: its op passed simulation but failed on chain', { entity: culprit, reason: f.reason, userOpHash: e.hash })
        }
        this.drop(e, `failed on chain: ${f.reason}`)
      } else if (e.attempts >= 2) {
        this.drop(e, 'in two reverted bundles')
      }
    }
    if (this.consecutiveFailures >= 3) {
      this.pausedUntil = Date.now() + 60_000
      this.log.error('three bundles in a row failed: pausing bundling for 60 s')
    }
  }

  private drop(e: PoolEntry, reason: string): void {
    this.mempool.remove(e.hash)
    this.remember(this.dropped, e.hash, reason)
    this.metrics.inc('bundler_ops_dropped_total', 'UserOperations dropped from the mempool after admission')
    this.log.warn('op dropped', { userOpHash: e.hash, sender: e.op.sender, reason })
  }

  private housekeeping(head: Head): void {
    const stale = this.mempool.stale({
      nowMs: Date.now(),
      nowSec: Math.max(head.timestamp, Math.floor(Date.now() / 1000)),
      block: head.number,
      ttlSeconds: this.cfg.mempoolTtlSeconds,
      liveBlocks: this.cfg.throttledEntityLiveBlocks,
      status: (a) => this.reputation.status(a as Address),
    })
    for (const s of stale) this.drop(s.entry, s.reason)
    if (stale.length) this.persist()
  }

  private async heartbeat(): Promise<void> {
    try {
      const head = await this.head(true)
      await this.balance(true)
      this.housekeeping(head)
      this.metrics.set('bundler_rpc_up', '1 when the last RPC call succeeded recently', 1)
    } catch (e) {
      this.metrics.set('bundler_rpc_up', '1 when the last RPC call succeeded recently', 0)
      this.log.warn('RPC heartbeat failed', { error: (e as Error).message.split('\n')[0] })
    }
    this.reputation.decay()
    this.updateGauges()
    this.persist()
  }

  // ------------------------------------------------------------------------------------------- receipts / lookups
  private buildReceipt(userOpHash: Hex, r: TransactionReceipt): UserOperationReceipt | null {
    let ev: { sender: Address; paymaster: Address; nonce: bigint; success: boolean; actualGasCost: bigint; actualGasUsed: bigint } | undefined
    let reason: Hex = '0x'
    // ERC-4337 receipt logs: the logs of this op's execution = after BeforeExecution or the previous op's
    // UserOperationEvent, up to its own UserOperationEvent.
    let start = 0
    let end = -1
    for (let i = 0; i < r.logs.length; i++) {
      const log = r.logs[i]!
      if (!isAddressEqual(log.address, this.entryPoint)) continue
      try {
        const d = decodeEventLog({ abi: entryPointAbi, data: log.data, topics: log.topics })
        const a = d.args as Record<string, unknown>
        if (d.eventName === 'BeforeExecution') start = i + 1
        if (d.eventName === 'UserOperationRevertReason' && String(a.userOpHash).toLowerCase() === userOpHash) reason = a.revertReason as Hex
        if (d.eventName === 'UserOperationEvent') {
          if (String(a.userOpHash).toLowerCase() === userOpHash) {
            ev = a as typeof ev
            end = i
            break
          }
          start = i + 1
        }
      } catch {
        /* other event */
      }
    }
    if (!ev) return null
    const json = (v: unknown): unknown => JSON.parse(JSON.stringify(v, (_k, x) => (typeof x === 'bigint' ? toHex(x) : x)))
    return {
      userOpHash,
      entryPoint: this.entryPoint,
      sender: ev.sender,
      nonce: toHex(ev.nonce),
      paymaster: ev.paymaster,
      actualGasCost: toHex(ev.actualGasCost),
      actualGasUsed: toHex(ev.actualGasUsed),
      success: ev.success,
      reason,
      logs: json(r.logs.slice(start, end)) as unknown[],
      receipt: json(r),
    }
  }

  private async findOnChain(userOpHash: Hex): Promise<TransactionReceipt | null> {
    const latest = await this.client.getBlockNumber()
    const fromBlock = latest > this.opts.logLookback ? latest - this.opts.logLookback : 0n
    const logs = await this.client.getLogs({
      address: this.entryPoint,
      event: USER_OPERATION_EVENT,
      args: { userOpHash },
      fromBlock,
      toBlock: latest,
    })
    const log = logs[0]
    if (!log?.transactionHash) return null
    return this.client.getTransactionReceipt({ hash: log.transactionHash })
  }

  async getUserOperationReceipt(userOpHash: Hex): Promise<UserOperationReceipt | null> {
    const inc = this.included.get(userOpHash)
    if (inc) return inc.receipt
    if (this.mempool.get(userOpHash)) return null
    const r = await this.findOnChain(userOpHash)
    return r ? this.buildReceipt(userOpHash, r) : null
  }

  async getUserOperationByHash(userOpHash: Hex): Promise<unknown> {
    const inc = this.included.get(userOpHash)
    if (inc) {
      return {
        userOperation: toRpcUserOp(inc.op),
        entryPoint: this.entryPoint,
        transactionHash: inc.txHash,
        blockHash: inc.blockHash,
        blockNumber: toHex(inc.blockNumber),
      }
    }
    const e = this.mempool.get(userOpHash)
    if (e) return { userOperation: toRpcUserOp(e.op), entryPoint: this.entryPoint, transactionHash: null, blockHash: null, blockNumber: null }
    const r = await this.findOnChain(userOpHash)
    if (!r) return null
    const tx = await this.client.getTransaction({ hash: r.transactionHash })
    const d = decodeFunctionData({ abi: entryPointAbi, data: tx.input })
    if (d.functionName !== 'handleOps') return null
    const ops = (d.args as readonly [readonly PackedUserOperation[], Address])[0]
    for (const p of ops) {
      const h = p.initCode.toLowerCase().startsWith('0x7702')
        ? ((await this.client.readContract({ address: this.entryPoint, abi: entryPointAbi, functionName: 'getUserOpHash', args: [p] })) as Hex)
        : hashPacked(p, this.chainId, this.entryPoint)
      if (h.toLowerCase() === userOpHash) {
        return {
          userOperation: toRpcUserOp(unpackUserOp(p)),
          entryPoint: this.entryPoint,
          transactionHash: r.transactionHash,
          blockHash: r.blockHash,
          blockNumber: toHex(r.blockNumber),
        }
      }
    }
    return null
  }

  // ------------------------------------------------------------------------------------------- estimation
  /**
   * Measures deployment, validation, paymaster validation / postOp and execution gas by running the gas meter
   * inside the EntryPoint (`delegateAndRevert`), then adds margins. Signature failures are ignored (callers send a
   * dummy signature of the right shape); everything else that reverts is reported.
   */
  async estimateUserOperationGas(input: UserOperation): Promise<Record<string, Hex>> {
    checkStatic(input, this.cfg, true)
    const op: UserOperation = { ...input }
    if (op.maxFeePerGas === 0n) {
      const f = await this.client.estimateFeesPerGas()
      op.maxFeePerGas = f.maxFeePerGas
      op.maxPriorityFeePerGas = f.maxPriorityFeePerGas
    }
    // Nominal limits used only for the paymaster's maxCost check during measurement.
    const nominal = {
      verificationGasLimit: op.verificationGasLimit || 600_000n,
      callGasLimit: op.callGasLimit || 300_000n,
      paymasterVerificationGasLimit: op.paymaster ? op.paymasterVerificationGasLimit || 150_000n : 0n,
      paymasterPostOpGasLimit: op.paymaster ? op.paymasterPostOpGasLimit || 80_000n : 0n,
      preVerificationGas: op.preVerificationGas || 80_000n,
    }
    Object.assign(op, nominal)
    const maxCost = opMaxCost(op)

    const packed = packUserOp(op)
    const inner = encodeFunctionData({ abi: gasMeterAbi, functionName: 'measure', args: [packed, maxCost] })
    const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'delegateAndRevert', args: [this.meter, inner] })
    const res = await this.sims.run(() => this.rawCall(this.entryPoint, data))
    if (!res.revert) throw new RpcError(RpcCodes.internal, `gas meter returned no revert data (${res.error ?? 'ok?'})`)
    let ret: Hex
    try {
      const d = decodeErrorResult({ abi: entryPointAbi, data: res.revert })
      if (d.errorName !== 'DelegateAndRevert') throw new Error(d.errorName)
      ret = (d.args as readonly [boolean, Hex])[1]
    } catch {
      throw new RpcError(RpcCodes.internal, `gas meter failed: ${describeRevert(res.revert)}`)
    }
    const m = decodeErrorResult({ abi: gasMeterAbi, data: ret })
    if (m.errorName === 'StepFailed') {
      const [step, reason] = m.args as readonly [number, Hex]
      const why = describeRevert(reason)
      if (step === 1) throw new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `AA13 initCode failed: ${why}`, { revertData: reason })
      if (step === 3) throw new RpcError(RpcCodes.rejectedByPaymaster, `AA33 reverted: ${why}`, { revertData: reason })
      throw new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `AA23 reverted: ${why}`, { revertData: reason })
    }
    const [deployGas, validationGas, , pmGas, , postOpGas, callSuccess, callGas, callResult] = m.args as readonly [
      bigint, bigint, bigint, bigint, bigint, bigint, boolean, bigint, Hex,
    ]
    if (!callSuccess) {
      throw new RpcError(RpcCodes.executionReverted, `execution reverted: ${describeRevert(callResult)}`, { revertData: callResult })
    }
    // Margins: +20 % plus a fixed amount (dummy passkey signatures skip SHA-256 / P256VERIFY work; prefund transfer).
    const out: UserOperation = {
      ...op,
      verificationGasLimit: mul(deployGas + validationGas, 120n) + 60_000n,
      callGasLimit: callGas === 0n ? 0n : mul(callGas, 120n) + 25_000n,
      paymasterVerificationGasLimit: op.paymaster ? mul(pmGas, 120n) + 20_000n : null,
      paymasterPostOpGasLimit: op.paymaster ? (postOpGas > 0n ? mul(postOpGas, 120n) + 15_000n : 40_000n) : null,
    }
    out.preVerificationGas = estimatePreVerificationGas(out, this.beneficiary)
    const result: Record<string, Hex> = {
      preVerificationGas: toHex(out.preVerificationGas),
      verificationGasLimit: toHex(out.verificationGasLimit),
      callGasLimit: toHex(out.callGasLimit),
    }
    if (op.paymaster) {
      result.paymasterVerificationGasLimit = toHex(out.paymasterVerificationGasLimit ?? 0n)
      result.paymasterPostOpGasLimit = toHex(out.paymasterPostOpGasLimit ?? 0n)
    }
    return result
  }

  /** preVerificationGas `eth_estimateUserOperationGas` would return for `op`. */
  preVerificationGas(op: UserOperation): bigint {
    return estimatePreVerificationGas(op, this.beneficiary)
  }

  // ------------------------------------------------------------------------------------------- ops surface
  /** `/health` payload; `ok` is false when the RPC is unreachable or the operator cannot pay for bundles. */
  health(): { ok: boolean; body: Record<string, unknown> } {
    const rpcOk = Date.now() - this.lastRpcOk < 3 * HEARTBEAT_MS
    const bal = this.balanceCache?.value
    const funded = bal !== undefined && bal >= this.cfg.operatorMinBalance
    const ok = rpcOk && funded && !this.stopping
    return {
      ok,
      body: {
        status: ok ? 'ok' : 'degraded',
        chainId: this.chainId,
        entryPoint: this.entryPoint,
        operator: this.account.address,
        rpc: rpcOk ? 'ok' : 'unreachable',
        operatorBalance: funded ? (bal! < this.cfg.operatorLowBalance ? 'low' : 'ok') : 'below-floor',
        mempool: this.mempool.size,
        inflightBundles: this.inflight.size,
        headBlock: this.headCache ? Number(this.headCache.number) : null,
        pausedUntil: this.pausedUntil > Date.now() ? new Date(this.pausedUntil).toISOString() : null,
      },
    }
  }

  private declareMetrics(): void {
    const m = this.metrics
    m.declare('bundler_ops_received_total', 'eth_sendUserOperation calls by result', 'counter', { result: 'accepted' })
    m.declare('bundler_ops_received_total', 'eth_sendUserOperation calls by result', 'counter', { result: 'rejected' })
    m.declare('bundler_ops_included_total', 'UserOperations included on chain', 'counter')
    m.declare('bundler_ops_dropped_total', 'UserOperations dropped from the mempool after admission', 'counter')
    m.declare('bundler_bundles_submitted_total', 'Bundles submitted', 'counter')
    m.declare('bundler_bundles_included_total', 'Bundles mined successfully', 'counter')
    m.declare('bundler_bundles_failed_total', 'Bundles that failed (submission error or on-chain revert)', 'counter', { stage: 'reverted' })
    m.declare('bundler_fees_collected_wei_total', 'Fees paid to the beneficiary by included ops (wei)', 'counter')
    m.declare('bundler_gas_spent_wei_total', 'Gas paid by the operator for bundles (wei)', 'counter')
    this.updateGauges()
  }

  private updateGauges(): void {
    const all = this.mempool.all()
    const pending = all.filter((e) => e.status === 'pending').length
    this.metrics.set('bundler_mempool_ops', 'UserOperations in the mempool', pending, { status: 'pending' })
    this.metrics.set('bundler_mempool_ops', 'UserOperations in the mempool', all.length - pending, { status: 'submitted' })
    this.metrics.set('bundler_inflight_bundles', 'Bundles submitted and not yet mined', this.inflight.size)
    const c = this.reputation.counts()
    for (const s of ['ok', 'throttled', 'banned'] as const) {
      this.metrics.set('bundler_reputation_entities', 'Entities tracked by reputation, by status', c[s], { status: s })
    }
  }

  metricsText(): string {
    this.updateGauges()
    return this.metrics.render()
  }

  // ------------------------------------------------------------------------------------------- persistence
  private remember<T>(m: Map<Hex, T>, k: Hex, v: T): void {
    m.delete(k)
    m.set(k, v)
    if (m.size > MAX_RECENT) m.delete(m.keys().next().value as Hex)
  }

  private snapshot(): PersistedState {
    return {
      v: 1,
      chainId: this.chainId,
      entryPoint: this.entryPoint,
      savedAt: Date.now(),
      ops: this.mempool.all().map((e) => ({ ...e, op: toRpcUserOp(e.op) })),
      reputation: this.reputation.snapshot(),
      sponsorCounts: Object.fromEntries(this.sponsorCounts),
    }
  }

  private persist(): void {
    this.store?.schedule(() => this.snapshot())
  }

  private async restore(): Promise<void> {
    const s = this.store?.load()
    if (!s) return
    if (s.v !== 1 || s.chainId !== this.chainId || !isAddressEqual(s.entryPoint, this.entryPoint)) {
      this.log.warn('state file is for another chain / EntryPoint: ignored', { file: this.store?.path })
      return
    }
    this.reputation.restore(s.reputation)
    for (const [k, v] of Object.entries(s.sponsorCounts ?? {})) if (Number.isInteger(v)) this.sponsorCounts.set(lc(k), v)
    const submitted = new Map<Hex, PoolEntry[]>()
    for (const p of s.ops ?? []) {
      try {
        const e: PoolEntry = { ...p, op: fromRpcUserOp(p.op) }
        e.op.sender = getAddress(e.op.sender)
        this.mempool.put(e)
        if (e.status === 'submitted' && e.txHash) submitted.set(e.txHash, [...(submitted.get(e.txHash) ?? []), e])
        else e.status = 'pending'
      } catch (err) {
        this.log.warn('skipping unreadable persisted op', { error: (err as Error).message })
      }
    }
    // Bundles submitted before the restart: settle the mined ones, keep waiting for known ones, re-queue the rest.
    for (const [txHash, entries] of submitted) {
      const b: Inflight = { txHash, entries, entities: Mempool.entities(entries), sentAt: Date.now() }
      const r = await this.client.getTransactionReceipt({ hash: txHash }).catch(() => null)
      if (r) {
        if (r.status === 'success') this.onIncluded(b, r)
        else await this.onReverted(b, r)
        continue
      }
      const known = await this.client.getTransaction({ hash: txHash }).catch(() => null)
      if (known) {
        this.inflight.set(txHash, b)
        this.trackInBackground(b)
      } else {
        for (const e of entries) {
          e.status = 'pending'
          e.txHash = undefined
        }
      }
    }
    this.log.info('restored state', { ops: this.mempool.size, inflightBundles: this.inflight.size, file: this.store?.path })
    this.updateGauges()
    this.persist()
  }
}

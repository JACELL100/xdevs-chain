// DEV-ONLY ERC-4337 v0.9 bundler core. Single operator, one UserOperation per bundle, bundles strictly sequential.
// No mempool, no ERC-7562 opcode/storage rules, no reputation, no P2P: do not expose to the public internet.
import {
  type Address,
  type Hex,
  type PublicClient,
  type TransactionReceipt,
  concat,
  createPublicClient,
  createWalletClient,
  decodeErrorResult,
  decodeEventLog,
  decodeFunctionData,
  encodeFunctionData,
  getContractAddress,
  http,
  isAddressEqual,
  pad,
  parseAbiItem,
  size,
  toHex,
} from 'viem'
import { type PrivateKeyAccount, privateKeyToAccount } from 'viem/accounts'
import { CREATE2_DEPLOYER, entryPointAbi, gasMeterAbi } from './abi.ts'
import { RpcCodes, RpcError, codeForReason, describeRevert, revertDataOf } from './errors.ts'
import { GAS_METER_BYTECODE } from './gas-meter-artifact.ts'
import {
  ENTRY_POINT_V09,
  type PackedUserOperation,
  type RpcUserOperation,
  type UserOperation,
  calldataGas,
  fromRpcUserOp,
  packUserOp,
  toRpcUserOp,
  unpackUserOp,
} from './userop.ts'

export type BundlerOptions = {
  rpcUrl: string
  privateKey: Hex
  entryPoint?: Address
  /** Receives the bundle fees (default: the bundler key). */
  beneficiary?: Address
  /** How far back (blocks) receipt / by-hash lookups scan logs for ops this process did not submit. */
  logLookback?: bigint
  receiptTimeoutMs?: number
  pollingIntervalMs?: number
  log?: (msg: string) => void
}

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

type Entry = {
  op: UserOperation
  hash: Hex
  txHash?: Hex
  status: 'submitted' | 'included' | 'failed'
  receipt?: UserOperationReceipt
  blockHash?: Hex
  blockNumber?: bigint
  error?: string
}

const USER_OPERATION_EVENT = parseAbiItem(
  'event UserOperationEvent(bytes32 indexed userOpHash, address indexed sender, address indexed paymaster, uint256 nonce, bool success, uint256 actualGasCost, uint256 actualGasUsed)',
)

const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000' as Address

/** Fixed per-bundle overhead the EntryPoint does not meter per op (tx intrinsic + loop + compensation). */
const PVG_FIXED = 21_000n
const PVG_PER_OP = 18_300n

const mul = (v: bigint, pct: bigint) => (v * pct + 99n) / 100n

export class Bundler {
  readonly entryPoint: Address
  readonly account: PrivateKeyAccount
  readonly beneficiary: Address
  readonly client: PublicClient
  readonly rpcUrl: string
  chainId = 0
  meter: Address = ZERO_ADDRESS
  private readonly entries = new Map<Hex, Entry>()
  private queue: Promise<unknown> = Promise.resolve()
  private nonce: number | undefined
  private readonly opts: Required<Pick<BundlerOptions, 'logLookback' | 'receiptTimeoutMs' | 'pollingIntervalMs'>>
  private readonly log: (msg: string) => void
  private rpcId = 0

  constructor(o: BundlerOptions) {
    this.rpcUrl = o.rpcUrl
    this.entryPoint = o.entryPoint ?? ENTRY_POINT_V09
    this.account = privateKeyToAccount(o.privateKey)
    this.beneficiary = o.beneficiary ?? this.account.address
    this.opts = {
      logLookback: o.logLookback ?? 9_000n,
      receiptTimeoutMs: o.receiptTimeoutMs ?? 120_000,
      pollingIntervalMs: o.pollingIntervalMs ?? 250,
    }
    this.log = o.log ?? ((m) => console.log(`[bundler] ${m}`))
    this.client = createPublicClient({ transport: http(o.rpcUrl), pollingInterval: this.opts.pollingIntervalMs })
  }

  /** Checks the chain, the EntryPoint and deploys the gas meter helper if needed. */
  async init(): Promise<void> {
    this.chainId = await this.client.getChainId()
    const epCode = await this.client.getCode({ address: this.entryPoint })
    if (!epCode || epCode === '0x') throw new Error(`no EntryPoint code at ${this.entryPoint} (chain ${this.chainId})`)
    await this.ensureMeter()
    const bal = await this.client.getBalance({ address: this.account.address })
    this.log(
      `chain ${this.chainId}, EntryPoint ${this.entryPoint}, operator ${this.account.address} (${bal} wei), meter ${this.meter}`,
    )
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
    this.log(`deployed gas meter at ${this.meter}`)
  }

  // ------------------------------------------------------------------------------------------- JSON-RPC
  async handle(method: string, params: unknown[]): Promise<unknown> {
    switch (method) {
      case 'eth_chainId':
        return toHex(this.chainId)
      case 'eth_supportedEntryPoints':
        return [this.entryPoint]
      case 'eth_sendUserOperation':
        this.checkEntryPoint(params[1])
        return this.sendUserOperation(this.parseOp(params[0], false))
      case 'eth_estimateUserOperationGas':
        this.checkEntryPoint(params[1])
        if (params[2] !== undefined && params[2] !== null) {
          throw new RpcError(RpcCodes.invalidParams, 'state overrides are not supported by the dev bundler')
        }
        return this.estimateUserOperationGas(this.parseOp(params[0], true))
      case 'eth_getUserOperationReceipt':
        return this.getUserOperationReceipt(this.parseHash(params[0]))
      case 'eth_getUserOperationByHash':
        return this.getUserOperationByHash(this.parseHash(params[0]))
      default:
        throw new RpcError(RpcCodes.methodNotFound, `method ${method} not supported by the xDEVS dev bundler`)
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

  // ------------------------------------------------------------------------------------------- raw RPC
  /** eth_call returning `{ result }` or `{ revert }` (raw JSON-RPC, so revert data is never lost). */
  private async rawCall(to: Address, data: Hex): Promise<{ result?: Hex; revert?: Hex; error?: string }> {
    const body = JSON.stringify({
      jsonrpc: '2.0',
      id: ++this.rpcId,
      method: 'eth_call',
      params: [{ from: this.account.address, to, data }, 'latest'],
    })
    const res = await fetch(this.rpcUrl, { method: 'POST', headers: { 'content-type': 'application/json' }, body })
    const json = (await res.json()) as { result?: Hex; error?: { code: number; message: string; data?: unknown } }
    if (json.error) {
      const revert = revertDataOf(json.error)
      return { revert, error: json.error.message }
    }
    return { result: json.result }
  }

  private async sendTx(tx: {
    to?: Address
    data: Hex
    gas: bigint
    maxFeePerGas?: bigint
    maxPriorityFeePerGas?: bigint
  }): Promise<Hex> {
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
    const wallet = createWalletClient({ account: this.account, transport: http(this.rpcUrl) })
    try {
      const hash = await wallet.sendTransaction({
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
  async sendUserOperation(op: UserOperation): Promise<Hex> {
    if (op.signature === '0x') throw new RpcError(RpcCodes.invalidParams, 'missing signature')
    if (op.maxPriorityFeePerGas > op.maxFeePerGas) {
      throw new RpcError(RpcCodes.invalidParams, 'maxPriorityFeePerGas exceeds maxFeePerGas')
    }
    if (op.verificationGasLimit === 0n || op.preVerificationGas === 0n) {
      throw new RpcError(RpcCodes.invalidParams, 'gas limits missing: call eth_estimateUserOperationGas first')
    }
    const packed = packUserOp(op)
    const hash = (await this.client.readContract({
      address: this.entryPoint,
      abi: entryPointAbi,
      functionName: 'getUserOpHash',
      args: [packed],
    })) as Hex
    const known = this.entries.get(hash)
    if (known && known.status !== 'failed') {
      throw new RpcError(RpcCodes.invalidParams, `UserOperation ${hash} already ${known.status}`)
    }

    let resolveSubmitted!: (h: Hex) => void
    let rejectSubmitted!: (e: unknown) => void
    const submitted = new Promise<Hex>((res, rej) => {
      resolveSubmitted = res
      rejectSubmitted = rej
    })
    // Strictly sequential: simulate on the state left by the previous bundle, submit, wait until it is mined.
    this.queue = this.queue
      .catch(() => {})
      .then(async () => {
        let entry: Entry | undefined
        try {
          const block = await this.client.getBlock({ blockTag: 'latest' })
          const baseFee = block.baseFeePerGas ?? 0n
          if (op.maxFeePerGas < baseFee) {
            throw new RpcError(RpcCodes.invalidParams, `maxFeePerGas ${op.maxFeePerGas} is below the base fee ${baseFee}`)
          }
          const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'handleOps', args: [[packed], this.beneficiary] })
          const sim = await this.rawCall(this.entryPoint, data)
          if (sim.revert !== undefined || sim.error) throw this.validationError(sim.revert, sim.error)
          const estimated = await this.client.estimateGas({ account: this.account.address, to: this.entryPoint, data })
          entry = { op, hash, status: 'submitted' }
          const txHash = await this.sendTx({
            to: this.entryPoint,
            data,
            gas: mul(estimated, 120n) + 20_000n,
            maxFeePerGas: op.maxFeePerGas,
            maxPriorityFeePerGas: op.maxPriorityFeePerGas,
          })
          entry.txHash = txHash
          this.entries.set(hash, entry)
          this.log(`op ${hash} from ${op.sender} nonce ${op.nonce} -> tx ${txHash}`)
          resolveSubmitted(hash)
        } catch (e) {
          rejectSubmitted(e instanceof RpcError ? e : new RpcError(RpcCodes.internal, (e as Error).message))
          return
        }
        await this.track(entry!)
      })
    return submitted
  }

  private validationError(revert: Hex | undefined, message?: string): RpcError {
    if (!revert) return new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `simulation failed: ${message ?? 'unknown'}`)
    let reason = describeRevert(revert)
    try {
      const d = decodeErrorResult({ abi: entryPointAbi, data: revert })
      if (d.errorName === 'FailedOp' || d.errorName === 'FailedOpWithRevert') {
        const aa = String((d.args as readonly unknown[])[1])
        reason = d.errorName === 'FailedOpWithRevert' ? `${aa}: ${describeRevert((d.args as readonly unknown[])[2] as Hex)}` : aa
        return new RpcError(codeForReason(aa), reason, { revertData: revert })
      }
    } catch {
      /* not an EntryPoint error */
    }
    return new RpcError(RpcCodes.rejectedByEntryPointOrAccount, `handleOps simulation reverted: ${reason}`, { revertData: revert })
  }

  private async track(entry: Entry): Promise<void> {
    try {
      const r = await this.client.waitForTransactionReceipt({
        hash: entry.txHash as Hex,
        timeout: this.opts.receiptTimeoutMs,
        pollingInterval: this.opts.pollingIntervalMs,
      })
      entry.blockHash = r.blockHash
      entry.blockNumber = r.blockNumber
      if (r.status !== 'success') {
        entry.status = 'failed'
        entry.error = 'bundle transaction reverted'
        this.log(`bundle ${entry.txHash} reverted`)
        return
      }
      const receipt = this.buildReceipt(entry.hash, r)
      if (!receipt) {
        entry.status = 'failed'
        entry.error = 'no UserOperationEvent in the bundle'
        return
      }
      entry.receipt = receipt
      entry.status = 'included'
      this.log(`op ${entry.hash} included in block ${r.blockNumber} (success=${receipt.success})`)
    } catch (e) {
      entry.status = 'failed'
      entry.error = (e as Error).message
      this.log(`op ${entry.hash}: ${entry.error}`)
    }
  }

  private buildReceipt(userOpHash: Hex, r: TransactionReceipt): UserOperationReceipt | null {
    let ev: { sender: Address; paymaster: Address; nonce: bigint; success: boolean; actualGasCost: bigint; actualGasUsed: bigint } | undefined
    let reason: Hex = '0x'
    for (const log of r.logs) {
      if (!isAddressEqual(log.address, this.entryPoint)) continue
      try {
        const d = decodeEventLog({ abi: entryPointAbi, data: log.data, topics: log.topics })
        const a = d.args as Record<string, unknown>
        if (String(a.userOpHash).toLowerCase() !== userOpHash) continue
        if (d.eventName === 'UserOperationEvent') ev = a as typeof ev
        if (d.eventName === 'UserOperationRevertReason') reason = a.revertReason as Hex
      } catch {
        /* other event */
      }
    }
    if (!ev) return null
    const json = (v: unknown): unknown =>
      JSON.parse(JSON.stringify(v, (_k, x) => (typeof x === 'bigint' ? toHex(x) : x)))
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
      // One op per bundle: every log of the transaction belongs to this op.
      logs: json(r.logs) as unknown[],
      receipt: json(r),
    }
  }

  // ------------------------------------------------------------------------------------------- lookups
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
    const e = this.entries.get(userOpHash)
    if (e?.receipt) return e.receipt
    if (e && e.status === 'submitted') return null
    const r = await this.findOnChain(userOpHash)
    return r ? this.buildReceipt(userOpHash, r) : null
  }

  async getUserOperationByHash(userOpHash: Hex): Promise<unknown> {
    const e = this.entries.get(userOpHash)
    if (e) {
      return {
        userOperation: toRpcUserOp(e.op),
        entryPoint: this.entryPoint,
        transactionHash: e.txHash ?? null,
        blockHash: e.blockHash ?? null,
        blockNumber: e.blockNumber !== undefined ? toHex(e.blockNumber) : null,
      }
    }
    const r = await this.findOnChain(userOpHash)
    if (!r) return null
    const tx = await this.client.getTransaction({ hash: r.transactionHash })
    const d = decodeFunctionData({ abi: entryPointAbi, data: tx.input })
    if (d.functionName !== 'handleOps') return null
    const ops = (d.args as readonly [readonly PackedUserOperation[], Address])[0]
    for (const p of ops) {
      const h = (await this.client.readContract({
        address: this.entryPoint,
        abi: entryPointAbi,
        functionName: 'getUserOpHash',
        args: [p],
      })) as Hex
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
    const maxCost =
      (nominal.verificationGasLimit + nominal.callGasLimit + nominal.paymasterVerificationGasLimit +
        nominal.paymasterPostOpGasLimit + nominal.preVerificationGas) * op.maxFeePerGas

    const packed = packUserOp(op)
    const inner = encodeFunctionData({ abi: gasMeterAbi, functionName: 'measure', args: [packed, maxCost] })
    const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'delegateAndRevert', args: [this.meter, inner] })
    const res = await this.rawCall(this.entryPoint, data)
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
    out.preVerificationGas = this.preVerificationGas(out)
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

  /** Calldata + fixed overhead of a one-op bundle, with 10 % margin (signature length taken from the input). */
  preVerificationGas(op: UserOperation): bigint {
    const sigLen = Math.max(size(op.signature), 65)
    const filled: UserOperation = { ...op, signature: toHex(new Uint8Array(sigLen).fill(1)), preVerificationGas: 1n << 64n }
    const data = encodeFunctionData({
      abi: entryPointAbi,
      functionName: 'handleOps',
      args: [[packUserOp(filled)], this.beneficiary],
    })
    const words = BigInt(Math.ceil((size(data) - 4) / 32))
    return mul(PVG_FIXED + PVG_PER_OP + calldataGas(data) + 4n * words, 110n)
  }

  /** Waits for the internal queue (tests). */
  async idle(): Promise<void> {
    await this.queue.catch(() => {})
  }
}

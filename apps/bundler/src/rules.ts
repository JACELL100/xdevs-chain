// Stateless UserOperation checks: sizes, gas caps, fees, and the preVerificationGas that keeps the operator whole.
import { type Address, type Hex, encodeFunctionData, size, slice, toHex } from 'viem'
import { entryPointAbi } from './abi.ts'
import type { BundlerConfig } from './config.ts'
import { RpcCodes, RpcError } from './errors.ts'
import { type UserOperation, calldataGas, packUserOp } from './userop.ts'

/** Fixed per-bundle overhead the EntryPoint does not meter per op (tx intrinsic gas). Charged to EVERY op. */
export const PVG_FIXED = 21_000n
/** Per-op handleOps overhead outside the metered sections (loop, event, compensation). */
export const PVG_PER_OP = 18_300n

const mul = (v: bigint, pct: bigint) => (v * pct + 99n) / 100n

/**
 * Gas the bundle transaction spends on this op that the EntryPoint does not meter: the calldata of a one-op
 * `handleOps`, the per-op loop overhead AND the full 21,000 intrinsic gas. In a bundle of n ops the intrinsic gas
 * is paid once but charged n times, so `preVerificationGas >= requiredPreVerificationGas(op)` for every op means the
 * beneficiary is paid at least the gas the operator spends (with fees capped as in `bundleFees`).
 */
export function requiredPreVerificationGas(op: UserOperation, beneficiary: Address): bigint {
  const data = encodeFunctionData({ abi: entryPointAbi, functionName: 'handleOps', args: [[packUserOp(op)], beneficiary] })
  const words = BigInt(Math.ceil((size(data) - 4) / 32))
  return PVG_FIXED + PVG_PER_OP + calldataGas(data) + 4n * words
}

/** The preVerificationGas `eth_estimateUserOperationGas` returns: worst-case bytes for the signature and the PVG field, +10 %. */
export function estimatePreVerificationGas(op: UserOperation, beneficiary: Address): bigint {
  const sigLen = Math.max(size(op.signature), 65)
  const filled: UserOperation = { ...op, signature: toHex(new Uint8Array(sigLen).fill(1)), preVerificationGas: 1n << 64n }
  return mul(requiredPreVerificationGas(filled, beneficiary), 110n)
}

/** Total gas an op may use (what `handleOps` must reserve for it). */
export function opGasLimit(op: UserOperation): bigint {
  return (
    op.preVerificationGas +
    op.verificationGasLimit +
    op.callGasLimit +
    (op.paymasterVerificationGasLimit ?? 0n) +
    (op.paymasterPostOpGasLimit ?? 0n)
  )
}

/** The EntryPoint's required prefund (`maxCost`). */
export function opMaxCost(op: UserOperation): bigint {
  return opGasLimit(op) * op.maxFeePerGas
}

/** ABI-encoded size of the PackedUserOperation (what it adds to the bundle calldata). */
export function opByteSize(op: UserOperation): number {
  const p = packUserOp(op)
  const dyn = (h: Hex) => 32 + Math.ceil(size(h) / 32) * 32
  return 32 * 9 + dyn(p.initCode) + dyn(p.callData) + dyn(p.paymasterAndData) + dyn(p.signature)
}

/** True for a real factory (not the EIP-7702 `0x7702…` initCode marker). */
export function isFactory(a: Address | null | undefined): a is Address {
  return !!a && !a.toLowerCase().startsWith('0x7702')
}

/** XdevsPaymaster sponsor (`paymasterData[0:20]`) and whether the op uses open / allowlist mode (20 bytes, no signer). */
export function sponsorOf(op: UserOperation, sponsorPaymaster: Address): { sponsor: Address; openMode: boolean } | null {
  if (!op.paymaster || op.paymaster.toLowerCase() !== sponsorPaymaster.toLowerCase()) return null
  const d = op.paymasterData ?? '0x'
  if (size(d) < 20) return null
  return { sponsor: slice(d, 0, 20) as Address, openMode: size(d) === 20 }
}

type StaticLimits = Pick<BundlerConfig, 'maxOpBytes' | 'maxVerificationGas' | 'maxCallGas' | 'maxBundleGas'>

/** Shape / size / gas-cap checks that need no chain state. */
export function checkStatic(op: UserOperation, cfg: StaticLimits, forEstimate = false): void {
  const bad = (m: string) => new RpcError(RpcCodes.invalidParams, m)
  const bytes = opByteSize(op)
  if (bytes > cfg.maxOpBytes) throw bad(`UserOperation too large: ${bytes} bytes (max ${cfg.maxOpBytes})`)
  if (op.maxPriorityFeePerGas > op.maxFeePerGas) throw bad('maxPriorityFeePerGas exceeds maxFeePerGas')
  if (op.verificationGasLimit > cfg.maxVerificationGas) throw bad(`verificationGasLimit above ${cfg.maxVerificationGas}`)
  if ((op.paymasterVerificationGasLimit ?? 0n) > cfg.maxVerificationGas) {
    throw bad(`paymasterVerificationGasLimit above ${cfg.maxVerificationGas}`)
  }
  if (op.callGasLimit > cfg.maxCallGas) throw bad(`callGasLimit above ${cfg.maxCallGas}`)
  if (forEstimate) return
  if (op.signature === '0x') throw bad('missing signature')
  if (op.verificationGasLimit === 0n || op.preVerificationGas === 0n) {
    throw bad('gas limits missing: call eth_estimateUserOperationGas first')
  }
  if (op.paymaster && (op.paymasterVerificationGasLimit ?? 0n) === 0n) throw bad('paymasterVerificationGasLimit missing')
  if (size(op.callData) > 0 && op.callGasLimit < 9_100n) throw bad('callGasLimit below the 9,100 gas of a CALL with value')
  const total = opGasLimit(op)
  if (total > cfg.maxBundleGas) throw bad(`total gas ${total} does not fit in a bundle (max ${cfg.maxBundleGas})`)
}

/** Fee floor: the op must pay the current base fee (with margin) plus the minimum tip. */
export function checkFees(op: UserOperation, baseFee: bigint, cfg: Pick<BundlerConfig, 'minPriorityFeePerGas' | 'baseFeeMarginPct'>): void {
  if (op.maxPriorityFeePerGas < cfg.minPriorityFeePerGas) {
    throw new RpcError(RpcCodes.invalidParams, `maxPriorityFeePerGas below the bundler minimum ${cfg.minPriorityFeePerGas} wei`)
  }
  const floor = (baseFee * cfg.baseFeeMarginPct + 99n) / 100n + cfg.minPriorityFeePerGas
  if (op.maxFeePerGas < floor) {
    throw new RpcError(
      RpcCodes.invalidParams,
      `maxFeePerGas ${op.maxFeePerGas} too low: needs >= ${floor} (base fee ${baseFee} + minimum tip ${cfg.minPriorityFeePerGas})`,
    )
  }
}

/** The op must pay for the gas it adds to the bundle beyond what the EntryPoint meters (no operator loss). */
export function checkPreVerificationGas(op: UserOperation, beneficiary: Address): void {
  const need = requiredPreVerificationGas(op, beneficiary)
  if (op.preVerificationGas < need) {
    throw new RpcError(
      RpcCodes.invalidParams,
      `preVerificationGas ${op.preVerificationGas} below ${need} (calldata + bundle overhead): use eth_estimateUserOperationGas`,
    )
  }
}

/**
 * Bundle transaction fees: the minimum caps of its ops. Each op is then charged
 * `min(maxFee_i, tip_i + baseFee) >= min(txMaxFee, txTip + baseFee)` = the price the operator pays per gas.
 */
export function bundleFees(ops: UserOperation[]): { maxFeePerGas: bigint; maxPriorityFeePerGas: bigint } {
  let maxFeePerGas = ops[0]!.maxFeePerGas
  let maxPriorityFeePerGas = ops[0]!.maxPriorityFeePerGas
  for (const o of ops) {
    if (o.maxFeePerGas < maxFeePerGas) maxFeePerGas = o.maxFeePerGas
    if (o.maxPriorityFeePerGas < maxPriorityFeePerGas) maxPriorityFeePerGas = o.maxPriorityFeePerGas
  }
  if (maxPriorityFeePerGas > maxFeePerGas) maxPriorityFeePerGas = maxFeePerGas
  return { maxFeePerGas, maxPriorityFeePerGas }
}

/** Effective tip the op pays at `baseFee` (for ordering). */
export function effectiveTip(op: UserOperation, baseFee: bigint): bigint {
  const room = op.maxFeePerGas - baseFee
  return room < op.maxPriorityFeePerGas ? room : op.maxPriorityFeePerGas
}

/** Unpacks ERC-4337 validationData: (aggregator / sig-failed flag, validUntil, validAfter). */
export function parseValidationData(v: bigint): { aggregator: Address; sigFailed: boolean; validUntil: number; validAfter: number } {
  const agg = v & ((1n << 160n) - 1n)
  const validUntil = Number((v >> 160n) & ((1n << 48n) - 1n))
  const validAfter = Number((v >> 208n) & ((1n << 48n) - 1n))
  return {
    aggregator: toHex(agg, { size: 20 }) as Address,
    sigFailed: agg === 1n,
    validUntil: validUntil === 0 ? 0 : validUntil,
    validAfter,
  }
}

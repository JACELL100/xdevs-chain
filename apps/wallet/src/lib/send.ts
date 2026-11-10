// UserOperation flow: nonce -> fees -> bundler gas estimate (dummy signature) -> owner signature -> bundler -> receipt.
import { type Address, type Hex, type PublicClient, decodeEventLog, decodeErrorResult } from 'viem'
import { accountAbi, entryPointAbi } from './abi.ts'
import { ENTRY_POINT, PAYMASTER } from './account.ts'
import type { OwnerSigner } from './signers.ts'
import { type RpcUserOperation, type UserOperation, fromRpcUserOp, getUserOpHash, toRpcUserOp } from './userop.ts'

export class BundlerError extends Error {
  readonly code: number
  readonly data?: unknown
  constructor(code: number, message: string, data?: unknown) {
    super(message)
    this.code = code
    this.data = data
  }
}

let rpcId = 0
export async function bundlerRpc<T>(url: string, method: string, params: unknown[]): Promise<T> {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcId, method, params }),
  })
  const j = (await res.json()) as { result?: T; error?: { code: number; message: string; data?: unknown } }
  if (j.error) throw new BundlerError(j.error.code, j.error.message, j.error.data)
  return j.result as T
}

export type UserOpReceipt = {
  userOpHash: Hex
  sender: Address
  success: boolean
  actualGasCost: Hex
  paymaster: Address
  reason: Hex
  logs: { address: Address; topics: Hex[]; data: Hex }[]
  receipt: { transactionHash: Hex; blockNumber: Hex }
}

export type SendUserOpArgs = {
  client: PublicClient
  bundlerUrl: string
  chainId: number
  sender: Address
  callData: Hex
  signer: OwnerSigner
  /** Deploy the account in this op (counterfactual factory accounts). */
  factory?: { address: Address; data: Hex } | null
  /** Paymaster sponsor (XdevsPaymaster open / allowlist mode: paymasterData = sponsor address). */
  sponsor?: Address | null
  onStatus?: (s: string) => void
  timeoutMs?: number
}

export type SentUserOp = {
  userOpHash: Hex
  receipt: UserOpReceipt
  /** Vault action ids queued by this op (VaultActionQueued events of the sender). */
  queuedIds: bigint[]
}

export async function buildUserOp(a: SendUserOpArgs): Promise<UserOperation> {
  const code = await a.client.getCode({ address: a.sender })
  const deployed = !!code && code !== '0x'
  const nonce = deployed || !a.factory
    ? await a.client.readContract({ address: ENTRY_POINT, abi: entryPointAbi, functionName: 'getNonce', args: [a.sender, 0n] })
    : 0n
  const block = await a.client.getBlock({ blockTag: 'latest' })
  const baseFee = block.baseFeePerGas ?? 1_000_000_000n
  let priority = 1_000_000n
  try {
    priority = await a.client.estimateMaxPriorityFeePerGas()
  } catch {
    /* node without eth_maxPriorityFeePerGas */
  }
  return {
    sender: a.sender,
    nonce,
    factory: !deployed && a.factory ? a.factory.address : null,
    factoryData: !deployed && a.factory ? a.factory.data : null,
    callData: a.callData,
    callGasLimit: 0n,
    verificationGasLimit: 0n,
    preVerificationGas: 0n,
    maxFeePerGas: baseFee * 2n + priority,
    maxPriorityFeePerGas: priority,
    paymaster: a.sponsor ? PAYMASTER : null,
    paymasterData: a.sponsor ?? null,
    paymasterVerificationGasLimit: null,
    paymasterPostOpGasLimit: null,
    signature: a.signer.dummySignature(),
  }
}

export async function estimateUserOp(a: SendUserOpArgs, op: UserOperation): Promise<UserOperation> {
  const est = await bundlerRpc<Partial<RpcUserOperation>>(a.bundlerUrl, 'eth_estimateUserOperationGas', [toRpcUserOp(op), ENTRY_POINT])
  return fromRpcUserOp({ ...toRpcUserOp(op), ...est })
}

export async function waitForUserOpReceipt(bundlerUrl: string, hash: Hex, timeoutMs = 90_000): Promise<UserOpReceipt> {
  const until = Date.now() + timeoutMs
  while (Date.now() < until) {
    const r = await bundlerRpc<UserOpReceipt | null>(bundlerUrl, 'eth_getUserOperationReceipt', [hash])
    if (r) return r
    await new Promise((res) => setTimeout(res, 500))
  }
  throw new Error(`UserOperation ${hash} not included after ${timeoutMs / 1000}s`)
}

/** Decodes the account's vault events in a receipt (queued action ids). */
export function queuedIdsOf(receipt: UserOpReceipt, account: Address): bigint[] {
  const ids: bigint[] = []
  for (const log of receipt.logs) {
    if (log.address.toLowerCase() !== account.toLowerCase()) continue
    try {
      const d = decodeEventLog({ abi: accountAbi, data: log.data, topics: log.topics as [Hex, ...Hex[]] })
      if (d.eventName === 'VaultActionQueued') ids.push((d.args as { id: bigint }).id)
    } catch {
      /* not a vault event */
    }
  }
  return ids
}

/** Human-readable revert reason of a failed op's execution (`UserOperationRevertReason`). */
export function describeOpRevert(reason: Hex): string {
  if (!reason || reason === '0x') return 'execution reverted'
  try {
    const d = decodeErrorResult({ abi: accountAbi, data: reason })
    return `${d.errorName}(${((d.args ?? []) as readonly unknown[]).map(String).join(', ')})`
  } catch {
    return `execution reverted (${reason.slice(0, 10)})`
  }
}

/** Builds, estimates, signs (one authenticator prompt) and submits a UserOperation, then waits for inclusion. */
export async function sendUserOp(a: SendUserOpArgs): Promise<SentUserOp> {
  const status = a.onStatus ?? (() => {})
  status('Preparing UserOperation…')
  let op = await buildUserOp(a)
  status('Estimating gas with the bundler…')
  op = await estimateUserOp(a, op)
  const hash = getUserOpHash(op, a.chainId)
  status(a.signer.kind === 'passkey' ? 'Confirm with your passkey…' : 'Signing…')
  op.signature = await a.signer.signHash(hash)
  status('Submitting to the bundler…')
  const userOpHash = await bundlerRpc<Hex>(a.bundlerUrl, 'eth_sendUserOperation', [toRpcUserOp(op), ENTRY_POINT])
  status('Waiting for inclusion…')
  const receipt = await waitForUserOpReceipt(a.bundlerUrl, userOpHash, a.timeoutMs)
  if (!receipt.success) throw new Error(`UserOperation included but reverted: ${describeOpRevert(receipt.reason)}`)
  return { userOpHash, receipt, queuedIds: queuedIdsOf(receipt, a.sender) }
}

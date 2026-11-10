// Copied from apps/bundler/src/userop.ts (kept in sync by the wallet's userop test against the EntryPoint).
// ERC-4337 v0.9 UserOperation encoding: JSON-RPC (unpacked) <-> PackedUserOperation, and the EIP-712 userOpHash.
// Mirrors contracts/lib/account-abstraction (UserOperationLib, Helpers.paymasterDataKeccak, EntryPoint.getUserOpHash).
import {
  type Address,
  type Hex,
  concat,
  encodeAbiParameters,
  getAddress,
  keccak256,
  pad,
  size,
  slice,
  toHex,
} from 'viem'

export const ENTRY_POINT_V09: Address = '0x433709009B8330FDa32311DF1C2AFA402eD8D009'

/** v0.9 paymaster-signature magic `keccak("PaymasterSignature")[:8]`. */
export const PAYMASTER_SIG_MAGIC: Hex = '0x22e325a297439656'
const PAYMASTER_DATA_OFFSET = 52
const PAYMASTER_SUFFIX_LEN = 10 // uint16 length + 8-byte magic

/** Unpacked UserOperation (the ERC-4337 v0.7+ JSON-RPC shape), with bigint quantities. */
export type UserOperation = {
  sender: Address
  nonce: bigint
  factory?: Address | null
  factoryData?: Hex | null
  callData: Hex
  callGasLimit: bigint
  verificationGasLimit: bigint
  preVerificationGas: bigint
  maxFeePerGas: bigint
  maxPriorityFeePerGas: bigint
  paymaster?: Address | null
  paymasterVerificationGasLimit?: bigint | null
  paymasterPostOpGasLimit?: bigint | null
  paymasterData?: Hex | null
  signature: Hex
}

/** Same as `UserOperation` with hex quantities, as sent over JSON-RPC. */
export type RpcUserOperation = {
  sender: Address
  nonce: Hex
  factory?: Address | null
  factoryData?: Hex | null
  callData: Hex
  callGasLimit: Hex
  verificationGasLimit: Hex
  preVerificationGas: Hex
  maxFeePerGas: Hex
  maxPriorityFeePerGas: Hex
  paymaster?: Address | null
  paymasterVerificationGasLimit?: Hex | null
  paymasterPostOpGasLimit?: Hex | null
  paymasterData?: Hex | null
  signature: Hex
}

export type PackedUserOperation = {
  sender: Address
  nonce: bigint
  initCode: Hex
  callData: Hex
  accountGasLimits: Hex
  preVerificationGas: bigint
  gasFees: Hex
  paymasterAndData: Hex
  signature: Hex
}

const U128 = (1n << 128n) - 1n

function packUints(high: bigint, low: bigint): Hex {
  if (high < 0n || high > U128 || low < 0n || low > U128) throw new Error('gas value does not fit in uint128')
  return pad(toHex((high << 128n) | low), { size: 32 })
}

function unpackUints(packed: Hex): [bigint, bigint] {
  const v = BigInt(packed)
  return [v >> 128n, v & U128]
}

export function getInitCode(op: Pick<UserOperation, 'factory' | 'factoryData'>): Hex {
  if (!op.factory) return '0x'
  return concat([op.factory, op.factoryData ?? '0x'])
}

export function getPaymasterAndData(op: UserOperation): Hex {
  if (!op.paymaster) return '0x'
  return concat([
    op.paymaster,
    pad(toHex(op.paymasterVerificationGasLimit ?? 0n), { size: 16 }),
    pad(toHex(op.paymasterPostOpGasLimit ?? 0n), { size: 16 }),
    op.paymasterData ?? '0x',
  ])
}

export function packUserOp(op: UserOperation): PackedUserOperation {
  return {
    sender: op.sender,
    nonce: op.nonce,
    initCode: getInitCode(op),
    callData: op.callData,
    accountGasLimits: packUints(op.verificationGasLimit, op.callGasLimit),
    preVerificationGas: op.preVerificationGas,
    gasFees: packUints(op.maxPriorityFeePerGas, op.maxFeePerGas),
    paymasterAndData: getPaymasterAndData(op),
    signature: op.signature,
  }
}

export function unpackUserOp(p: PackedUserOperation): UserOperation {
  const [verificationGasLimit, callGasLimit] = unpackUints(p.accountGasLimits)
  const [maxPriorityFeePerGas, maxFeePerGas] = unpackUints(p.gasFees)
  const op: UserOperation = {
    sender: getAddress(p.sender),
    nonce: p.nonce,
    factory: null,
    factoryData: null,
    callData: p.callData,
    callGasLimit,
    verificationGasLimit,
    preVerificationGas: p.preVerificationGas,
    maxFeePerGas,
    maxPriorityFeePerGas,
    paymaster: null,
    paymasterVerificationGasLimit: null,
    paymasterPostOpGasLimit: null,
    paymasterData: null,
    signature: p.signature,
  }
  if (size(p.initCode) >= 20) {
    op.factory = getAddress(slice(p.initCode, 0, 20))
    op.factoryData = size(p.initCode) > 20 ? slice(p.initCode, 20) : '0x'
  }
  if (size(p.paymasterAndData) >= PAYMASTER_DATA_OFFSET) {
    op.paymaster = getAddress(slice(p.paymasterAndData, 0, 20))
    op.paymasterVerificationGasLimit = BigInt(slice(p.paymasterAndData, 20, 36))
    op.paymasterPostOpGasLimit = BigInt(slice(p.paymasterAndData, 36, 52))
    op.paymasterData = size(p.paymasterAndData) > 52 ? slice(p.paymasterAndData, 52) : '0x'
  }
  return op
}

const q = (v: bigint): Hex => toHex(v)
const optQ = (v: bigint | null | undefined): Hex | null => (v == null ? null : toHex(v))
const big = (v: Hex | string | number | bigint | null | undefined, field: string): bigint => {
  if (v === null || v === undefined || v === '') throw new Error(`missing field ${field}`)
  try {
    return BigInt(v)
  } catch {
    throw new Error(`invalid quantity in field ${field}`)
  }
}
const optBig = (v: Hex | null | undefined, field: string): bigint | null => (v == null ? null : big(v, field))

export function toRpcUserOp(op: UserOperation): RpcUserOperation {
  return {
    sender: op.sender,
    nonce: q(op.nonce),
    factory: op.factory ?? null,
    factoryData: op.factory ? (op.factoryData ?? '0x') : null,
    callData: op.callData,
    callGasLimit: q(op.callGasLimit),
    verificationGasLimit: q(op.verificationGasLimit),
    preVerificationGas: q(op.preVerificationGas),
    maxFeePerGas: q(op.maxFeePerGas),
    maxPriorityFeePerGas: q(op.maxPriorityFeePerGas),
    paymaster: op.paymaster ?? null,
    paymasterVerificationGasLimit: op.paymaster ? optQ(op.paymasterVerificationGasLimit) : null,
    paymasterPostOpGasLimit: op.paymaster ? optQ(op.paymasterPostOpGasLimit) : null,
    paymasterData: op.paymaster ? (op.paymasterData ?? '0x') : null,
    signature: op.signature,
  }
}

/** Parses an RPC UserOperation. Gas fields missing in `partial` mode default to 0 (for estimation). */
export function fromRpcUserOp(raw: Partial<RpcUserOperation>, partial = false): UserOperation {
  if (!raw || typeof raw !== 'object') throw new Error('userOp must be an object')
  const gas = (v: Hex | null | undefined, field: string) => (partial && v == null ? 0n : big(v, field))
  const isHex = (v: unknown): v is Hex => typeof v === 'string' && /^0x([0-9a-fA-F]{2})*$/.test(v)
  for (const f of ['callData', 'signature'] as const) {
    const v = raw[f]
    if (v !== undefined && !isHex(v)) throw new Error(`field ${f} must be 0x-prefixed even-length hex`)
  }
  if (!raw.sender) throw new Error('missing field sender')
  return {
    sender: getAddress(raw.sender),
    nonce: big(raw.nonce, 'nonce'),
    factory: raw.factory ? getAddress(raw.factory) : null,
    factoryData: raw.factory ? (raw.factoryData ?? '0x') : null,
    callData: raw.callData ?? '0x',
    callGasLimit: gas(raw.callGasLimit, 'callGasLimit'),
    verificationGasLimit: gas(raw.verificationGasLimit, 'verificationGasLimit'),
    preVerificationGas: gas(raw.preVerificationGas, 'preVerificationGas'),
    maxFeePerGas: gas(raw.maxFeePerGas, 'maxFeePerGas'),
    maxPriorityFeePerGas: gas(raw.maxPriorityFeePerGas, 'maxPriorityFeePerGas'),
    paymaster: raw.paymaster ? getAddress(raw.paymaster) : null,
    paymasterVerificationGasLimit: raw.paymaster
      ? (optBig(raw.paymasterVerificationGasLimit, 'paymasterVerificationGasLimit') ?? 0n)
      : null,
    paymasterPostOpGasLimit: raw.paymaster
      ? (optBig(raw.paymasterPostOpGasLimit, 'paymasterPostOpGasLimit') ?? 0n)
      : null,
    paymasterData: raw.paymaster ? (raw.paymasterData ?? '0x') : null,
    signature: raw.signature ?? '0x',
  }
}

/** `Helpers.paymasterDataKeccak`: the v0.9 paymaster signature (if any) is excluded, its magic is kept. */
export function paymasterDataKeccak(paymasterAndData: Hex): Hex {
  const len = size(paymasterAndData)
  if (len >= PAYMASTER_DATA_OFFSET + PAYMASTER_SUFFIX_LEN && slice(paymasterAndData, len - 8) === PAYMASTER_SIG_MAGIC) {
    const sigLen = Number(BigInt(slice(paymasterAndData, len - PAYMASTER_SUFFIX_LEN, len - 8)))
    if (sigLen > len - PAYMASTER_DATA_OFFSET - PAYMASTER_SUFFIX_LEN) throw new Error('invalid paymaster signature length')
    return keccak256(concat([slice(paymasterAndData, 0, len - (sigLen + PAYMASTER_SUFFIX_LEN)), PAYMASTER_SIG_MAGIC]))
  }
  return keccak256(paymasterAndData)
}

export const PACKED_USEROP_TYPEHASH = keccak256(
  toHex(
    'PackedUserOperation(address sender,uint256 nonce,bytes initCode,bytes callData,bytes32 accountGasLimits,uint256 preVerificationGas,bytes32 gasFees,bytes paymasterAndData)',
  ),
)

/**
 * EIP-712 userOpHash of EntryPoint v0.9 (domain `ERC4337` / `1`). Not valid for EIP-7702 initCode (`0x7702…`), whose
 * hash depends on the sender's on-chain delegate: ask the EntryPoint (`getUserOpHash`) for those.
 */
export function getUserOpHash(op: UserOperation, chainId: number | bigint, entryPoint: Address = ENTRY_POINT_V09): Hex {
  const p = packUserOp(op)
  if (p.initCode.toLowerCase().startsWith('0x7702')) throw new Error('EIP-7702 initCode: use EntryPoint.getUserOpHash')
  return hashPacked(p, chainId, entryPoint)
}

/** Manual EIP-712 hashing (identical to `UserOperationLib.encode` + OZ `EIP712._hashTypedDataV4`). */
export function hashPacked(p: PackedUserOperation, chainId: number | bigint, entryPoint: Address): Hex {
  const structHash = keccak256(
    encodeAbiParameters(
      [
        { type: 'bytes32' },
        { type: 'address' },
        { type: 'uint256' },
        { type: 'bytes32' },
        { type: 'bytes32' },
        { type: 'bytes32' },
        { type: 'uint256' },
        { type: 'bytes32' },
        { type: 'bytes32' },
      ],
      [
        PACKED_USEROP_TYPEHASH,
        p.sender,
        p.nonce,
        keccak256(p.initCode),
        keccak256(p.callData),
        p.accountGasLimits,
        p.preVerificationGas,
        p.gasFees,
        paymasterDataKeccak(p.paymasterAndData),
      ],
    ),
  )
  const domainSeparator = keccak256(
    encodeAbiParameters(
      [{ type: 'bytes32' }, { type: 'bytes32' }, { type: 'bytes32' }, { type: 'uint256' }, { type: 'address' }],
      [
        keccak256(toHex('EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)')),
        keccak256(toHex('ERC4337')),
        keccak256(toHex('1')),
        BigInt(chainId),
        entryPoint,
      ],
    ),
  )
  return keccak256(concat(['0x1901', domainSeparator, structHash]))
}

/** Calldata gas of a byte string: 4 per zero byte, 16 per non-zero byte (EIP-2028). */
export function calldataGas(data: Hex): bigint {
  let gas = 0n
  for (let i = 2; i < data.length; i += 2) gas += data.slice(i, i + 2) === '00' ? 4n : 16n
  return gas
}

// xDEVS smart-account helpers: init params, counterfactual address, call builders, on-chain state.
import {
  type Address,
  type Hex,
  type PublicClient,
  encodeFunctionData,
  getAddress,
  pad,
  toHex,
  zeroAddress,
} from 'viem'
import { accountAbi, factoryAbi } from './abi.ts'
import type { SignerStruct } from './signers.ts'

export const SMART_ACCOUNT_IMPL: Address = '0x0000000000000000000000000000000000001005'
export const SMART_ACCOUNT_FACTORY: Address = '0x0000000000000000000000000000000000001006'
export const PAYMASTER: Address = '0x0000000000000000000000000000000000001007'
export const XNS_REGISTRY: Address = '0x0000000000000000000000000000000000001004'
export const PERMIT2_EXPIRING: Address = '0x0000000000000000000000000000000000001008'
export const ENTRY_POINT: Address = '0x433709009B8330FDa32311DF1C2AFA402eD8D009'
/** XIP-0001 lock-word slot: keccak256("xdevs.account.eoaLock") - 1 (raw, not ERC-7201 masked). */
export const EOA_LOCK_SLOT: Hex = '0xfd81d7eef3094fecca95e5e53fe72709c7315b39f65e414992ead9c855e841ce'
/** EIP-7702 delegation designator of an EOA delegated to the xDEVS Smart Account. */
export const DESIGNATOR = `0xef0100${SMART_ACCOUNT_IMPL.slice(2).toLowerCase()}` as const

export const VAULT_EXECUTION_WINDOW = 14n * 86_400n
export const MIN_VAULT_DELAY = 3_600n
export const MAX_VAULT_DELAY = 7n * 86_400n
export const UNLIMITED_APPROVAL = (1n << 160n) - 1n

export type VaultPolicy = {
  enabled: boolean
  blockUnlimitedApprovals: boolean
  allowInstantSignatures: boolean
  delay: bigint
  nativeThreshold: bigint
  dailyLimit: bigint
  dailyGasBudget: bigint
}

export type TokenLimit = { token: Address; perTx: bigint; daily: bigint }

export type InitParams = {
  signers: SignerStruct[]
  guardians: Address[]
  guardianThreshold: number
  recoveryDelay: bigint
  policy: VaultPolicy
  whitelist: Address[]
  tokenLimits: TokenLimit[]
}

export type Call = { target: Address; value: bigint; data: Hex }

export const VAULT_OFF: VaultPolicy = {
  enabled: false,
  blockUnlimitedApprovals: false,
  allowInstantSignatures: false,
  delay: 0n,
  nativeThreshold: 0n,
  dailyLimit: 0n,
  dailyGasBudget: 0n,
}

/** Recommended wallet defaults (smart-accounts.md §11), scaled for a testnet balance. */
export function recommendedPolicy(o: Partial<VaultPolicy> = {}): VaultPolicy {
  return {
    enabled: true,
    blockUnlimitedApprovals: true,
    allowInstantSignatures: false,
    delay: 24n * 3_600n,
    nativeThreshold: 10n * 10n ** 18n,
    dailyLimit: 50n * 10n ** 18n,
    dailyGasBudget: 10n ** 18n,
    ...o,
  }
}

export function buildInitParams(o: {
  signers: SignerStruct[]
  guardians?: Address[]
  guardianThreshold?: number
  recoveryDelay?: bigint
  policy?: VaultPolicy
  whitelist?: Address[]
  tokenLimits?: TokenLimit[]
}): InitParams {
  const guardians = (o.guardians ?? []).map((g) => getAddress(g))
  const threshold = guardians.length === 0 ? 0 : (o.guardianThreshold ?? Math.floor(guardians.length / 2) + 1)
  return {
    signers: o.signers,
    guardians,
    guardianThreshold: threshold,
    recoveryDelay: o.recoveryDelay ?? 0n,
    policy: o.policy ?? VAULT_OFF,
    whitelist: (o.whitelist ?? []).map((a) => getAddress(a)),
    tokenLimits: o.tokenLimits ?? [],
  }
}

export function saltFrom(n: bigint | number): Hex {
  return pad(toHex(BigInt(n)), { size: 32 })
}

/** Counterfactual CREATE2 address (asks the factory predeploy, which commits to the full config). */
export async function counterfactualAddress(client: PublicClient, params: InitParams, salt: Hex): Promise<Address> {
  return client.readContract({
    address: SMART_ACCOUNT_FACTORY,
    abi: factoryAbi,
    functionName: 'getAddressWithConfig',
    args: [params, salt],
  })
}

export function factoryData(params: InitParams, salt: Hex): Hex {
  return encodeFunctionData({ abi: factoryAbi, functionName: 'createAccountWithConfig', args: [params, salt] })
}

// ------------------------------------------------------------------------------------------- call builders
export function executeCallData(c: Call): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'execute', args: [c.target, c.value, c.data] })
}

export function executeBatchCallData(calls: Call[]): Hex {
  return encodeFunctionData({ abi: accountAbi, functionName: 'executeBatch', args: [calls] })
}

/** UserOp callData for a list of calls (single → execute, several → executeBatch). */
export function callsToCallData(calls: Call[]): Hex {
  if (calls.length === 1) return executeCallData(calls[0])
  return executeBatchCallData(calls)
}

// ------------------------------------------------------------------------------------------- state
export type EoaLock = {
  status: 'unlocked' | 'locked' | 'sealed'
  word: bigint
  /** True when the chain enforces XIP-0001 (from `xdevs_getEoaLock`); null if unknown (not an xDEVS node). */
  protocolActive: boolean | null
  delegatedTo: Address | null
  source: 'node' | 'storage'
}

export type AccountState = {
  address: Address
  deployed: boolean
  /** EIP-7702 EOA delegated to the xDEVS Smart Account. */
  is7702: boolean
  balance: bigint
  initialized: boolean
  policy: VaultPolicy
  whitelist: Address[]
  spentToday: bigint
  guardians: Address[]
  guardianThreshold: number
  recoveryDelay: bigint
  frozen: boolean
  frozenUntil: bigint
  freezeDuration: bigint
  nextFreezeAllowedAt: bigint
  lastActionId: bigint
  signers: SignerStruct[]
  eoaKeyActive: boolean
  eoaLock: EoaLock | null
}

const LOCK_STATUS = ['unlocked', 'locked', 'sealed'] as const

/** XIP-0001 lock state: `xdevs_getEoaLock` on an xDEVS node, else the raw storage word. */
export async function readEoaLock(client: PublicClient, address: Address): Promise<EoaLock> {
  try {
    const r = (await client.request({
      method: 'xdevs_getEoaLock' as never,
      params: [address, 'latest'] as never,
    })) as { active: boolean; status: EoaLock['status']; word: Hex; delegatedTo: Address | null }
    return { status: r.status, word: BigInt(r.word), protocolActive: r.active, delegatedTo: r.delegatedTo, source: 'node' }
  } catch {
    const [word, code] = await Promise.all([
      client.getStorageAt({ address, slot: EOA_LOCK_SLOT }),
      client.getCode({ address }),
    ])
    const w = BigInt(word ?? '0x0')
    const delegated = code?.toLowerCase().startsWith('0xef0100') ? getAddress(`0x${code.slice(8, 48)}`) : null
    return { status: LOCK_STATUS[Number(w)] ?? 'unlocked', word: w, protocolActive: null, delegatedTo: delegated, source: 'storage' }
  }
}

export async function readAccountState(client: PublicClient, address: Address): Promise<AccountState> {
  const [code, balance] = await Promise.all([client.getCode({ address }), client.getBalance({ address })])
  const deployed = !!code && code !== '0x'
  const is7702 = !!code && code.toLowerCase() === DESIGNATOR
  const base: AccountState = {
    address,
    deployed,
    is7702,
    balance,
    initialized: false,
    policy: VAULT_OFF,
    whitelist: [],
    spentToday: 0n,
    guardians: [],
    guardianThreshold: 0,
    recoveryDelay: 0n,
    frozen: false,
    frozenUntil: 0n,
    freezeDuration: 0n,
    nextFreezeAllowedAt: 0n,
    lastActionId: 0n,
    signers: [],
    eoaKeyActive: true,
    eoaLock: null,
  }
  if (!deployed) return base
  const read = <T,>(functionName: string, args: unknown[] = []) =>
    client.readContract({ address, abi: accountAbi, functionName: functionName as never, args: args as never }) as Promise<T>
  const [initialized, policy, whitelist, spentToday, g, recoveryDelay, frozen, fs, lastActionId, signers, eoaKeyActive] =
    await Promise.all([
      read<boolean>('isInitialized'),
      read<VaultPolicy>('vaultPolicy'),
      read<Address[]>('getWhitelist'),
      read<bigint>('spentToday'),
      read<readonly [Address[], number]>('getGuardians'),
      read<bigint>('recoveryDelay'),
      read<boolean>('isFrozen'),
      read<readonly [bigint, bigint, bigint]>('freezeState'),
      read<bigint>('lastActionId'),
      read<SignerStruct[]>('getSigners'),
      read<boolean>('isEoaKeyActive'),
    ])
  return {
    ...base,
    initialized,
    policy,
    whitelist: [...whitelist],
    spentToday,
    guardians: [...g[0]],
    guardianThreshold: Number(g[1]),
    recoveryDelay,
    frozen,
    frozenUntil: fs[0],
    freezeDuration: fs[1],
    nextFreezeAllowedAt: fs[2],
    lastActionId,
    signers: signers.map((s) => ({ kind: Number(s.kind), x: s.x, y: s.y })),
    eoaKeyActive,
    eoaLock: is7702 ? await readEoaLock(client, address) : null,
  }
}

export const isZero = (a: Address | null | undefined) => !a || a.toLowerCase() === zeroAddress

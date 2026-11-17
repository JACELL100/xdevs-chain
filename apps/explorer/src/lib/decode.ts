// Client-side calldata and log decoding for the predeploys and token standards the explorer knows. Pure (viem only),
// unit-tested. Used for the RPC-only views and for the "Input" panel of a transaction in every mode.

import { decodeEventLog, decodeFunctionData, parseAbi, type Abi, type Hex } from 'viem'
import { predeploys } from '@xdevs/sdk'

export type Standard = 'erc20' | 'xdrc20' | 'wxds' | 'staking' | 'governance' | 'xns' | 'vault' | 'account' | 'permit2'

const erc20 = parseAbi([
  'function transfer(address to, uint256 value) returns (bool)',
  'function approve(address spender, uint256 value) returns (bool)',
  'function transferFrom(address from, address to, uint256 value) returns (bool)',
  'event Transfer(address indexed from, address indexed to, uint256 value)',
  'event Approval(address indexed owner, address indexed spender, uint256 value)',
])

const xdrc20 = parseAbi([
  'function approve(address spender, uint256 value, uint64 expiry) returns (bool)',
  'function revokeAll()',
  'event ApprovalWithExpiry(address indexed owner, address indexed spender, uint256 value, uint64 expiry)',
  'event AllApprovalsRevoked(address indexed owner, uint32 newEpoch)',
])

const wxds = parseAbi([
  'function deposit() payable',
  'function withdraw(uint256 wad)',
  'event Deposit(address indexed dst, uint256 wad)',
  'event Withdrawal(address indexed src, uint256 wad)',
])

const staking = parseAbi([
  'function register(bytes32 consensusKey, uint16 commissionBps) payable',
  'function delegate(address validator) payable',
  'function undelegate(address validator, uint256 amount)',
  'function withdrawUnbonded()',
  'function claimRewards(address validator)',
  'function unjail()',
  'function setCommission(uint16 newCommissionBps)',
  'event ValidatorRegistered(address indexed operator, bytes32 consensusKey, uint256 selfStake)',
  'event Delegated(address indexed delegator, address indexed validator, uint256 amount)',
  'event Slashed(address indexed validator, uint256 amount, uint8 reason)',
  'event EpochTransition(uint256 indexed epoch, bytes32 validatorSetHash)',
])

const governance = parseAbi([
  'function propose(address[] targets, uint256[] values, bytes[] calldatas, string description) returns (uint256 id)',
  'function castVote(uint256 id, uint8 support) returns (uint256 weight)',
  'function queue(uint256 id)',
  'function execute(uint256 id) payable',
  'function cancel(uint256 id)',
])

const xns = parseAbi([
  'function register(string label, address owner, uint256 duration) payable returns (bytes32 node, uint256 expires)',
  'function renew(string label, uint256 duration) payable returns (uint256 expires)',
  'function setName(string name) returns (bytes32)',
  'function claim(address owner) returns (bytes32)',
])

const permit2 = parseAbi([
  'function approve(address token, address spender, uint160 amount, uint48 expiration)',
  'function lockdown((address token, address spender)[] approvals)',
  'function revokeAll()',
])

/** Guardian Vault (on every xDEVS smart account) and the account's own entry points. */
export const vaultAbi = parseAbi([
  'function cancel(uint256 id)',
  'function execute(uint256 id)',
  'function setPolicy(uint256 nativeThreshold, uint64 delaySeconds, address[] whitelist)',
  'event VaultActionQueued(uint256 indexed id, address target, uint256 value, bytes data, uint64 executeAfter)',
  'event VaultActionCancelled(uint256 indexed id, address by)',
  'event VaultActionExecuted(uint256 indexed id)',
])

const account = parseAbi([
  'function execute(address target, uint256 value, bytes data) returns (bytes)',
  'function executeBatch((address target, uint256 value, bytes data)[] calls) returns (uint256)',
  'function freeze()',
  'function unfreeze()',
  'event AccountFrozen(uint64 frozenUntil)',
  'event AccountUnfrozen()',
  'event EoaLockUpdated(uint256 state)',
  'event EoaKeyRevoked()',
])

const lower = (a: string) => a.toLowerCase()
const P = predeploys as Record<string, string>

/** ABIs tried for a call to `to`, most specific first. */
function abisFor(to: string | null): { abi: Abi; standard: Standard; contract: string | null }[] {
  const t = to ? lower(to) : ''
  if (t === lower(P.STAKING!)) return [{ abi: staking, standard: 'staking', contract: 'Staking' }]
  if (t === lower(P.GOVERNANCE!)) return [{ abi: governance, standard: 'governance', contract: 'Governance' }]
  if (t === lower(P.XNS_REGISTRY!)) return [{ abi: xns, standard: 'xns', contract: 'XNS Registry' }]
  if (t === lower(P.PERMIT2_EXPIRING!)) return [{ abi: permit2, standard: 'permit2', contract: 'Permit2Expiring' }]
  if (t === lower(P.WXDS!)) return [
    { abi: wxds, standard: 'wxds', contract: 'WXDS' },
    { abi: erc20, standard: 'erc20', contract: 'WXDS' },
  ]
  // Unknown target: token standards first (most traffic), then the Guardian Vault / account surface.
  return [
    { abi: xdrc20, standard: 'xdrc20', contract: null },
    { abi: erc20, standard: 'erc20', contract: null },
    { abi: vaultAbi, standard: 'vault', contract: null },
    { abi: account, standard: 'account', contract: null },
  ]
}

export interface DecodedCall {
  standard: Standard
  contract: string | null
  name: string
  signature: string
  args: { name: string; type: string; value: unknown }[]
}

/** Decodes `input` sent to `to`, or `null` when no known ABI matches. Never throws. */
export function decodeCall(to: string | null, input: string): DecodedCall | null {
  if (!input || input === '0x' || input.length < 10) return null
  for (const { abi, standard, contract } of abisFor(to)) {
    try {
      const d = decodeFunctionData({ abi, data: input as Hex })
      const item = abi.find((x) => x.type === 'function' && x.name === d.functionName && (x.inputs?.length ?? 0) === (d.args?.length ?? 0))
      const inputs = item && item.type === 'function' ? item.inputs : []
      const args = (d.args ?? []).map((value, i) => ({ name: inputs[i]?.name || `arg${i}`, type: inputs[i]?.type ?? 'unknown', value }))
      return { standard, contract, name: d.functionName, signature: `${d.functionName}(${inputs.map((p) => p.type).join(',')})`, args }
    } catch {
      /* try the next ABI */
    }
  }
  return null
}

const eventAbis: { abi: Abi; standard: Standard }[] = [
  { abi: xdrc20, standard: 'xdrc20' },
  { abi: erc20, standard: 'erc20' },
  { abi: wxds, standard: 'wxds' },
  { abi: vaultAbi, standard: 'vault' },
  { abi: account, standard: 'account' },
  { abi: staking, standard: 'staking' },
]

export interface DecodedEvent {
  event: string
  standard: Standard
  args: Record<string, unknown>
}

/** Decodes one log against the known events, or `null`. Never throws. */
export function decodeLog(topics: readonly string[], data: string): DecodedEvent | null {
  if (!topics.length) return null
  for (const { abi, standard } of eventAbis) {
    try {
      const d = decodeEventLog({ abi, topics: topics as [Hex, ...Hex[]], data: data as Hex, strict: true })
      return { event: String(d.eventName), standard, args: (d.args ?? {}) as Record<string, unknown> }
    } catch {
      /* next */
    }
  }
  return null
}

/** JSON-safe, display-friendly rendering of a decoded value (bigint → decimal string, nested arrays kept). */
export function displayValue(v: unknown): string {
  if (typeof v === 'bigint') return v.toString()
  if (Array.isArray(v)) return `[${v.map(displayValue).join(', ')}]`
  if (v && typeof v === 'object') return `{ ${Object.entries(v).map(([k, x]) => `${k}: ${displayValue(x)}`).join(', ')} }`
  return String(v)
}

/** Guardian Vault activity in a transaction's logs (queued / cancelled / executed). */
export interface VaultActivity {
  account: string
  kind: 'queued' | 'cancelled' | 'executed'
  id: bigint
  target?: string
  value?: bigint
  data?: string
  executeAfter?: number
  by?: string
}

export function vaultActivity(logs: readonly { address: string; topics: readonly string[]; data: string }[]): VaultActivity[] {
  const out: VaultActivity[] = []
  for (const l of logs) {
    try {
      const d = decodeEventLog({ abi: vaultAbi, topics: l.topics as [Hex, ...Hex[]], data: l.data as Hex, strict: true })
      const a = d.args as Record<string, unknown>
      if (d.eventName === 'VaultActionQueued')
        out.push({ account: l.address, kind: 'queued', id: a.id as bigint, target: a.target as string, value: a.value as bigint, data: a.data as string, executeAfter: Number(a.executeAfter) })
      else if (d.eventName === 'VaultActionCancelled') out.push({ account: l.address, kind: 'cancelled', id: a.id as bigint, by: a.by as string })
      else if (d.eventName === 'VaultActionExecuted') out.push({ account: l.address, kind: 'executed', id: a.id as bigint })
    } catch {
      /* not a vault log */
    }
  }
  return out
}

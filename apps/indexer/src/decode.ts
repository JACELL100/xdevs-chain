// Event decoding: one table from topic0 to the known xDEVS / ERC-20 events.

import { decodeEventLog, toEventSelector, type AbiEvent, type AbiParameter, type Hex } from 'viem'
import { accountEvents, factoryEvents, permit2Events, vaultEvents, xdrc20Events, xnsEvents } from './abi.generated.ts'

export type Standard = 'erc20' | 'xdrc20' | 'permit2' | 'xns' | 'vault' | 'account' | 'factory' | 'wxds'

export const erc20Events = [
  {
    type: 'event',
    name: 'Transfer',
    inputs: [
      { type: 'address', name: 'from', indexed: true },
      { type: 'address', name: 'to', indexed: true },
      { type: 'uint256', name: 'value', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'Approval',
    inputs: [
      { type: 'address', name: 'owner', indexed: true },
      { type: 'address', name: 'spender', indexed: true },
      { type: 'uint256', name: 'value', indexed: false },
    ],
  },
] as const

export const wxdsEvents = [
  {
    type: 'event',
    name: 'Deposit',
    inputs: [
      { type: 'address', name: 'dst', indexed: true },
      { type: 'uint256', name: 'wad', indexed: false },
    ],
  },
  {
    type: 'event',
    name: 'Withdrawal',
    inputs: [
      { type: 'address', name: 'src', indexed: true },
      { type: 'uint256', name: 'wad', indexed: false },
    ],
  },
] as const

interface Known {
  item: AbiEvent
  standard: Standard
  signature: string
}

/** Canonical signature, e.g. `VaultPolicyUpdated((bool,bool,bool,uint64,uint128,uint128,uint128))`. */
export function signatureOf(item: AbiEvent): string {
  const type = (p: AbiParameter): string => {
    if (!p.type.startsWith('tuple')) return p.type
    const comps = (p as { components?: readonly AbiParameter[] }).components ?? []
    return `(${comps.map(type).join(',')})${p.type.slice('tuple'.length)}`
  }
  return `${item.name}(${item.inputs.map(type).join(',')})`
}

const table = new Map<Hex, Known>()
const add = (items: readonly unknown[], standard: Standard) => {
  for (const raw of items) {
    const item = raw as AbiEvent
    const sel = toEventSelector(item)
    if (table.has(sel)) continue // first registration wins (e.g. GovernanceChanged in two contracts)
    table.set(sel, { item, standard, signature: signatureOf(item) })
  }
}
add(erc20Events, 'erc20')
add(xdrc20Events, 'xdrc20')
add(permit2Events, 'permit2')
add(xnsEvents, 'xns')
add(vaultEvents, 'vault')
add(accountEvents, 'account')
add(factoryEvents, 'factory')
add(wxdsEvents, 'wxds')

export const TOPIC = Object.fromEntries(
  [...table.entries()].map(([sel, k]) => [`${k.standard}.${k.item.name}`, sel]),
) as Record<string, Hex>

export interface RawLog {
  address: string
  topics: readonly string[]
  data: string
}

export interface DecodedEvent {
  event: string
  signature: string
  standard: Standard
  /** viem-decoded args (bigint for integers). */
  args: Record<string, unknown>
}

/** Decodes a log against the known events; `null` if the topic0 is unknown or the shape does not match. */
export function decodeLog(log: RawLog): DecodedEvent | null {
  const t0 = log.topics[0]?.toLowerCase() as Hex | undefined
  if (!t0) return null
  const known = table.get(t0)
  if (!known) return null
  try {
    const res = decodeEventLog({
      abi: [known.item],
      topics: log.topics as [Hex, ...Hex[]],
      data: log.data as Hex,
      strict: true,
    })
    return { event: known.item.name, signature: known.signature, standard: known.standard, args: res.args as never }
  } catch {
    return null // e.g. ERC-721 Transfer (4 topics) against the ERC-20 shape
  }
}

/** JSON-safe copy: bigint → 0x hex, nested arrays/objects preserved. */
export function toJsonSafe(v: unknown): unknown {
  if (typeof v === 'bigint') return `0x${v.toString(16)}`
  if (Array.isArray(v)) return v.map(toJsonSafe)
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, toJsonSafe(x)]))
  return v
}

export const MAX_UINT256 = 2n ** 256n - 1n
export const MAX_UINT160 = 2n ** 160n - 1n

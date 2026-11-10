// Recipient resolution (address or `.xdevs` name) and address-poisoning checks (plan §16.3).
import { type Address, type PublicClient, getAddress, isAddress } from 'viem'
import {
  XdevsMethodNotAvailableError,
  checkRecipient as sdkCheckRecipient,
  findLookAlikeAddresses,
  isMethodNotFoundError,
  normalizeXnsName,
  resolveXnsName,
  xnsNamehash,
} from '@xdevs/sdk'
import { xnsAbi } from './abi.ts'
import { XNS_REGISTRY } from './account.ts'

export type ResolvedRecipient = { address: Address; name: string | null; via: 'address' | 'node' | 'registry' }

const notAvailable = (e: unknown) => e instanceof XdevsMethodNotAvailableError || isMethodNotFoundError(e)

/** Resolves `0x…` or an XNS name (`alice` / `alice.xdevs`). Throws on invalid / unregistered / expired names. */
export async function resolveRecipient(client: PublicClient, input: string): Promise<ResolvedRecipient> {
  const s = input.trim()
  if (/^0x/i.test(s)) {
    if (!isAddress(s, { strict: false })) throw new Error('not a valid address')
    return { address: getAddress(s), name: null, via: 'address' }
  }
  const name = normalizeXnsName(s) // throws InvalidXnsNameError (look-alike-proof rules)
  try {
    const a = await resolveXnsName(client, { name })
    if (!a) throw new Error(`${name} is not registered (or expired)`)
    return { address: a, name, via: 'node' }
  } catch (e) {
    if (!notAvailable(e)) throw e
  }
  const a = await client.readContract({ address: XNS_REGISTRY, abi: xnsAbi, functionName: 'resolveAddr', args: [xnsNamehash(name)] })
  if (!a || /^0x0{40}$/i.test(a)) throw new Error(`${name} is not registered (or expired)`)
  return { address: getAddress(a), name, via: 'registry' }
}

export type RecipientCheckResult = {
  firstTime: boolean
  lookAlikeOf: Address[]
  name: string | null
  isContract: boolean
  source: 'node' | 'local'
}

/**
 * `xdevs_checkRecipient` when the node supports it, else local: the wallet's own send history (first-time and
 * look-alike via the SDK helper), code check, and the forward-verified XNS primary name.
 */
export async function checkRecipient(
  client: PublicClient,
  from: Address,
  to: Address,
  history: readonly Address[],
): Promise<RecipientCheckResult> {
  const localLookAlikes = findLookAlikeAddresses(to, history)
  try {
    const r = await sdkCheckRecipient(client, { from, to })
    const lookAlikeOf = [...new Set([...r.lookAlikeOf, ...localLookAlikes].map((a) => getAddress(a)))]
    return { firstTime: r.firstTime, lookAlikeOf, name: r.name, isContract: r.isContract, source: 'node' }
  } catch (e) {
    if (!notAvailable(e)) throw e
  }
  const [code, name] = await Promise.all([
    client.getCode({ address: to }),
    client.readContract({ address: XNS_REGISTRY, abi: xnsAbi, functionName: 'primaryName', args: [to] }).catch(() => ''),
  ])
  return {
    firstTime: !history.some((h) => h.toLowerCase() === to.toLowerCase()),
    lookAlikeOf: localLookAlikes,
    name: name || null,
    isContract: !!code && code !== '0x',
    source: 'local',
  }
}

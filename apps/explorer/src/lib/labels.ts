// Human names for well-known addresses: the genesis predeploys (same on every xDEVS network) and, in demo mode,
// the fixture cast. Indexer labels and XNS names come with the data and take precedence.

import { MULTICALL3_ADDRESS, predeploys } from '@xdevs/sdk'
import { config } from './config.ts'
import { DEMO_NAMES } from './fixtures.ts'

export const PREDEPLOY_NAMES: Record<string, string> = Object.fromEntries(
  Object.entries({
    STAKING: 'Staking',
    GOVERNANCE: 'Governance',
    TREASURY: 'Treasury',
    FEE_CONFIG: 'FeeConfig',
    XNS_REGISTRY: 'XNS Registry',
    SMART_ACCOUNT_IMPL: 'Smart Account impl.',
    SMART_ACCOUNT_FACTORY: 'Account Factory',
    PAYMASTER: 'Paymaster',
    PERMIT2_EXPIRING: 'Permit2Expiring',
    WXDS: 'WXDS',
    SYSTEM_ADDRESS: 'System',
    P256_VERIFY: 'P256VERIFY',
  }).map(([k, label]) => [(predeploys as Record<string, string>)[k]!.toLowerCase(), label]),
)
PREDEPLOY_NAMES[MULTICALL3_ADDRESS.toLowerCase()] = 'Multicall3'

export const isPredeploy = (a: string | null | undefined) => !!a && a.toLowerCase() in PREDEPLOY_NAMES

/** Short display label for `address`, or null. */
export function knownLabel(address: string | null | undefined): string | null {
  if (!address) return null
  const k = address.toLowerCase()
  return PREDEPLOY_NAMES[k] ?? (config.demo ? (DEMO_NAMES[k] ?? null) : null)
}

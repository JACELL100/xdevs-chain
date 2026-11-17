// NEXT_PUBLIC_DEMO=1 fixtures. Every screen that shows them carries a "Demo data" label.
// Pick a scenario with ?scenario=ready|success|limited|empty|down|soon.

import type { ClaimOutcome, FaucetStatus } from './faucet-api'

export const DEMO_SCENARIOS = ['ready', 'success', 'limited', 'empty', 'down', 'soon'] as const
export type DemoScenario = (typeof DEMO_SCENARIOS)[number]

export const DEMO_ADDRESS = '0x8ba1f109551bd432803012645ac136ddd64dba72'
const DEMO_FAUCET = '0x5a0b54d5dc17e0aadc383d2db43b0a0d3e029c4c'

export function parseScenario(search: string): DemoScenario {
  const v = new URLSearchParams(search).get('scenario')
  return (DEMO_SCENARIOS as readonly string[]).includes(v ?? '') ? (v as DemoScenario) : 'ready'
}

export function demoStatus(s: DemoScenario): FaucetStatus {
  const base = { address: DEMO_FAUCET, chainId: 717171, symbol: 'tXDS', dripAmount: '100', minBalance: '1000' }
  switch (s) {
    case 'soon':
      return { reachable: false, healthy: false }
    case 'down':
      return { reachable: true, healthy: false, ...base, balance: null, error: 'rpc_unavailable' }
    case 'empty':
      return { reachable: true, healthy: false, ...base, balance: '912.5' }
    default:
      return { reachable: true, healthy: true, ...base, balance: '18450' }
  }
}

/** A stable fake tx hash for an address (not a real transaction). */
export function demoTxHash(seed: string): `0x${string}` {
  let h = 0x811c9dc5
  let out = ''
  for (let i = 0; out.length < 64; i++) {
    h ^= seed.charCodeAt(i % seed.length) + i
    h = Math.imul(h, 0x01000193) >>> 0
    out += h.toString(16).padStart(8, '0')
  }
  return `0x${out.slice(0, 64)}`
}

export function demoClaim(address: string, scenario: DemoScenario): ClaimOutcome {
  if (scenario === 'limited') {
    return {
      ok: false,
      kind: 'rate_limited',
      code: 'rate_limited',
      message: 'This address already received tXDS in the last 24 hours.',
      retryAfter: 17 * 3600 + 42 * 60,
    }
  }
  if (scenario === 'empty') {
    return {
      ok: false,
      kind: 'faucet_empty',
      code: 'faucet_empty',
      message: 'The faucet is running low and is paused until it is refilled.',
    }
  }
  if (scenario === 'down') {
    return {
      ok: false,
      kind: 'network_down',
      code: 'rpc_unavailable',
      message: 'The xDEVS Chain RPC node is unavailable right now. Please try again later.',
    }
  }
  return { ok: true, txHash: demoTxHash(address), to: address, amount: '100', symbol: 'tXDS', explorerUrl: null }
}

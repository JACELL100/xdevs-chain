// Build-time configuration (NEXT_PUBLIC_* are inlined by `next build`; each must be read literally).
// Defaults are the planned Enclave endpoints.

import { ENCLAVE, XDEVS_LINKS } from '@xdevs/ui/network'

const trim = (s: string) => s.replace(/\/+$/, '')

export const config = {
  rpcUrl: trim(process.env.NEXT_PUBLIC_RPC_URL || ENCLAVE.rpcUrl),
  wsUrl: trim(process.env.NEXT_PUBLIC_WS_URL || ENCLAVE.wsUrl),
  explorerUrl: trim(process.env.NEXT_PUBLIC_EXPLORER_URL || ENCLAVE.explorerUrl),
  explorerApiUrl: trim(process.env.NEXT_PUBLIC_EXPLORER_API_URL || ENCLAVE.explorerApiUrl),
  /** Faucet site; its API (the faucet Worker) is `${faucetUrl}/api`. */
  faucetUrl: trim(process.env.NEXT_PUBLIC_FAUCET_URL || XDEVS_LINKS.faucet),
  bundlerUrl: trim(process.env.NEXT_PUBLIC_BUNDLER_URL || ENCLAVE.bundlerUrl),
  siteUrl: trim(process.env.NEXT_PUBLIC_SITE_URL || XDEVS_LINKS.site),
  /** Size of the genesis validator set, used until `xdevs_validators` (Phase 3) is served. */
  validatorCount: Number(process.env.NEXT_PUBLIC_VALIDATOR_COUNT || 4),
  /** Set to 1 once Enclave has launched: an unreachable RPC then reads as an outage, not "not launched yet". */
  launched: process.env.NEXT_PUBLIC_LAUNCHED === '1',
  /** Labeled fixture data instead of live probes, for design review. */
  demo: process.env.NEXT_PUBLIC_DEMO === '1',
} as const

/** Planned network parameters, shown before launch and on /rpc. */
export const PLANNED = {
  blockTimeSeconds: 2,
  finality: 'Single-slot BFT (Malachite): final on commit',
  validators: config.validatorCount,
} as const

export const docsLinks = {
  endpoints: XDEVS_LINKS.endpoints,
  jsonRpc: `${XDEVS_LINKS.site}/developers/json-rpc`,
  quickstart: `${XDEVS_LINKS.site}/guide/quickstart`,
} as const

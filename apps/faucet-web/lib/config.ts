// Build-time configuration (NEXT_PUBLIC_* are inlined by `next build`; each must be read literally).
// Defaults target the Enclave testnet; the faucet API is the apps/faucet Worker routed at /api/*.

import { ENCLAVE } from '@xdevs/ui/network'

function trimSlash(s: string): string {
  return s.replace(/\/+$/, '')
}

export const config = {
  /** Faucet Worker API base: same origin by default (faucet.xdevschain.tech/api/* is routed to the Worker). */
  apiBase: trimSlash(process.env.NEXT_PUBLIC_FAUCET_API || '/api'),
  /** Cloudflare Turnstile site key (public). Empty = captcha not configured (claims disabled). */
  turnstileSiteKey: process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY || '',
  explorerUrl: trimSlash(process.env.NEXT_PUBLIC_EXPLORER_URL || ENCLAVE.explorerUrl),
  statusUrl: trimSlash(process.env.NEXT_PUBLIC_STATUS_URL || 'https://status.xdevschain.tech'),
  /** Shown until /api/status answers (the Worker's DRIP_AMOUNT is authoritative). */
  dripAmount: process.env.NEXT_PUBLIC_DRIP_AMOUNT || '100',
  /** Accept .xdevs names (only once XNS and the Worker's resolveName() are live). */
  xnsEnabled: process.env.NEXT_PUBLIC_XNS_ENABLED === '1',
  /** Labeled fixture data instead of real API calls, for design review. */
  demo: process.env.NEXT_PUBLIC_DEMO === '1',
} as const

export const COOLDOWN_SECONDS = 24 * 60 * 60

export function txUrl(hash: string): string {
  return `${config.explorerUrl}/tx/${hash}`
}

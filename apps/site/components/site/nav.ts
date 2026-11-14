/** Site-wide constants and the top navigation (same structure as the old VitePress nav). */

export const SITE_URL = 'https://xdevschain.tech'
export const SITE_NAME = 'xDEVS Chain'
export const TAGLINE = 'The chain that has your back'
export const SITE_DESCRIPTION =
  'xDEVS Chain is an EVM-equivalent Proof-of-Stake Layer 1 that protects users at the protocol level: passkey smart accounts, Guardian Vault, expiring approvals and look-alike-proof .xdevs names.'
export const GITHUB_URL = 'https://github.com/JACELL100/xdevs-chain'
export const XDEVS_URL = 'https://xdevs.in'

export type NavLink = { text: string; href: string; activeMatch?: string; external?: boolean }

export const NAV: NavLink[] = [
  { text: 'Guide', href: '/guide/introduction', activeMatch: '/guide/' },
  { text: 'Concepts', href: '/concepts/safety-layer', activeMatch: '/concepts/' },
  { text: 'Developers', href: '/developers/json-rpc', activeMatch: '/developers/' },
  { text: 'Node operators', href: '/operators/run-a-node', activeMatch: '/operators/' },
]

export const NAV_MORE: NavLink[] = [
  { text: 'Network endpoints', href: '/network/endpoints', activeMatch: '/network/' },
  { text: 'Roadmap', href: '/roadmap', activeMatch: '/roadmap' },
  { text: 'Security', href: '/security', activeMatch: '/security' },
  { text: 'THE xDEVS', href: XDEVS_URL, external: true },
]

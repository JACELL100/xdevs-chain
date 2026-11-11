/**
 * Docs sidebar: the same groups, labels and order as the old VitePress sidebar.
 * Every page under content/ must appear here exactly once (checked at build
 * time in lib/docs/content.ts), which also gives prev/next order.
 */

export type DocsNavItem = { text: string; href: string }
export type DocsNavGroup = { text: string; items: DocsNavItem[] }

export const DOCS_NAV: DocsNavGroup[] = [
  {
    text: 'Getting started',
    items: [
      { text: 'Introduction', href: '/guide/introduction' },
      { text: 'Why xDEVS Chain?', href: '/guide/why' },
      { text: 'Quickstart (local dev chain)', href: '/guide/quickstart' },
    ],
  },
  {
    text: 'Concepts',
    items: [
      { text: 'The user-safety layer', href: '/concepts/safety-layer' },
      { text: 'Smart accounts & passkeys', href: '/concepts/smart-accounts' },
      { text: 'Guardian Vault & freeze', href: '/concepts/guardian-vault' },
      { text: 'EOA lock (XIP-0001)', href: '/concepts/eoa-lock' },
      { text: 'Expiring approvals', href: '/concepts/approvals' },
      { text: 'XNS names & address poisoning', href: '/concepts/xns' },
      { text: 'Fees & the tXDS coin', href: '/concepts/fees' },
      { text: 'Predeployed contracts', href: '/concepts/predeploys' },
    ],
  },
  {
    text: 'Developers',
    items: [
      { text: 'JSON-RPC reference', href: '/developers/json-rpc' },
      { text: 'TypeScript SDK', href: '/developers/sdk' },
      { text: 'Deploying contracts', href: '/developers/deploy-contracts' },
      { text: 'Chain IDs & parameters', href: '/developers/chain-ids' },
    ],
  },
  {
    text: 'Node operators',
    items: [
      { text: 'Run a node', href: '/operators/run-a-node' },
      { text: 'Local multi-validator network', href: '/operators/local-network' },
      { text: 'Configuration reference', href: '/operators/configuration' },
      { text: 'Validators (coming)', href: '/operators/validators' },
    ],
  },
  {
    text: 'Network & project',
    items: [
      { text: 'Network endpoints', href: '/network/endpoints' },
      { text: 'Roadmap', href: '/roadmap' },
      { text: 'Security', href: '/security' },
    ],
  },
]

export const DOCS_FLAT: (DocsNavItem & { group: string })[] = DOCS_NAV.flatMap((g) =>
  g.items.map((i) => ({ ...i, group: g.text })),
)

export const REPO_URL = 'https://github.com/JACELL100/xdevs-chain'
export const EDIT_BASE = `${REPO_URL}/edit/main/apps/site/content`

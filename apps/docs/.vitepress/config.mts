import { defineConfig } from 'vitepress'

const REPO = 'https://github.com/JACELL100/xdevs-chain'
const SITE = 'https://xdevschain.xdevs.in'

// `/` for the custom domain (xdevschain.xdevs.in) and Cloudflare Pages.
// Set DOCS_BASE=/xdevs-chain/ only for a GitHub Pages project URL without a custom domain.
const base = process.env.DOCS_BASE ?? '/'

export default defineConfig({
  base,
  lang: 'en-US',
  title: 'xDEVS Chain',
  description:
    'xDEVS Chain is an EVM-equivalent Proof-of-Stake Layer 1 that protects users at the protocol level: passkey smart accounts, Guardian Vault, expiring approvals and look-alike-proof .xdevs names.',
  cleanUrls: true,
  // The README in this folder describes the site itself; it is not a page.
  srcExclude: ['README.md'],
  sitemap: { hostname: SITE },
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}logo.svg` }],
    ['meta', { name: 'theme-color', content: '#6d28d9' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'xDEVS Chain' }],
    ['meta', { property: 'og:title', content: 'xDEVS Chain: the chain that has your back' }],
    [
      'meta',
      {
        property: 'og:description',
        content:
          'An EVM-equivalent PoS Layer 1 with user protection built into the chain. Built by THE xDEVS.',
      },
    ],
    ['meta', { property: 'og:url', content: SITE }],
  ],

  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'xDEVS Chain',

    nav: [
      { text: 'Guide', link: '/guide/introduction', activeMatch: '/guide/' },
      { text: 'Concepts', link: '/concepts/safety-layer', activeMatch: '/concepts/' },
      { text: 'Developers', link: '/developers/json-rpc', activeMatch: '/developers/' },
      { text: 'Node operators', link: '/operators/run-a-node', activeMatch: '/operators/' },
      {
        text: 'More',
        items: [
          { text: 'Network endpoints', link: '/network/endpoints' },
          { text: 'Roadmap', link: '/roadmap' },
          { text: 'Security', link: '/security' },
          { text: 'THE xDEVS', link: 'https://xdevs.in' },
        ],
      },
    ],

    sidebar: [
      {
        text: 'Getting started',
        items: [
          { text: 'Introduction', link: '/guide/introduction' },
          { text: 'Why xDEVS Chain?', link: '/guide/why' },
          { text: 'Quickstart (local dev chain)', link: '/guide/quickstart' },
        ],
      },
      {
        text: 'Concepts',
        items: [
          { text: 'The user-safety layer', link: '/concepts/safety-layer' },
          { text: 'Smart accounts & passkeys', link: '/concepts/smart-accounts' },
          { text: 'Guardian Vault & freeze', link: '/concepts/guardian-vault' },
          { text: 'EOA lock (XIP-0001)', link: '/concepts/eoa-lock' },
          { text: 'Expiring approvals', link: '/concepts/approvals' },
          { text: 'XNS names & address poisoning', link: '/concepts/xns' },
          { text: 'Fees & the tXDS coin', link: '/concepts/fees' },
          { text: 'Predeployed contracts', link: '/concepts/predeploys' },
        ],
      },
      {
        text: 'Developers',
        items: [
          { text: 'JSON-RPC reference', link: '/developers/json-rpc' },
          { text: 'TypeScript SDK', link: '/developers/sdk' },
          { text: 'Deploying contracts', link: '/developers/deploy-contracts' },
          { text: 'Chain IDs & parameters', link: '/developers/chain-ids' },
        ],
      },
      {
        text: 'Node operators',
        items: [
          { text: 'Run a node', link: '/operators/run-a-node' },
          { text: 'Local multi-validator network', link: '/operators/local-network' },
          { text: 'Configuration reference', link: '/operators/configuration' },
          { text: 'Validators (coming)', link: '/operators/validators' },
        ],
      },
      {
        text: 'Network & project',
        items: [
          { text: 'Network endpoints', link: '/network/endpoints' },
          { text: 'Roadmap', link: '/roadmap' },
          { text: 'Security', link: '/security' },
        ],
      },
    ],

    socialLinks: [{ icon: 'github', link: REPO }],

    editLink: {
      pattern: `${REPO}/edit/main/apps/docs/:path`,
      text: 'Edit this page on GitHub',
    },

    search: { provider: 'local' },

    outline: { level: [2, 3] },

    footer: {
      message:
        'Built by <a href="https://xdevs.in">THE xDEVS</a> · Apache-2.0 · Testnet tokens (tXDS) have no monetary value.',
      copyright: 'xDEVS Chain is pre-mainnet software. Public testnet not live yet.',
    },
  },
})

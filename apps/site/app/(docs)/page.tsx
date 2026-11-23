import type { Metadata } from 'next'
import Link from 'next/link'
import { DOCS_NAV } from '@/lib/docs/nav'
import { PRODUCT_URL } from '@/components/site/nav'

export const metadata: Metadata = {
  title: { absolute: 'xDEVS Chain Docs' },
  description:
    'Documentation for xDEVS Chain and the Enclave testnet: getting started, the user-safety layer, the JSON-RPC reference, the TypeScript SDK and running a node.',
  alternates: { canonical: '/' },
}

const BLURB: Record<string, string> = {
  'Getting started': 'What xDEVS Chain is, why it exists, and your first contract on Enclave.',
  Concepts: 'The user-safety layer: smart accounts, Guardian Vault, approvals, names, fees.',
  Developers: 'JSON-RPC reference, the TypeScript SDK, deploying contracts, chain ids.',
  'Node operators': 'Run a node, start a local 4-validator network, configure and validate.',
  'Network & project': 'Public endpoints, the roadmap and how security is handled.',
}

const START = [
  { t: 'Quickstart', d: 'Add Enclave, get tXDS, deploy a contract.', href: '/guide/quickstart' },
  { t: 'Network endpoints', d: 'RPC, WebSocket, explorer, faucet, bundler.', href: '/network/endpoints' },
  { t: 'JSON-RPC reference', d: 'eth_ plus the xdevs_ safety methods.', href: '/developers/json-rpc' },
]

export default function DocsHome() {
  return (
    <div className="d-page">
      <article className="d-article d-home" data-pagefind-ignore="all">
        <p className="d-home-kicker">xDEVS Chain documentation</p>
        <h1>Build on the chain that has your back.</h1>
        <p className="d-home-lead">
          Guides, concepts and references for xDEVS Chain, an EVM-equivalent Layer 1 with user protection built into the
          protocol, and for Enclave, its live public testnet. New here? Read the{' '}
          <Link href="/guide/introduction">introduction</Link> or visit <a href={PRODUCT_URL}>xdevschain.tech</a>.
        </p>

        <div className="d-home-start">
          {START.map((s) => (
            <Link key={s.href} href={s.href} className="d-home-card is-start">
              <strong>{s.t}</strong>
              <span>{s.d}</span>
            </Link>
          ))}
        </div>

        <div className="d-home-groups">
          {DOCS_NAV.map((g) => (
            <section key={g.text} className="d-home-card">
              <h2>{g.text}</h2>
              <p>{BLURB[g.text] ?? ''}</p>
              <ul>
                {g.items.map((i) => (
                  <li key={i.href}>
                    <Link href={i.href}>{i.text}</Link>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </article>
    </div>
  )
}

/** Shared server-rendered sections: the apps grid, the closing call to action and page heads. */
import type { ReactNode } from 'react'
import Link from 'next/link'
import { DOCS_URL, ENCLAVE, LINKS } from '@/lib/site'
import { rv } from '@/lib/rv'
import { ArrowUpRight, Book, Box, GitHub, Plug } from './icons'

type App = { name: string; url: string; host: string; body: string; img?: string; meta?: string; icon?: ReactNode; big?: boolean }

const APPS: App[] = [
  {
    name: 'Wallet',
    url: LINKS.wallet,
    host: 'wallet.xdevschain.tech',
    body: 'Passkey account, Guardian Vault, guardians, approvals and names. Try the demo without signing anything.',
    img: '/img/desk-wallet-home.webp',
    big: true,
  },
  {
    name: 'Explorer',
    url: LINKS.explorer,
    host: 'explorer.xdevschain.tech',
    body: 'Blocks, transactions, addresses, validators and .xdevs names, plus a public REST API behind it.',
    img: '/img/desk-explorer.webp',
    big: true,
  },
  {
    name: 'Faucet',
    url: LINKS.faucet,
    host: 'faucet.xdevschain.tech',
    body: '100 tXDS per claim, once every 24 hours, to deploy contracts and try the wallet.',
    img: '/img/desk-faucet.webp',
  },
  {
    name: 'Status',
    url: LINKS.status,
    host: 'status.xdevschain.tech',
    body: 'Block height, block time, finality, validators and every public endpoint, checked from your browser.',
    img: '/img/desk-status.webp',
  },
  {
    name: 'JSON-RPC',
    url: LINKS.rpcInfo,
    host: 'rpc.xdevschain.tech',
    body: 'The Ethereum JSON-RPC plus the xdevs_ safety methods, over HTTPS and WebSocket.',
    icon: <Plug />,
    meta: ENCLAVE.wsUrl,
  },
  {
    name: 'Bundler',
    url: `${ENCLAVE.bundlerUrl}/health`,
    host: 'bundler.xdevschain.tech',
    body: 'An ERC-4337 bundler that activates passkey accounts with sponsored gas.',
    icon: <Box />,
    meta: 'ERC-4337 · EntryPoint',
  },
  {
    name: 'Docs',
    url: DOCS_URL,
    host: 'docs.xdevschain.tech',
    body: 'Guides, concepts, the JSON-RPC reference, the SDK and how to run a node.',
    icon: <Book />,
    meta: 'Guide · Concepts · Developers · Operators',
  },
  {
    name: 'Source',
    url: LINKS.github,
    host: 'github.com/JACELL100/xdevs-chain',
    body: 'The Rust node, the Solidity system contracts, the SDK and every app on this page. Apache-2.0.',
    icon: <GitHub width={20} height={20} />,
    meta: 'Rust · Solidity · TypeScript',
  },
]

export function AppsGrid() {
  return (
    <div className="bento">
      {APPS.map((a, i) => (
        <a key={a.name} href={a.url} className={`app glow${a.big ? ' is-big' : ''}`} {...rv((i % 3) * 70)}>
          <div>
            <div className="app-head">
              <div>
                {a.icon ? (
                  <span className="feature" style={{ display: 'block' }}>
                    <span className="icon">{a.icon}</span>
                  </span>
                ) : null}
                <h3 className="app-name">{a.name}</h3>
                <p className="app-url">{a.host}</p>
              </div>
              <span className="go" aria-hidden="true">
                <ArrowUpRight />
              </span>
            </div>
            <p>{a.body}</p>
          </div>
          {a.img ? (
            <div className="shot">
              <img
                src={a.img}
                srcSet={`${a.img.replace('.webp', '-800.webp')} 800w, ${a.img} 1440w`}
                sizes={a.big ? '(max-width: 1000px) 90vw, 680px' : '(max-width: 1000px) 90vw, 460px'}
                alt={`${a.name} screenshot`}
                width={1440}
                height={900}
                loading="lazy"
                decoding="async"
              />
            </div>
          ) : (
            <p className="app-meta">{a.meta}</p>
          )}
        </a>
      ))}
    </div>
  )
}

export function Cta({
  title = 'Start on Enclave.',
  body = 'Create a passkey account, get free test coins and deploy your first contract. It takes a few minutes and costs nothing.',
}: {
  title?: string
  body?: string
}) {
  return (
    <section className="section is-tight" aria-labelledby="cta-title">
      <div className="wrap">
        <div className="cta" {...rv(0, 'scale')}>
          <div className="grid-bg" aria-hidden="true" />
          <h2 id="cta-title" className="h2">
            <span className="metal">{title}</span>
          </h2>
          <p className="lead">{body}</p>
          <div className="btns">
            <a className="btn is-primary" href={LINKS.wallet}>
              Open the wallet
            </a>
            <a className="btn is-ghost" href={LINKS.faucet}>
              Get tXDS
            </a>
            <Link className="btn is-ghost" href="/developers">
              Build on it
            </Link>
          </div>
          <p className="fine">Enclave is a test network. tXDS has no value and never converts to mainnet XDS.</p>
        </div>
      </div>
    </section>
  )
}

export function PageHead({ eyebrow, title, lead, children }: { eyebrow: string; title: ReactNode; lead: ReactNode; children?: ReactNode }) {
  return (
    <header className="head" style={{ marginBottom: 0 }}>
      <p className="eyebrow" {...rv()}>
        {eyebrow}
      </p>
      <h1 className="h-hero h-page" {...rv(60)}>
        {title}
      </h1>
      <p className="lead" {...rv(120)}>
        {lead}
      </p>
      {children ? <div {...rv(180)}>{children}</div> : null}
    </header>
  )
}

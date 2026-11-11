import Link from 'next/link'
import { GITHUB_URL, XDEVS_URL } from './nav'
import './site.css'

export type FooterProps = {
  /** 'full' = landing footer with link columns; 'compact' = one quiet row for docs pages. */
  variant?: 'full' | 'compact'
}

const COLS: { h: string; links: { t: string; href: string; external?: boolean }[] }[] = [
  {
    h: 'start',
    links: [
      { t: 'Introduction', href: '/guide/introduction' },
      { t: 'Why xDEVS Chain?', href: '/guide/why' },
      { t: 'Quickstart', href: '/guide/quickstart' },
    ],
  },
  {
    h: 'build',
    links: [
      { t: 'JSON-RPC', href: '/developers/json-rpc' },
      { t: 'TypeScript SDK', href: '/developers/sdk' },
      { t: 'Chain IDs', href: '/developers/chain-ids' },
    ],
  },
  {
    h: 'project',
    links: [
      { t: 'Roadmap', href: '/roadmap' },
      { t: 'Security', href: '/security' },
      { t: 'GitHub', href: GITHUB_URL, external: true },
    ],
  },
]

const YEAR = 2026

export default function Footer({ variant = 'full' }: FooterProps) {
  const legal = (
    <p className="xh-foot-legal">
      Apache-2.0 · Pre-mainnet software; public testnet not live yet. Testnet tokens (tXDS) have no monetary value. © {YEAR}{' '}
      THE xDEVS
    </p>
  )
  const by = (
    <p className="xh-foot-by">
      Built by{' '}
      <a href={XDEVS_URL} target="_blank" rel="noopener">
        THE xDEVS
      </a>
    </p>
  )

  if (variant === 'compact') {
    return (
      <footer className="xh-foot is-compact">
        <div className="xh-foot-bottom">
          {by}
          {legal}
        </div>
      </footer>
    )
  }

  return (
    <footer className="xh-foot">
      <div className="xh-wrap">
        <div className="xh-foot-top">
          <div>
            <Link href="/" className="xh-foot-logo" aria-label="xDEVS Chain home">
              <img src="/brand/xdevs-chain-logo.svg" alt="xDEVS Chain" width={150} height={36} />
            </Link>
            <p className="xh-foot-tag">&lt; the chain that has your back /&gt;</p>
          </div>
          <nav className="xh-foot-cols" aria-label="Footer">
            {COLS.map((c) => (
              <div key={c.h}>
                <h2>{c.h}</h2>
                <ul>
                  {c.links.map((l) => (
                    <li key={l.t}>
                      {l.external ? (
                        <a href={l.href} target="_blank" rel="noopener">
                          {l.t}
                        </a>
                      ) : (
                        <Link href={l.href}>{l.t}</Link>
                      )}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </nav>
        </div>
        <div className="xh-foot-bottom">
          {by}
          {legal}
        </div>
      </div>
    </footer>
  )
}

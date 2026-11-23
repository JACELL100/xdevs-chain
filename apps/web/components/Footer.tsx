import Link from 'next/link'
import { DOCS_URL, ENCLAVE, LINKS, docs } from '@/lib/site'

type L = { t: string; href: string }
const COLS: { h: string; links: L[] }[] = [
  {
    h: 'Products',
    links: [
      { t: 'Wallet', href: LINKS.wallet },
      { t: 'Explorer', href: LINKS.explorer },
      { t: 'Faucet', href: LINKS.faucet },
      { t: 'Network status', href: LINKS.status },
      { t: 'Wallet demo', href: LINKS.walletDemo },
    ],
  },
  {
    h: 'Network',
    links: [
      { t: 'Enclave testnet', href: '/enclave' },
      { t: 'JSON-RPC endpoint', href: LINKS.rpcInfo },
      { t: 'Explorer API', href: `${ENCLAVE.explorerApiUrl}/api/v1/stats` },
      { t: 'ERC-4337 bundler', href: `${ENCLAVE.bundlerUrl}/health` },
      { t: 'Network endpoints', href: docs('/network/endpoints') },
    ],
  },
  {
    h: 'Developers',
    links: [
      { t: 'Developers', href: '/developers' },
      { t: 'Documentation', href: DOCS_URL },
      { t: 'Quickstart', href: docs('/guide/quickstart') },
      { t: 'JSON-RPC reference', href: docs('/developers/json-rpc') },
      { t: 'Run a node', href: docs('/operators/run-a-node') },
    ],
  },
  {
    h: 'Project',
    links: [
      { t: 'Roadmap', href: '/roadmap' },
      { t: 'Security', href: '/security' },
      { t: 'Brand assets', href: '/brand' },
      { t: 'GitHub', href: LINKS.github },
      { t: 'THE xDEVS', href: LINKS.xdevs },
    ],
  },
]

const isInternal = (href: string) => href.startsWith('/')

export default function Footer() {
  return (
    <footer className="footer">
      <div className="wrap is-wide">
        <div className="footer-top">
          <div className="footer-brand">
            <Link href="/" className="brand" aria-label="xDEVS Chain home">
              <img src="/brand/xdevs-chain-mark.svg" width={26} height={26} alt="" />
              <span>
                xDEVS <span>Chain</span>
              </span>
            </Link>
            <p>The chain that has your back. An EVM-equivalent Layer 1 with user protection built into the protocol.</p>
            <a className="chip is-live" href={LINKS.status}>
              <span className="dot is-pulse" />
              Enclave testnet is live
            </a>
          </div>
          {COLS.map((c) => (
            <nav key={c.h} aria-label={c.h}>
              <h2>{c.h}</h2>
              <ul>
                {c.links.map((l) => (
                  <li key={l.t}>{isInternal(l.href) ? <Link href={l.href}>{l.t}</Link> : <a href={l.href}>{l.t}</a>}</li>
                ))}
              </ul>
            </nav>
          ))}
        </div>
        <p className="footer-word" aria-hidden="true">
          xDEVS Chain
        </p>
        <div className="footer-bottom">
          <p>
            © 2026{' '}
            <a href={LINKS.xdevs}>THE xDEVS</a> · Open source under Apache-2.0 · Pre-mainnet software. Testnet tokens (tXDS) have no
            value.
          </p>
          <p>
            <a href={LINKS.github}>GitHub</a> · <a href={DOCS_URL}>Docs</a> · <a href={LINKS.status}>Status</a>
          </p>
        </div>
      </div>
    </footer>
  )
}

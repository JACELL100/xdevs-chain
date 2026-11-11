import type { Metadata } from 'next'
import Link from 'next/link'
import Header from '@/components/site/Header'
import Footer from '@/components/site/Footer'
import Search from '@/components/docs/Search'
import '@/components/docs/docs.css'

export const metadata: Metadata = {
  title: { absolute: 'Page not found · xDEVS Chain' },
  description: 'This page does not exist. Find your way back to the xDEVS Chain docs.',
  robots: { index: false },
}

const POPULAR = [
  { text: 'Introduction', href: '/guide/introduction' },
  { text: 'Quickstart', href: '/guide/quickstart' },
  { text: 'Guardian Vault', href: '/concepts/guardian-vault' },
  { text: 'JSON-RPC', href: '/developers/json-rpc' },
  { text: 'Run a node', href: '/operators/run-a-node' },
  { text: 'Roadmap', href: '/roadmap' },
]

export default function NotFound() {
  return (
    <>
      <Header variant="docs" search={<Search />} />
      <main id="main" className="d-404">
        <div className="d-404-inner">
          <img className="d-404-mark" src="/brand/xdevs-chain-mark.svg" width={56} height={56} alt="" />
          <p className="xh-label">
            <b>404</b> <i>//</i> not finalized
          </p>
          <h1 className="xh-metal">This block was never produced.</h1>
          <p>The page you’re looking for doesn’t exist, or it moved. The docs are a search away.</p>
          <div className="xh-ctas">
            <Link className="xh-btn xh-btn-primary" href="/guide/introduction">
              Read the docs
            </Link>
            <Link className="xh-btn xh-btn-ghost" href="/">
              Back to home
            </Link>
          </div>
          <ul className="d-404-links" aria-label="Popular pages">
            {POPULAR.map((l) => (
              <li key={l.href}>
                <Link href={l.href}>{l.text}</Link>
              </li>
            ))}
          </ul>
        </div>
      </main>
      <div className="d-footer">
        <Footer variant="compact" />
      </div>
    </>
  )
}

import Link from 'next/link'
import { DOCS_URL } from '@/lib/site'

export default function NotFound() {
  return (
    <section className="notfound page-top">
      <div className="wrap" style={{ display: 'grid', gap: 22, justifyItems: 'center' }}>
        <p className="eyebrow">404</p>
        <h1 className="h2">
          <span className="metal">This page isn’t on the chain.</span>
        </h1>
        <p className="lead">
          Looking for the documentation? It moved to <a className="inline-link" href={DOCS_URL}>docs.xdevschain.tech</a>.
        </p>
        <div className="btns" style={{ justifyContent: 'center' }}>
          <Link className="btn is-primary" href="/">
            Home
          </Link>
          <a className="btn is-ghost" href={DOCS_URL}>
            Docs
          </a>
        </div>
      </div>
    </section>
  )
}

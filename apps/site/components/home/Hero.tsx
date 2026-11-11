import Link from 'next/link'
import { GITHUB_URL } from '@/components/site/nav'
import AnimatedMark from './AnimatedMark'
import HeroField from './HeroField'
import { Arrow, ArrowUpRight, Chevron } from './icons'
import './hero.css'

const INDEX = [
  { n: '01', t: 'finality', href: '#finality' },
  { n: '02', t: 'guardian vault', href: '#vault' },
  { n: '03', t: 'passkeys', href: '#passkeys' },
  { n: '04', t: 'approvals & names', href: '#safety' },
]

export default function Hero() {
  return (
    <section className="hero" aria-labelledby="hero-title">
      <HeroField />
      <div className="hero-glow" aria-hidden="true" />

      <div className="hero-inner xh-wrap">
        <div className="hero-mark">
          <AnimatedMark uid="hero" />
        </div>

        <Link className="pill" href="/network/endpoints">
          <span className="pill-tag">Testnet</span>
          <span className="pill-text">Public testnet coming soon</span>
          <Chevron />
        </Link>

        <h1 id="hero-title" className="xh-display hero-title">
          <span className="line">
            <span className="xh-metal">The chain that</span>
          </span>
          <span className="line">
            <span className="xh-metal">has your back.</span>
          </span>
        </h1>

        <p className="hero-lede">
          An EVM-equivalent Proof-of-Stake Layer&nbsp;1 with user protection built into the chain itself. No seed
          phrases. No instant drains. No forever approvals. No look-alike names.
        </p>

        <div className="xh-ctas hero-ctas">
          <Link className="xh-btn xh-btn-primary" href="/guide/introduction">
            Read the docs
            <Arrow />
          </Link>
          <Link className="xh-btn xh-btn-ghost" href="/guide/quickstart">
            Run a local chain
          </Link>
          <a className="xh-link" href={GITHUB_URL} target="_blank" rel="noopener">
            GitHub
            <ArrowUpRight />
          </a>
        </div>
      </div>

      <nav className="hero-index xh-wrap" aria-label="On this page">
        <span className="hero-tag xh-mono" aria-hidden="true">
          &lt; protected by default /&gt;
        </span>
        <ol>
          {INDEX.map((i) => (
            <li key={i.n}>
              <a href={i.href}>
                <span className="n">{i.n}</span>
                <span className="s">//</span> {i.t}
              </a>
            </li>
          ))}
        </ol>
      </nav>
    </section>
  )
}

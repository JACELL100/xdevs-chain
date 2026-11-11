import Link from 'next/link'
import AnimatedMark from './AnimatedMark'
import { Arrow } from './icons'
import { rv } from './util'
import './closing.css'

export default function Closing() {
  return (
    <section className="xh-section s-close closing" aria-labelledby="closing-title">
      <div className="xh-wrap inner">
        <div className="mark" {...rv()}>
          <AnimatedMark uid="close" animate={false} />
        </div>
        <h2 id="closing-title" className="xh-h2" {...rv(60)}>
          <span className="xh-metal">Build on the chain</span>
          <br />
          <span className="xh-metal">that has your back.</span>
        </h2>
        <p className="xh-lede" {...rv(120)}>
          Run a 4-validator network on your laptop today. The public testnet is coming soon.
        </p>
        <div className="xh-ctas" {...rv(180)}>
          <Link className="xh-btn xh-btn-primary" href="/guide/quickstart">
            Run a local chain
            <Arrow />
          </Link>
          <Link className="xh-btn xh-btn-ghost" href="/guide/introduction">
            Read the docs
          </Link>
        </div>
      </div>
    </section>
  )
}

import ProofStats from './ProofStats'
import { rv } from './util'
import './proof.css'

export default function Proof() {
  return (
    <section id="proof" className="xh-section s-proof" aria-labelledby="proof-title">
      <div className="xh-wrap">
        <header className="head">
          <p className="xh-label" {...rv()}>
            <b>06</b>
            <i>//</i> measured
          </p>
          <h2 id="proof-title" className="xh-h2" {...rv(60)}>
            <span className="xh-metal">Measured,</span>
            <br />
            <span className="xh-quiet">not promised.</span>
          </h2>
          <p className="xh-lede" {...rv(120)}>
            A 5-hour soak of a 4-validator network under load. Every number here comes from a real run, not a
            projection.
          </p>
        </header>
        <ProofStats />
      </div>
    </section>
  )
}

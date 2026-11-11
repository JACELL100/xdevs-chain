import Link from 'next/link'
import { ApprovalWidget, NamesWidget } from './SafetyWidgets'
import { rv } from './util'
import './safety.css'

export default function Safety() {
  return (
    <section id="safety" className="xh-section s-safe" aria-labelledby="safety-title">
      <div className="xh-wrap">
        <header className="head">
          <p className="xh-label" {...rv()}>
            <b>04</b>
            <i>//</i> approvals &amp; names
          </p>
          <h2 id="safety-title" className="xh-h2" {...rv(60)}>
            <span className="xh-metal">Approvals that expire.</span>
            <br />
            <span className="xh-quiet">Names that can’t be faked.</span>
          </h2>
        </header>

        <div className="cols">
          <article className="xh-panel card" {...rv()}>
            <div className="card-copy">
              <h3 className="xh-h3">Every allowance has an end date.</h3>
              <p className="xh-body">
                XDRC-20 tokens and the Permit2Expiring contract put an expiry on every approval, even “unlimited” ones.
                When it passes, the allowance reads zero. One call revokes them all.
              </p>
            </div>
            <ApprovalWidget />
            <Link className="xh-textlink more" href="/concepts/approvals">
              Expiring approvals
            </Link>
          </article>

          <article className="xh-panel card" {...rv(90)}>
            <div className="card-copy">
              <h3 className="xh-h3">Look-alikes can’t register.</h3>
              <p className="xh-body">
                While paypal.xdevs exists, nobody can claim paypa1.xdevs. Confusable characters share one skeleton, and
                non-ASCII homographs are refused outright.
              </p>
            </div>
            <NamesWidget />
            <Link className="xh-textlink more" href="/concepts/xns">
              XNS names
            </Link>
          </article>
        </div>
      </div>
    </section>
  )
}

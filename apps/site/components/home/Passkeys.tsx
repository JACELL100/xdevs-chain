import Link from 'next/link'
import PasskeyPhone from './PasskeyPhone'
import { rv } from './util'
import './passkeys.css'

const FEATURES = [
  { h: 'Passkey login', p: 'Face ID, a fingerprint or a security key. Nothing to write down, nothing to lose in a drawer.' },
  { h: 'Social recovery', p: 'Lose your phone and the guardians you chose approve a new key, after a delay you can cancel.' },
  { h: 'Session keys', p: 'Give a game or app a key with a time window and spend caps, never the keys to everything.' },
]

export default function Passkeys() {
  return (
    <section id="passkeys" className="xh-section s-pk" aria-labelledby="passkeys-title">
      <div className="xh-wrap grid">
        <div className="visual" {...rv()}>
          <PasskeyPhone />
        </div>

        <div className="copy">
          <p className="xh-label" {...rv()}>
            <b>03</b>
            <i>//</i> passkeys
          </p>
          <h2 id="passkeys-title" className="xh-h2" {...rv(60)}>
            <span className="xh-metal">No seed phrase.</span>
            <br />
            <span className="xh-quiet">Just you.</span>
          </h2>
          <p className="xh-lede" {...rv(120)}>
            Accounts on xDEVS Chain can be smart accounts secured by a passkey on your own device. The chain checks the
            signature natively, so there is no private key to copy, store or leak.
          </p>
          <ul className="feats">
            {FEATURES.map((f, i) => (
              <li key={f.h} {...rv(140 + i * 70)}>
                <h3>{f.h}</h3>
                <p className="xh-body">{f.p}</p>
              </li>
            ))}
          </ul>
          <Link className="xh-textlink" href="/concepts/smart-accounts" {...rv(320)}>
            How smart accounts work
          </Link>
        </div>
      </div>
    </section>
  )
}

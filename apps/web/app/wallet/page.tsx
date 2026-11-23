import type { Metadata } from 'next'
import { Laptop, Phone } from '@/components/Devices'
import { Cta, PageHead } from '@/components/Sections'
import { Arrow, Clock, Eye, Fingerprint, Lock, Plus, Shield, Tag, Users, Vault } from '@/components/icons'
import { LINKS, docs } from '@/lib/site'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: 'Wallet',
  description:
    'The xDEVS wallet: a passkey smart account with no seed phrase, a Guardian Vault, guardians, expiring approvals, .xdevs names and a safety check before every send. Live on the Enclave testnet.',
  alternates: { canonical: '/wallet' },
}

const FEATURES = [
  { icon: <Fingerprint />, t: 'Passkey sign-in', b: 'Face ID, a fingerprint or Windows Hello. No seed phrase, no extension, and the first activation is sponsored.' },
  { icon: <Vault />, t: 'Guardian Vault', b: 'Small payments go out instantly. Larger ones, approvals and unknown contract calls wait 24 hours by default and can be cancelled.' },
  { icon: <Users />, t: 'Guardians', b: 'People or devices you trust. A quorum can cancel a queued transfer, freeze the account and recover it to a new passkey.' },
  { icon: <Eye />, t: 'Review before you send', b: 'Every send is simulated by the node first, with plain warnings for first-time and look-alike recipients.' },
  { icon: <Clock />, t: 'Approvals manager', b: 'See every app that can spend your tokens, when each approval expires, and revoke one or all.' },
  { icon: <Tag />, t: '.xdevs names', b: 'Send to aria.xdevs instead of a 42-character address. Names that look like existing ones are refused.' },
  { icon: <Lock />, t: 'EOA lock for MetaMask addresses', b: 'Bring an existing address under the vault with EIP-7702, then lock the old key so it can’t go around it.' },
  { icon: <Shield />, t: 'Facts and advice, kept apart', b: 'On-chain facts are shown as facts. Heuristic warnings are labelled advice, never a guarantee of safety.' },
]

const FAQ = [
  ['Is there really no seed phrase?', 'Yes. The account is a smart contract that accepts your passkey’s signature. If you lose the device, your guardians recover the account to a new passkey.'],
  ['What happens if someone steals my phone?', 'They can spend at most your instant allowance. Anything bigger waits in the Guardian Vault, where you or your guardians cancel it, and guardians can freeze the account.'],
  ['Can xDEVS freeze or move my funds?', 'No. There is no admin key over user funds, and no validator or governance vote can move them. A freeze can only come from guardians you picked.'],
  ['Does it cost anything?', 'Not on Enclave. Account activation is sponsored, and tXDS from the faucet is free. tXDS has no value.'],
  ['Can I look around without creating an account?', 'Yes. Demo mode loads a sample account with sample data. Nothing is signed or sent, and no passkey is used.'],
]

export default function WalletPage() {
  return (
    <>
      <section className="page-top" aria-labelledby="wallet-title">
        <div className="wrap">
          <div className="hero-split">
            <div className="hero-copy">
              <PageHead
                eyebrow="xDEVS Wallet"
                title={
                  <span id="wallet-title">
                    <span className="metal">A wallet with</span>
                    <br />
                    <span className="metal">no seed phrase.</span>
                  </span>
                }
                lead="Unlock it with your face or fingerprint. Large transfers wait in your Guardian Vault, so a stolen device can’t drain you. It runs in the browser on the Enclave testnet."
              >
                <div className="btns" style={{ marginTop: 8 }}>
                  <a className="btn is-primary" href={LINKS.wallet}>
                    Create a wallet <Arrow />
                  </a>
                  <a className="btn is-ghost" href={LINKS.walletDemo}>
                    Try the demo
                  </a>
                </div>
              </PageHead>
            </div>
            <div style={{ display: 'flex', justifyContent: 'center', gap: 20 }} {...rv(200, 'scale')}>
              <div style={{ width: 'min(300px, 70vw)' }}>
                <Phone src="/img/phone-wallet-home.webp" alt="The wallet home screen in demo mode: aria.xdevs with 1,284.5 tXDS and the Guardian Vault on" eager />
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="section" aria-labelledby="feat-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              What’s inside
            </p>
            <h2 id="feat-title" className="h2" {...rv(60)}>
              <span className="metal">Protection you can see,</span> <span className="quiet">on every screen.</span>
            </h2>
          </header>
          <div className="grid-4">
            {FEATURES.map((f, i) => (
              <div key={f.t} className="card feature glow" {...rv((i % 4) * 70)}>
                <span className="icon">{f.icon}</span>
                <h3 className="h4">{f.t}</h3>
                <p>{f.b}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="queue-title">
        <div className="wrap">
          <div className="split">
            <div className="copy">
              <p className="eyebrow" {...rv()}>
                Guardian Vault
              </p>
              <h2 id="queue-title" className="h2" {...rv(60)}>
                Big transfers <span className="quiet">take a breath first.</span>
              </h2>
              <p className="lead" {...rv(120)}>
                Above your instant limit, a send joins a queue with a countdown. Nothing leaves your account until it runs out, and
                you or your guardians can cancel it with one tap. Switching the vault off waits out the same delay, so a thief
                can’t simply turn it off.
              </p>
              <p {...rv(160)}>
                <a className="text-cta" href={docs('/concepts/guardian-vault')}>
                  How the vault works <Arrow />
                </a>
              </p>
            </div>
            <div style={{ display: 'flex', justifyContent: 'center', gap: 'clamp(12px, 3vw, 28px)' }}>
              <div style={{ width: 'min(280px, 44vw)' }} {...rv(80, 'scale')}>
                <Phone src="/img/phone-wallet-queue.webp" alt="Send review: 600 tXDS to ren.xdevs will wait in the Guardian Vault for 24 hours" />
              </div>
              <div style={{ width: 'min(280px, 44vw)', marginTop: 64 }} {...rv(200, 'scale')}>
                <Phone src="/img/phone-wallet-vault.webp" alt="The Guardian Vault queue with countdowns and cancel buttons" />
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="section" aria-labelledby="desk-title">
        <div className="wrap">
          <header className="head is-center">
            <p className="eyebrow" {...rv()}>
              On your desk too
            </p>
            <h2 id="desk-title" className="h2" {...rv(60)}>
              Same account, <span className="quiet">any device with a passkey.</span>
            </h2>
          </header>
          <div style={{ maxWidth: 1000, margin: '0 auto' }} {...rv(100, 'scale')}>
            <Laptop src="/img/desk-wallet-vault.webp" alt="The Guardian Vault page of the wallet on a laptop" />
          </div>
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="faq-title">
        <div className="wrap">
          <div className="split" style={{ alignItems: 'start' }}>
            <header className="copy">
              <p className="eyebrow" {...rv()}>
                Questions
              </p>
              <h2 id="faq-title" className="h2" {...rv(60)}>
                Good to know.
              </h2>
            </header>
            <div className="faq" {...rv(100)}>
              {FAQ.map(([q, a]) => (
                <details key={q}>
                  <summary>
                    {q}
                    <Plus />
                  </summary>
                  <p>{a}</p>
                </details>
              ))}
            </div>
          </div>
        </div>
      </section>

      <Cta title="Your account is one passkey away." body="No seed phrase to write down and nothing to install. Enclave is a test network and tXDS has no value." />
    </>
  )
}

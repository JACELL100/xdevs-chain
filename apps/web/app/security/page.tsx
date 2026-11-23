import type { Metadata } from 'next'
import { Cta, PageHead } from '@/components/Sections'
import { Arrow, Check, Key, Layers, Lock, Shield, Warn } from '@/components/icons'
import { LINKS, docs } from '@/lib/site'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: 'Security',
  description:
    'How xDEVS Chain approaches security: no admin keys over user funds, opt-in protection that never reverses a finalized transaction, deterministic consensus, heavy testing and honest limits. How to report a vulnerability.',
  alternates: { canonical: '/security' },
}

const PRINCIPLES = [
  {
    icon: <Key />,
    t: 'No admin keys over user funds',
    b: 'No protocol key, validator or governance vote can freeze, seize or move anyone’s funds. A freeze can only come from guardians the owner picked.',
  },
  {
    icon: <Lock />,
    t: 'Finality is final',
    b: 'Protections decide what can happen next. They never undo a finalized transaction, and nothing reorganises a committed block.',
  },
  {
    icon: <Shield />,
    t: 'Opt-in by design',
    b: 'Someone who turns nothing on gets ordinary Ethereum behaviour. Every protection is a choice the account owner makes.',
  },
  {
    icon: <Layers />,
    t: 'Determinism first',
    b: 'Consensus code avoids floats, unordered iteration and wall-clock reads, so every node computes the same state root.',
  },
]

const TESTING = [
  'Unit and end-to-end tests for the node, driven through JSON-RPC',
  'Unit, fuzz and invariant tests for every contract area (Foundry)',
  'A Docker localnet with chaos scenarios on every pull request: crashes, partitions, packet loss',
  'A fork scan on every pull request',
  'A 5-hour soak of a 4-validator network: 360,000 transactions, 0 forks',
  'A persistent double-sign guard on every validator, separate from the chain database',
]

const LIMITS = [
  ['Pre-mainnet', 'Enclave is a testnet run on four validators operated by the project. It may be reset with notice.'],
  ['Not audited yet', 'Neither the node nor the contracts have had an external security audit. Audits come before any mainnet.'],
  ['Advice is not proof', 'Simulation and recipient warnings help you notice problems. They never prove a transaction is safe.'],
  ['Security grows with adoption', 'A new chain has far less stake and fewer independent operators than Ethereum. Until many join, this is not a decentralized network.'],
]

export default function SecurityPage() {
  return (
    <>
      <section className="page-top" aria-labelledby="sec-title">
        <div className="wrap">
          <PageHead
            eyebrow="Security"
            title={
              <span id="sec-title">
                <span className="metal">Safety is the product.</span>
                <br />
                <span className="quiet">So we’re careful about it.</span>
              </span>
            }
            lead="xDEVS Chain is pre-mainnet software. Here is how we build it, what is tested, what isn’t done yet, and how to tell us when you find a problem."
          />
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="principles-title">
        <div className="wrap">
          <h2 id="principles-title" className="sr-only">
            Principles
          </h2>
          <div className="grid-4">
            {PRINCIPLES.map((p, i) => (
              <div key={p.t} className="card feature glow" {...rv(i * 70)}>
                <span className="icon">{p.icon}</span>
                <h3 className="h4">{p.t}</h3>
                <p>{p.b}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="tested-title">
        <div className="wrap">
          <div className="split" style={{ alignItems: 'start' }}>
            <header className="copy">
              <p className="eyebrow" {...rv()}>
                How it’s tested
              </p>
              <h2 id="tested-title" className="h2" {...rv(60)}>
                Broken on purpose, <span className="quiet">every day.</span>
              </h2>
              <p className="lead" {...rv(120)}>
                Killing one of four validators keeps the chain going. Killing two halts it safely, and restarting resumes it with
                no forks. CI checks that on every change.
              </p>
            </header>
            <ul className="checks card" style={{ padding: 'clamp(24px, 3vw, 36px)', gap: 16 }} {...rv(100)}>
              {TESTING.map((t) => (
                <li key={t}>
                  <Check />
                  {t}
                </li>
              ))}
            </ul>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="limits-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Honest limits
            </p>
            <h2 id="limits-title" className="h2" {...rv(60)}>
              What we <span className="quiet">don’t claim.</span>
            </h2>
          </header>
          <div className="grid-2">
            {LIMITS.map(([t, b], i) => (
              <div key={t} className="card" {...rv((i % 2) * 80)}>
                <h3 className="h4" style={{ marginBottom: 8 }}>
                  {t}
                </h3>
                <p className="body">{b}</p>
              </div>
            ))}
          </div>
          <p className="small" style={{ marginTop: 24 }} {...rv()}>
            Before any mainnet: static analysis of all contracts, independent external audits of the node and contracts, a public
            bug bounty, and months of public testnet operation without critical incidents.
          </p>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="report-title">
        <div className="wrap">
          <div className="cta" style={{ textAlign: 'left' }} {...rv(0, 'scale')}>
            <div className="split" style={{ alignItems: 'start' }}>
              <div className="copy">
                <p className="eyebrow">Report a vulnerability</p>
                <h2 id="report-title" className="h2" style={{ margin: 0, maxWidth: 'none' }}>
                  Found something? <span className="quiet">Tell us privately.</span>
                </h2>
              </div>
              <div className="copy">
                <p className="body">
                  Please don’t open a public issue. Use GitHub’s private vulnerability reporting (“Report a vulnerability” under the
                  repository’s Security tab) and include the version, steps to reproduce and the impact. We aim to acknowledge
                  reports within 72 hours.
                </p>
                <p className="body">
                  In scope: the node, the CLI, every crate, the system contracts and the live Enclave services. Rewards today are
                  credit in the release notes and a Security Contributor role. Paid bounties come with a budget or a mainnet.
                </p>
                <div className="btns" style={{ justifyContent: 'flex-start' }}>
                  <a className="btn is-primary" href={LINKS.securityPolicy}>
                    Security policy <Arrow />
                  </a>
                  <a className="btn is-ghost" href={docs('/security')}>
                    Full security notes
                  </a>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="safe-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Staying safe
            </p>
            <h2 id="safe-title" className="h2" {...rv(60)}>
              Four rules <span className="quiet">worth keeping.</span>
            </h2>
          </header>
          <div className="grid-2">
            {[
              'Never share a seed phrase or private key. Nobody from xDEVS will ever ask for one.',
              'tXDS has no value. Nobody legitimately sells it.',
              'Every official Enclave service lives under xdevschain.tech.',
              'Dev keys shown in the docs are public. Never send anything of value to them.',
            ].map((t, i) => (
              <div key={t} className="notice" {...rv((i % 2) * 80)}>
                <Warn />
                <span>{t}</span>
              </div>
            ))}
          </div>
        </div>
      </section>

      <Cta />
    </>
  )
}

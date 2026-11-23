import type { Metadata } from 'next'
import Link from 'next/link'
import Hero from '@/components/home/Hero'
import Chapters from '@/components/home/Chapters'
import ScrollText from '@/components/ScrollText'
import Film from '@/components/Film'
import CodeTabs from '@/components/CodeTabs'
import LiveStrip from '@/components/LiveStrip'
import CountUp from '@/components/CountUp'
import RoadmapRail from '@/components/RoadmapRail'
import { Laptop } from '@/components/Devices'
import { AppsGrid, Cta } from '@/components/Sections'
import { Arrow, Bolt, Bridge, Check, Cube, Gauge, Key, Layers, Lock, Parallel } from '@/components/icons'
import { SNIPPETS } from '@/lib/snippets'
import { LINKS, docs } from '@/lib/site'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: { absolute: 'xDEVS Chain: the chain that has your back' },
  alternates: { canonical: '/' },
}

const ENGINE = [
  {
    icon: <Lock />,
    t: 'BFT consensus, one-block finality',
    b: 'Malachite (Tendermint-style) with execute-before-vote and stored commit certificates. One of four validators down: the chain keeps going.',
  },
  {
    icon: <Parallel />,
    t: 'Block-STM parallel execution',
    b: 'Transactions run in parallel and give the same result as running them one by one, checked by differential tests. Live on every Enclave validator.',
  },
  {
    icon: <Gauge />,
    t: 'Local fee markets',
    b: 'A contract that floods a block gets its own base fee. Everyone else keeps paying the normal fee. Live since block 97,200.',
  },
  {
    icon: <Layers />,
    t: 'Staking-driven validator set',
    b: 'Epochs, block rewards, and slashing for downtime and double-signing, read from the Staking contract since block 97,200.',
  },
  {
    icon: <Cube />,
    t: 'Proposer rotation',
    b: 'Block proposals rotate across the validator set, live on Enclave since block 225,000.',
  },
  {
    icon: <Bolt />,
    t: 'Rust, revm and alloy',
    b: 'An Ethereum-exact node written in Rust: same hashes, receipts and state roots, Osaka rules, EIP-1559 and EIP-7702.',
  },
]

export default function Home() {
  return (
    <>
      <Hero />

      <section className="section is-tight" aria-label="The launch film">
        <div className="wrap">
          <div {...rv(0, 'scale')}>
            <Film />
          </div>
        </div>
      </section>

      <section className="section" aria-label="Why xDEVS Chain">
        <div className="wrap">
          <p className="eyebrow" style={{ marginBottom: 28 }}>
            Why it exists
          </p>
          <ScrollText text="Most chains compete on speed and fees. People lose money another way: a lost seed phrase, a drained wallet, an approval they forgot, an address that looks almost right. xDEVS Chain moves the protection *into *the *chain." />
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="safety-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              The safety layer
            </p>
            <h2 id="safety-title" className="h2" {...rv(60)}>
              <span className="metal">Protection, built into the chain.</span> <span className="quiet">Opt-in, and no admin key.</span>
            </h2>
          </header>
          <Chapters />
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="eoa-title">
        <div className="wrap">
          <div className="split is-flip">
            <div {...rv(0, 'scale')}>
              <Laptop src="/img/desk-wallet-eoalock.webp" alt="The EOA lock page in the wallet demo: delegate, turn on the vault, lock the old key" />
            </div>
            <div className="copy">
              <p className="eyebrow" {...rv()}>
                Already on MetaMask
              </p>
              <h2 id="eoa-title" className="h2" {...rv(60)}>
                Keep your address. <span className="quiet">Lock the old key.</span>
              </h2>
              <p className="lead" {...rv(120)}>
                Point your existing address at the xDEVS smart account with one EIP-7702 transaction and keep its history and
                balance. The EOA lock (XIP-0001) is a node rule: once it’s on, the old private key can’t go around the vault.
              </p>
              <p {...rv(160)}>
                <a className="text-cta" href={docs('/concepts/eoa-lock')}>
                  How the EOA lock works <Arrow />
                </a>
              </p>
            </div>
          </div>
        </div>
      </section>

      <section className="section" aria-labelledby="dev-title">
        <div className="wrap">
          <div className="split">
            <div className="copy">
              <p className="eyebrow" {...rv()}>
                For developers
              </p>
              <h2 id="dev-title" className="h2" {...rv(60)}>
                <span className="metal">If it runs on Ethereum,</span> <span className="quiet">it runs here.</span>
              </h2>
              <p className="lead" {...rv(120)}>
                xDEVS Chain is EVM-equivalent: revm under Osaka rules and the standard JSON-RPC. Solidity, MetaMask, Foundry,
                Hardhat, viem and ethers work without a fork or a plugin.
              </p>
              <ul className="checks" {...rv(160)}>
                <li>
                  <Check /> Chain id 717171, 2-second blocks, final in one
                </li>
                <li>
                  <Check /> Safety methods next to eth_: simulate, check a recipient, list approvals
                </li>
                <li>
                  <Check /> Passkey smart accounts through an ERC-4337 bundler
                </li>
              </ul>
              <div className="btns" {...rv(200)}>
                <Link className="btn is-primary" href="/developers">
                  Developer overview <Arrow />
                </Link>
                <a className="btn is-ghost" href={docs('/guide/quickstart')}>
                  Quickstart
                </a>
              </div>
            </div>
            <div {...rv(120, 'scale')}>
              <CodeTabs snippets={SNIPPETS} />
            </div>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="live-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Right now
            </p>
            <h2 id="live-title" className="h2" {...rv(60)}>
              <span className="metal">Enclave is running.</span> <span className="quiet">These numbers are live.</span>
            </h2>
          </header>
          <div {...rv(100)}>
            <LiveStrip />
          </div>
        </div>
      </section>

      <section className="section" aria-labelledby="engine-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Under the hood
            </p>
            <h2 id="engine-title" className="h2" {...rv(60)}>
              <span className="metal">A careful Layer 1,</span> <span className="quiet">not a fast-and-loose one.</span>
            </h2>
            <p className="lead" {...rv(120)}>
              Every feature below is live on Enclave. Where something is built but not switched on, we say so.
            </p>
          </header>
          <div className="grid-3">
            {ENGINE.map((e, i) => (
              <div key={e.t} className="card feature glow" {...rv((i % 3) * 80)}>
                <span className="icon">{e.icon}</span>
                <h3 className="h4">{e.t}</h3>
                <p>{e.b}</p>
              </div>
            ))}
          </div>

          <div className="grid-2" style={{ marginTop: 16 }}>
            <div className="card feature glow" {...rv()}>
              <span className="icon">
                <Key />
              </span>
              <h3 className="h4">
                Encrypted mempool <span className="chip is-off" style={{ marginLeft: 8 }}>Built, switched off</span>
              </h3>
              <p>
                Transactions stay encrypted until their order is final, so bots can’t sandwich them. A test shows it stopping a
                sandwich attack. It turns on after a distributed key setup and an external review.
              </p>
            </div>
            <div className="card feature glow" {...rv(80)}>
              <span className="icon">
                <Bridge />
              </span>
              <h3 className="h4">
                Light-client bridge <span className="chip is-off" style={{ marginLeft: 8 }}>Prototype</span>
              </h3>
              <p>
                No multisig bridges. A two-way light-client bridge with Ethereum Sepolia is built and tested on real data. It is
                not deployed and waits for an external audit.
              </p>
            </div>
          </div>

          <div style={{ marginTop: 'clamp(72px, 9vw, 120px)' }}>
            <p className="eyebrow" {...rv()}>
              Measured, not promised
            </p>
            <p className="lead" style={{ marginTop: 14, marginBottom: 28 }} {...rv(60)}>
              A 5-hour soak test of a 4-validator network under load.
            </p>
            <dl className="measured" {...rv(100)}>
              <div>
                <dt>
                  <CountUp to={5} suffix="h" />
                </dt>
                <dd>Soak test under load</dd>
              </div>
              <div>
                <dt>
                  <CountUp to={4} />
                </dt>
                <dd>Validators, BFT consensus</dd>
              </div>
              <div>
                <dt>
                  <CountUp to={360000} />
                </dt>
                <dd>Transactions</dd>
              </div>
              <div>
                <dt className="teal">0</dt>
                <dd>Forks</dd>
              </div>
            </dl>
          </div>
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="apps-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Everything on Enclave
            </p>
            <h2 id="apps-title" className="h2" {...rv(60)}>
              <span className="metal">A whole network,</span> <span className="quiet">ready to use today.</span>
            </h2>
          </header>
          <AppsGrid />
        </div>
      </section>

      <section className="section" aria-labelledby="road-title">
        <div className="wrap">
          <div className="split" style={{ alignItems: 'start' }}>
            <header className="copy road-head">
              <p className="eyebrow" {...rv()}>
                Roadmap
              </p>
              <h2 id="road-title" className="h2" {...rv(60)}>
                <span className="metal">Built in phases.</span> <span className="quiet">Shipped in the open.</span>
              </h2>
              <p className="lead" {...rv(120)}>
                Phases 0 to 5 are built and most of it runs on Enclave. Independent validators, audits and mainnet come next. We
                don’t promise dates.
              </p>
              <p {...rv(160)}>
                <Link className="text-cta" href="/roadmap">
                  The full roadmap <Arrow />
                </Link>
              </p>
            </header>
            <RoadmapRail />
          </div>
        </div>
      </section>

      <Cta />
    </>
  )
}

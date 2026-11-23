import type { Metadata } from 'next'
import LiveStrip from '@/components/LiveStrip'
import { AddNetworkButton, CopyButton } from '@/components/Buttons'
import { Browser } from '@/components/Devices'
import { Cta, PageHead } from '@/components/Sections'
import { Arrow, Warn } from '@/components/icons'
import { ENCLAVE, LINKS, docs } from '@/lib/site'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: 'Enclave public testnet',
  description:
    'Enclave is the public xDEVS Chain testnet: chain id 717171, tXDS (no value), 2-second blocks with one-block finality, 4 validators. Add it to MetaMask, get tXDS and deploy.',
  alternates: { canonical: '/enclave' },
}

const PARAMS: { k: string; v: string; note?: string; href?: string; copy?: boolean }[] = [
  { k: 'Network name', v: ENCLAVE.name, copy: true },
  { k: 'Chain id', v: `${ENCLAVE.chainId}`, note: `${ENCLAVE.chainIdHex} in hex`, copy: true },
  { k: 'Currency', v: 'tXDS', note: 'EnclaveXDS, 18 decimals. No value.', copy: true },
  { k: 'RPC URL', v: ENCLAVE.rpcUrl, href: LINKS.rpcInfo, copy: true },
  { k: 'WebSocket', v: ENCLAVE.wsUrl, note: 'newHeads, logs, newPendingTransactions, vaultEvents', copy: true },
  { k: 'Block explorer', v: ENCLAVE.explorerUrl, href: ENCLAVE.explorerUrl, copy: true },
]

const ENDPOINTS: { k: string; v: string; href: string; note: string }[] = [
  { k: 'Wallet', v: 'wallet.xdevschain.tech', href: LINKS.wallet, note: 'Passkey accounts with sponsored activation' },
  { k: 'Explorer', v: 'explorer.xdevschain.tech', href: LINKS.explorer, note: 'Blocks, transactions, validators, names' },
  { k: 'Explorer API', v: 'explorer-api.xdevschain.tech', href: `${ENCLAVE.explorerApiUrl}/api/v1/stats`, note: 'REST, /api/v1' },
  { k: 'Faucet', v: 'faucet.xdevschain.tech', href: LINKS.faucet, note: '100 tXDS per claim, once every 24 hours' },
  { k: 'Bundler', v: 'bundler.xdevschain.tech', href: `${ENCLAVE.bundlerUrl}/health`, note: 'ERC-4337, sponsored account activation' },
  { k: 'Status', v: 'status.xdevschain.tech', href: LINKS.status, note: 'Health of every endpoint' },
]

const FACTS = [
  ['Live since', '17 Nov 2026'],
  ['Validators', '4, operated by the project'],
  ['Where', 'Oracle Cloud, Hyderabad and Mumbai'],
  ['Consensus', 'Malachite BFT, one-block finality'],
  ['Block time', '2 seconds'],
  ['Staking-driven validator set', 'since block 97,200'],
  ['Proposer rotation', 'since block 225,000'],
  ['Release', 'v0.3.1-enclave.2'],
]

export default function EnclavePage() {
  return (
    <>
      <section className="page-top" aria-labelledby="enclave-title">
        <div className="wrap">
          <div className="hero-split">
            <div className="hero-copy">
              <PageHead
                eyebrow="Public testnet"
                title={
                  <span id="enclave-title">
                    <span className="metal">Enclave.</span>
                    <br />
                    <span className="quiet">Try every protection today.</span>
                  </span>
                }
                lead="Enclave is the public xDEVS Chain testnet. It runs the full safety layer, a wallet, an explorer, a faucet and a bundler. Its coin, tXDS, has no value, so you can experiment freely."
              >
                <div className="btns" style={{ marginTop: 8 }}>
                  <AddNetworkButton />
                  <a className="btn is-ghost" href={LINKS.faucet}>
                    Get tXDS
                  </a>
                </div>
              </PageHead>
            </div>
            <div {...rv(200, 'scale')}>
              <Browser src="/img/desk-validators.webp" alt="The Enclave validators page in the explorer: four active validators" url="explorer.xdevschain.tech/validators" />
            </div>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-label="Live network">
        <div className="wrap" {...rv()}>
          <LiveStrip />
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="start-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Get started
            </p>
            <h2 id="start-title" className="h2" {...rv(60)}>
              Three steps, <span className="quiet">a few minutes.</span>
            </h2>
          </header>
          <ol className="steps">
            <li className="card step" {...rv()}>
              <h3 className="h4">Add Enclave to your wallet</h3>
              <p>One click for MetaMask or any wallet that supports EIP-3085. Or create a passkey account and skip the extension.</p>
              <AddNetworkButton className="btn is-ghost is-sm" />
            </li>
            <li className="card step" {...rv(80)}>
              <h3 className="h4">Claim test coins</h3>
              <p>The faucet sends 100 tXDS per claim, once every 24 hours per address. A short human check keeps bots out.</p>
              <a className="text-cta" href={LINKS.faucet}>
                Open the faucet <Arrow />
              </a>
            </li>
            <li className="card step" {...rv(160)}>
              <h3 className="h4">Build or try the wallet</h3>
              <p>Deploy a contract with Foundry or Hardhat, or set up guardians and the vault in the wallet.</p>
              <a className="text-cta" href={docs('/guide/quickstart')}>
                Read the quickstart <Arrow />
              </a>
            </li>
          </ol>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="params-title">
        <div className="wrap">
          <div className="split" style={{ alignItems: 'start' }}>
            <div className="copy">
              <p className="eyebrow" {...rv()}>
                Network parameters
              </p>
              <h2 id="params-title" className="h2" {...rv(60)}>
                Add it by hand.
              </h2>
              <p className="lead" {...rv(120)}>
                Every wallet and tool needs the same five values. Opening the RPC URL in a browser shows its info page.
              </p>
              <div className="notice" {...rv(160)}>
                <Warn />
                <span>
                  Enclave is a test network run by the project. It may be reset with notice on the status page. tXDS has no value,
                  nobody legitimately sells it, and it never converts to mainnet XDS.
                </span>
              </div>
            </div>
            <dl className="params" {...rv(100)}>
              {PARAMS.map((p) => (
                <div className="param" key={p.k}>
                  <dt>{p.k}</dt>
                  <dd>
                    {p.href ? <a href={p.href}>{p.v}</a> : p.v}
                    {p.note ? <span className="note">{p.note}</span> : null}
                  </dd>
                  {p.copy ? <CopyButton value={p.v} label={`Copy ${p.k}`} /> : <span />}
                </div>
              ))}
            </dl>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="services-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Services
            </p>
            <h2 id="services-title" className="h2" {...rv(60)}>
              Everything lives under xdevschain.tech.
            </h2>
            <p className="lead" {...rv(120)}>
              If a site claims to be an official Enclave service and isn’t on this domain, it isn’t one.
            </p>
          </header>
          <dl className="params" {...rv()}>
            {ENDPOINTS.map((e) => (
              <div className="param" key={e.k}>
                <dt>{e.k}</dt>
                <dd>
                  <a href={e.href}>{e.v}</a>
                  <span className="note">{e.note}</span>
                </dd>
                <span className="chip is-live">
                  <span className="dot" /> Live
                </span>
              </div>
            ))}
          </dl>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="facts-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              The network
            </p>
            <h2 id="facts-title" className="h2" {...rv(60)}>
              Small, honest, <span className="quiet">and running.</span>
            </h2>
            <p className="lead" {...rv(120)}>
              Four validators run by the project, not a decentralized network yet. Independent community validators are the next
              phase.
            </p>
          </header>
          <dl className="grid-4">
            {FACTS.map(([k, v], i) => (
              <div key={k} className="card" {...rv((i % 4) * 60)}>
                <dt className="small">{k}</dt>
                <dd className="h4" style={{ marginTop: 10 }}>
                  {v}
                </dd>
              </div>
            ))}
          </dl>
          <p className="small" style={{ marginTop: 24 }} {...rv()}>
            Endpoints and limits in detail: <a className="inline-link" href={docs('/network/endpoints')}>network endpoints</a> ·{' '}
            <a className="inline-link" href={docs('/developers/chain-ids')}>chain ids and parameters</a>
          </p>
        </div>
      </section>

      <Cta title="Your first block is two seconds away." />
    </>
  )
}

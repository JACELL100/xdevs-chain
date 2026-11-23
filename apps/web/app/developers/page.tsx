import type { Metadata } from 'next'
import CodeTabs from '@/components/CodeTabs'
import { AddNetworkButton } from '@/components/Buttons'
import { Cta, PageHead } from '@/components/Sections'
import { Arrow, Book, Code, Cube, Plug, Terminal } from '@/components/icons'
import { SNIPPETS } from '@/lib/snippets'
import { ENCLAVE, LINKS, docs } from '@/lib/site'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: 'Developers',
  description:
    'Build on xDEVS Chain with the tools you already use: Solidity, Foundry, Hardhat, viem and MetaMask. EVM-equivalent, chain id 717171, plus safety RPCs for simulation and recipient checks.',
  alternates: { canonical: '/developers' },
}

const TOOLS = ['Solidity', 'Foundry', 'Hardhat', 'viem', 'ethers', 'MetaMask', 'revm', 'alloy', 'ERC-4337', 'EIP-1559', 'EIP-7702', 'Multicall3', 'Permit2Expiring', 'WebSocket subscriptions']

const RPCS: [string, string][] = [
  ['xdevs_simulateTransaction', 'Runs a transaction against current state and returns asset changes, approvals it sets, the Guardian Vault outcome and plain-language warnings. Commits nothing.'],
  ['xdevs_checkRecipient', 'Facts about a recipient (contract, smart account, EOA lock, .xdevs name) kept apart from advice: first-time, look-alike and poisoning signals.'],
  ['xdevs_getApprovals', 'Live token approvals for an owner, for a revoke screen.'],
  ['xdevs_getVaultQueue', 'Pending Guardian Vault actions for an account.'],
  ['xdevs_resolveName · xdevs_reverseName', 'XNS lookups in both directions.'],
  ['xdevs_localFee', 'The global and the local base fee for transactions to a contract (local fee markets).'],
  ['xdevs_getCommitCertificate', 'The signatures that committed a block: proof of finality you can verify.'],
  ['eth_subscribe("vaultEvents")', 'Real-time Guardian Vault alerts over WebSocket.'],
]

const DOCS = [
  { icon: <Terminal />, t: 'Quickstart', b: 'From zero to a deployed contract on Enclave.', href: docs('/guide/quickstart') },
  { icon: <Code />, t: 'Deploying contracts', b: 'Foundry, Hardhat, viem and forge script.', href: docs('/developers/deploy-contracts') },
  { icon: <Plug />, t: 'JSON-RPC reference', b: 'Every eth_, net_, web3_ and xdevs_ method.', href: docs('/developers/json-rpc') },
  { icon: <Book />, t: 'TypeScript SDK', b: 'viem chains, typed xdevs_ actions, name helpers.', href: docs('/developers/sdk') },
  { icon: <Cube />, t: 'Predeployed contracts', b: 'System contracts at fixed addresses in genesis.', href: docs('/concepts/predeploys') },
  { icon: <Terminal />, t: 'Run a node', b: 'Full node, local 4-validator network, configuration.', href: docs('/operators/run-a-node') },
]

export default function DevelopersPage() {
  return (
    <>
      <section className="page-top" aria-labelledby="dev-title">
        <div className="wrap">
          <div className="hero-split">
            <div className="hero-copy">
              <PageHead
                eyebrow="Developers"
                title={
                  <span id="dev-title">
                    <span className="metal">Your tools.</span>
                    <br />
                    <span className="quiet">Safer defaults.</span>
                  </span>
                }
                lead="xDEVS Chain is EVM-equivalent, so your contracts, scripts and wallets work as they are. On top, the node answers questions other chains leave to third parties: what will this transaction do, and is this the address I meant?"
              >
                <div className="btns" style={{ marginTop: 8 }}>
                  <a className="btn is-primary" href={docs('/guide/quickstart')}>
                    Quickstart <Arrow />
                  </a>
                  <AddNetworkButton className="btn is-ghost" />
                </div>
              </PageHead>
            </div>
            <div {...rv(200, 'scale')}>
              <CodeTabs snippets={SNIPPETS} />
            </div>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-label="Compatible tools">
        <div className="marquee" {...rv(0, 'fade')}>
          <div className="marquee-track">
            {TOOLS.map((t) => (
              <span key={t} className="chip">
                {t}
              </span>
            ))}
            <span className="dup" aria-hidden="true" style={{ display: 'contents' }}>
              {TOOLS.map((t) => (
                <span key={`d-${t}`} className="chip">
                  {t}
                </span>
              ))}
            </span>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="compat-title">
        <div className="wrap">
          <div className="grid-3">
            {[
              ['EVM-equivalent', 'revm under the Osaka rules, Ethereum-exact hashes, receipts and state roots. Legacy, EIP-2930, EIP-1559 and EIP-7702 transactions.'],
              ['Familiar JSON-RPC', 'eth_, net_ and web3_ over HTTPS and WebSocket, with state history for the last 10,000 blocks so forks and pinned scripts work.'],
              ['Fast, final blocks', '2-second blocks on Enclave, final in one block. Your app never has to wait for confirmations or handle reorgs.'],
            ].map(([t, b], i) => (
              <div className="card feature glow" key={t} {...rv(i * 80)}>
                <h3 className="h4">{t}</h3>
                <p>{b}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="section" aria-labelledby="rpc-title">
        <div className="wrap">
          <div className="split" style={{ alignItems: 'start' }}>
            <header className="copy road-head">
              <p className="eyebrow" {...rv()}>
                Safety RPCs
              </p>
              <h2 id="rpc-title" className="h2" {...rv(60)}>
                The node <span className="quiet">has opinions.</span>
              </h2>
              <p className="lead" {...rv(120)}>
                Built into the node and served by the public RPC at <span className="mono">{ENCLAVE.rpcUrl.replace('https://', '')}</span>. The
                wallet and explorer use them; your app can too.
              </p>
              <p {...rv(160)}>
                <a className="text-cta" href={docs('/developers/json-rpc')}>
                  JSON-RPC reference <Arrow />
                </a>
              </p>
            </header>
            <ul className="rpc-list">
              {RPCS.map(([m, d], i) => (
                <li key={m} {...rv(i * 40)}>
                  <code>{m}</code>
                  <p>{d}</p>
                </li>
              ))}
            </ul>
          </div>
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="docs-title">
        <div className="wrap">
          <header className="head">
            <p className="eyebrow" {...rv()}>
              Documentation
            </p>
            <h2 id="docs-title" className="h2" {...rv(60)}>
              Go deeper <span className="quiet">at docs.xdevschain.tech.</span>
            </h2>
          </header>
          <div className="grid-3">
            {DOCS.map((d, i) => (
              <a key={d.t} href={d.href} className="card feature glow" {...rv((i % 3) * 70)}>
                <span className="icon">{d.icon}</span>
                <h3 className="h4">{d.t}</h3>
                <p>{d.b}</p>
              </a>
            ))}
          </div>
          <p className="small" style={{ marginTop: 24 }} {...rv()}>
            The node, the system contracts, the SDK and every app are open source under Apache-2.0 on{' '}
            <a className="inline-link" href={LINKS.github}>
              GitHub
            </a>
            .
          </p>
        </div>
      </section>

      <Cta title="Ship something to Enclave." body="Add the network, take some tXDS from the faucet and deploy with the tools you already have." />
    </>
  )
}

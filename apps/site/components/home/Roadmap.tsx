import Link from 'next/link'
import { rv } from './util'
import './roadmap.css'

type Status = 'built' | 'next' | 'planned'
const PHASES: { n: string; h: string; p: string; s: Status }[] = [
  { n: 'P1', h: 'Single-node chain', p: 'EVM execution, JSON-RPC, MetaMask and Foundry.', s: 'built' },
  { n: 'P2', h: 'BFT network', p: 'Malachite consensus, block sync, gossip, WebSockets.', s: 'built' },
  { n: 'P3', h: 'Proof of Stake devnet', p: 'Staking-driven validators, public devnet, faucet, explorer.', s: 'next' },
  { n: 'P4', h: 'Safety layer RPCs', p: 'Simulation and recipient checks, reference wallet. Contracts already built.', s: 'planned' },
  { n: 'P5', h: 'Performance & MEV', p: 'Encrypted mempool, parallel execution, local fee markets.', s: 'planned' },
  { n: 'P6', h: 'Public testnet', p: 'Long-lived testnet, independent validators, bug bounty.', s: 'planned' },
  { n: 'P7', h: 'Interop', p: 'Light-client bridges only, audits, mainnet preparation.', s: 'planned' },
]
const LABEL: Record<Status, string> = { built: 'Built', next: 'Next', planned: 'Planned' }

export default function Roadmap() {
  return (
    <section id="roadmap" className="xh-section s-road" aria-labelledby="roadmap-title">
      <div className="xh-wrap">
        <header className="head">
          <div>
            <p className="xh-label" {...rv()}>
              <b>07</b>
              <i>//</i> roadmap
            </p>
            <h2 id="roadmap-title" className="xh-h2" {...rv(60)}>
              <span className="xh-metal">Built in the open.</span>
              <br />
              <span className="xh-quiet">Shipped in phases.</span>
            </h2>
          </div>
          <p className="xh-body side" {...rv(120)}>
            Phases 1 and 2 are built and tested. The public testnet is not live yet; it comes with Phase 3. Dates aren’t
            promised. Progress is.
            <Link className="xh-textlink" href="/roadmap">
              Full roadmap
            </Link>
          </p>
        </header>

        <ol className="line" {...rv(80)}>
          {PHASES.map((ph) => (
            <li key={ph.n} className={ph.s}>
              <span className="node" aria-hidden="true" />
              <span className="n xh-mono">{ph.n}</span>
              <h3>{ph.h}</h3>
              <p>{ph.p}</p>
              <span className={`xh-chip${ph.s === 'built' ? ' is-final' : ''}`}>{LABEL[ph.s]}</span>
            </li>
          ))}
        </ol>
      </div>
    </section>
  )
}

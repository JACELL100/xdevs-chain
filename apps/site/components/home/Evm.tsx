import Link from 'next/link'
import CodePanel from './CodePanel'
import { rv } from './util'
import './evm.css'

const TOOLS = ['Solidity', 'Foundry', 'Hardhat', 'viem', 'ethers', 'MetaMask']

export default function Evm() {
  return (
    <section id="evm" className="xh-section s-evm" aria-labelledby="evm-title">
      <div className="xh-wrap grid">
        <div className="copy">
          <p className="xh-label" {...rv()}>
            <b>05</b>
            <i>//</i> evm-equivalent
          </p>
          <h2 id="evm-title" className="xh-h2" {...rv(60)}>
            <span className="xh-metal">Your tools.</span>
            <br />
            <span className="xh-quiet">Unchanged.</span>
          </h2>
          <p className="xh-lede" {...rv(120)}>
            revm with Ethereum’s Osaka rules. Hashes, receipts and state roots are computed the Ethereum way. If it runs
            on Ethereum, it runs here.
          </p>
          <ul className="tools xh-mono" aria-label="Works with" {...rv(180)}>
            {TOOLS.map((t) => (
              <li key={t}>{t}</li>
            ))}
          </ul>
          <p className="eips xh-mono" {...rv(220)}>
            EIP-1559 · EIP-2930 · EIP-7702 · P256VERIFY
          </p>
          <Link className="xh-textlink" href="/developers/deploy-contracts" {...rv(260)}>
            Deploy a contract
          </Link>
        </div>
        <CodePanel />
      </div>
    </section>
  )
}

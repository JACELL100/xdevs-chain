import Link from 'next/link'
import VaultStory from './VaultStory'
import { rv } from './util'
import './vault.css'

export default function Vault() {
  return (
    <section id="vault" className="xh-section vault" aria-labelledby="vault-title">
      <div className="xh-wrap intro">
        <p className="xh-label" {...rv()}>
          <b>02</b>
          <i>//</i> guardian vault
        </p>
        <h2 id="vault-title" className="xh-h2" {...rv(60)}>
          <span className="xh-metal">A stolen key is not</span>
          <br />
          <span className="xh-quiet">a drained wallet.</span>
        </h2>
      </div>

      <VaultStory />

      <div className="xh-wrap after" {...rv()}>
        <p className="xh-body">
          Already on MetaMask? <Link className="xh-textlink" href="/concepts/eoa-lock">XIP-0001</Link> lets an existing
          address opt into the same protection. If a thief and the owner both hold keys,{' '}
          <Link className="xh-textlink" href="/concepts/guardian-vault">
            guardian freeze
          </Link>{' '}
          breaks the stalemate.
        </p>
      </div>
    </section>
  )
}

'use client'
// BFT commit certificate: which validators precommitted the block, weighted by voting power, against the 2/3 line.

import { CopyButton } from '@xdevs/ui'
import { commitView, type CommitView } from '../lib/chainstats'
import { shortAddress } from '../lib/format'
import { paths } from '../lib/routes'
import type { CommitDetail, ValidatorSet } from '../lib/types'
import { Pill } from './bits'

export function CommitCertificate({ commit, set, signedCount }: { commit: CommitDetail | null; set: ValidatorSet | null; signedCount?: number }) {
  if (!commit) {
    return (
      <div className="x-cert is-empty">
        <p className="x-cert-head">
          <Pill tone="neutral">no certificate</Pill>
          <span className="x-muted">
            {signedCount !== undefined && signedCount > 0
              ? `The indexer saw ${signedCount} signatures, but the node did not return the certificate itself.`
              : 'The node did not return a commit certificate for this height (genesis, a single-node dev chain, or xdevs_getCommitCertificate is not served).'}
          </span>
        </p>
      </div>
    )
  }
  const members = (set?.validators ?? [])
    .filter((v) => v.inSigningSet || v.status === 'active')
    .map((v) => ({ operator: v.operator, consensusKey: v.consensusKey, votingPower: BigInt(v.votingPower ?? '0x0'), name: v.name }))
  const view: CommitView = commitView(commit.signatures, members)
  const signed = view.votes.filter((v) => v.signed).length
  const pct = view.totalPower > 0n ? view.signedBps / 100 : null
  return (
    <div className="x-cert">
      <div className="x-cert-head">
        <span className="x-final-mark" aria-hidden="true">
          <svg viewBox="0 0 16 16" width="14" height="14">
            <path d="M3.5 8.5 6.5 11.5 12.5 4.5" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </span>
        <div>
          <p className="x-cert-title">
            Final. Signed by {signed} of {members.length || signed} validators
            {pct !== null ? <span className="x-muted"> · {pct.toFixed(pct === 100 ? 0 : 1)}% of voting power</span> : null}
          </p>
          <p className="x-cert-sub">
            Round {commit.round} · {view.quorum === false ? 'below' : 'above'} the 2/3 threshold · cannot be reverted
          </p>
        </div>
      </div>

      {view.totalPower > 0n ? (
        <div className="x-power" role="img" aria-label={`${pct?.toFixed(1)}% of voting power signed; finality needs more than 66.7%`}>
          <div className="x-power-bar">
            {view.votes
              .filter((v) => v.votingPower > 0n)
              .map((v) => (
                <span
                  key={v.consensusAddress}
                  className={v.signed ? 'is-signed' : 'is-missed'}
                  style={{ flexGrow: Number((v.votingPower * 10000n) / view.totalPower) }}
                  title={`${v.name ?? shortAddress(v.operator)}: ${v.signed ? 'signed' : 'missed'}`}
                />
              ))}
          </div>
          <span className="x-power-threshold" style={{ left: '66.667%' }}>
            <span>2/3</span>
          </span>
        </div>
      ) : null}

      <ul className="x-votes" aria-label="Validator precommits">
        {view.votes.map((v) => {
          const share = view.totalPower > 0n ? Number((v.votingPower * 10000n) / view.totalPower) / 100 : null
          return (
            <li key={v.consensusAddress} className={v.signed ? 'is-signed' : 'is-missed'}>
              <span className="x-vote-dot" aria-hidden="true" />
              <span className="x-vote-name">
                {v.operator ? (
                  <a className="x-link" href={paths.address(v.operator)}>
                    {v.name ?? shortAddress(v.operator)}
                  </a>
                ) : (
                  <span className="x-mono" title="Not in the current validator set">
                    {shortAddress(v.consensusAddress)}
                  </span>
                )}
              </span>
              <span className="x-vote-power x-num">{share !== null ? `${share.toFixed(1)}%` : ''}</span>
              <span className="x-vote-state">{v.signed ? 'signed' : 'missed'}</span>
              {v.signature ? (
                <span className="x-vote-sig">
                  <code className="x-mono" title={v.signature}>
                    {v.signature.slice(0, 10)}…
                  </code>
                  <CopyButton value={v.signature} title="Copy signature" />
                </span>
              ) : (
                <span className="x-vote-sig" />
              )}
            </li>
          )
        })}
      </ul>
      {!set ? <p className="x-small x-muted">Validator set unavailable: showing signer addresses only.</p> : null}
    </div>
  )
}

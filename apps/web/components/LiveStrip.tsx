'use client'

/** Live Enclave numbers, read from the public RPC in the visitor's browser. */
import { AnimatePresence, m } from 'motion/react'
import { fmtInt, useChain } from '@/lib/chain'
import { LINKS } from '@/lib/site'

function Ticker({ value }: { value: string }) {
  // The number rolls up when it changes (transform/opacity only).
  return (
    <span style={{ display: 'inline-block', position: 'relative', overflow: 'hidden', verticalAlign: 'bottom' }}>
      <AnimatePresence mode="popLayout" initial={false}>
        <m.span
          key={value}
          style={{ display: 'inline-block' }}
          initial={{ y: '60%', opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: '-60%', opacity: 0 }}
          transition={{ duration: 0.45, ease: [0.2, 0.8, 0.2, 1] }}
        >
          {value}
        </m.span>
      </AnimatePresence>
    </span>
  )
}

export default function LiveStrip() {
  const c = useChain()
  const live = c.status === 'live'
  return (
    <div className="live">
      <div className="live-top">
        <p className="eyebrow">
          <span className={`chip ${live ? 'is-live' : c.status === 'offline' ? 'is-warn' : 'is-off'}`}>
            <span className={`dot${live ? ' is-pulse' : ''}`} />
            {live ? 'Live' : c.status === 'offline' ? 'Unreachable' : 'Connecting'}
          </span>
          Enclave public testnet
        </p>
        <a className="text-cta" href={LINKS.status}>
          Full network status
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
            <path d="M5 12h14M13 6l6 6-6 6" />
          </svg>
        </a>
      </div>
      <dl className="live-grid" aria-live="off">
        <div className="stat">
          <dt>Latest block</dt>
          <dd>{live && c.height ? <Ticker value={fmtInt(c.height)} /> : '—'}</dd>
          <p>Every one of them final.</p>
        </div>
        <div className="stat">
          <dt>Block time</dt>
          <dd>
            {c.blockTimeMs ? (c.blockTimeMs / 1000).toFixed(2) : '—'}
            <small>s</small>
          </dd>
          <p>Average of the last 60 blocks.</p>
        </div>
        <div className="stat">
          <dt>Finality</dt>
          <dd>
            1<small>block</small>
          </dd>
          <p>BFT, no reorgs after commit.</p>
        </div>
        <div className="stat">
          <dt>Validators</dt>
          <dd>4</dd>
          <p>Operated by the project.</p>
        </div>
        <div className="stat">
          <dt>Chain id</dt>
          <dd>717171</dd>
          <p>tXDS has no value.</p>
        </div>
      </dl>
      <div className="live-foot">
        <span>
          Read directly from <span className="mono">rpc.xdevschain.tech</span> by your browser.
          {c.totalTransactions != null ? ` Explorer index: ${fmtInt(c.totalTransactions)} transactions.` : ''}
        </span>
        <span>Hyderabad and Mumbai · Oracle Cloud</span>
      </div>
    </div>
  )
}

'use client'

/**
 * Phone screens drawn in code for the two chapters that have no wallet screenshot:
 * the XNS look-alike rule and one-block finality (with live Enclave blocks when reachable).
 */
import { AnimatePresence, m } from 'motion/react'
import { Check, Warn } from '@/components/icons'
import { fmtInt, useChain } from '@/lib/chain'

export function NamesScreen() {
  return (
    <div className="ui" role="img" aria-label="Illustration: the wallet refuses to register a1ice.xdevs because it looks like alice.xdevs">
      <div>
        <p className="ui-label">xns · illustration</p>
        <p className="ui-title">Claim a name</p>
        <p className="ui-sub">A name people can send to instead of a long address.</p>
      </div>
      <div className="ui-card">
        <p className="ui-label">claim a name</p>
        <div className="ui-input is-bad">
          a1ice<span>.xdevs</span>
        </div>
        <p className="ui-msg is-bad">
          <Warn />
          Refused: looks like alice.xdevs, which already exists.
        </p>
      </div>
      <div className="ui-card">
        <p className="ui-label">treated as the same name</p>
        {[
          ['a1ice', 'alice'],
          ['n00r', 'noor'],
          ['rnila', 'mila'],
          ['vvallet', 'wallet'],
        ].map(([a, b]) => (
          <div className="ui-row" key={a}>
            <code>
              <s>{a}</s>
            </code>
            <span className="small">→</span>
            <code>{b}</code>
          </div>
        ))}
      </div>
      <div className="ui-card">
        <div className="ui-input">
          aria<span>.xdevs</span>
        </div>
        <p className="ui-msg is-ok">
          <Check />
          Unique spelling. Free on the testnet.
        </p>
      </div>
    </div>
  )
}

export function FinalityScreen() {
  const { blocks, status, blockTimeMs } = useChain()
  const live = status === 'live' && blocks.length > 0
  return (
    <div className="ui" role="img" aria-label="Live Enclave blocks, each marked final as soon as it is committed">
      <div>
        <p className="ui-label">{live ? 'enclave · live' : 'enclave'}</p>
        <p className="ui-title">Final in one block</p>
      </div>
      <div className="ui-card" style={{ display: 'flex', alignItems: 'end', justifyContent: 'space-between' }}>
        <div>
          <p className="ui-label">block time</p>
          <p className="big-num">
            {blockTimeMs ? (blockTimeMs / 1000).toFixed(1) : '2'}
            <span style={{ fontSize: 18, color: 'var(--muted)', marginLeft: 4 }}>s</span>
          </p>
        </div>
        <div style={{ textAlign: 'right' }}>
          <p className="ui-label">confirmations</p>
          <p className="big-num">1</p>
        </div>
      </div>
      <div style={{ display: 'grid', gap: 8 }}>
        <AnimatePresence initial={false} mode="popLayout">
          {(live ? blocks.slice(0, 6) : []).map((b, i) => (
            <m.div
              layout
              key={b.number}
              className="blk-mini"
              initial={{ opacity: 0, y: -16 }}
              animate={{ opacity: 1 - i * 0.12, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.6, ease: [0.2, 0.8, 0.2, 1] }}
            >
              <b>#{fmtInt(b.number)}</b>
              <span className="bar">
                <m.i initial={{ scaleX: i === 0 ? 0 : 1 }} animate={{ scaleX: 1 }} transition={{ duration: 0.9, ease: [0.65, 0, 0.35, 1] }} />
              </span>
              <span className="ok">final</span>
            </m.div>
          ))}
        </AnimatePresence>
        {!live
          ? Array.from({ length: 6 }, (_, i) => (
              <div key={i} className="blk-mini" style={{ opacity: 1 - i * 0.12 }}>
                <b>{status === 'offline' ? 'offline' : '…'}</b>
                <span className="bar">
                  <i />
                </span>
                <span className="ok">final</span>
              </div>
            ))
          : null}
      </div>
    </div>
  )
}

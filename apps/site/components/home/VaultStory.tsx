'use client'

/**
 * Scroll-driven story: a stolen key signs a big transfer, the vault queues it,
 * guardians cancel, funds stay put. Progress is tied to the scroll position
 * inside a tall track with a pinned stage. Continuous values (rail, countdown,
 * bar) are written straight to CSS variables / textContent; React only
 * re-renders when the step or the vote count changes.
 * Reduced motion: no pinning, everything shown in its final state.
 */
import { useEffect, useRef, useState } from 'react'
import { Check, ArrowUpRight } from './icons'
import { clamp, reducedMotion } from './motion'

const STEPS = [
  {
    t: '02:14',
    h: 'Someone has your key.',
    p: 'They sign a transfer of 48,000 tXDS. On most chains, that is the end of the story.',
  },
  {
    t: '02:14',
    h: 'Above your limit, so it waits.',
    p: 'Large transfers, approvals and unknown contract calls enter the vault queue for your delay: 24 hours by default.',
  },
  {
    t: '07:40',
    h: 'Your guardians step in.',
    p: 'You can cancel on your own. So can the guardians you chose, once enough of them agree.',
  },
  {
    t: '07:41',
    h: 'Cancelled. Nothing moved.',
    p: 'And the thief can’t just switch the vault off. Turning protection off waits out the same delay.',
  },
]

const GUARDIANS = [
  { n: 'mira.xdevs', i: 'M' },
  { n: 'omar.xdevs', i: 'O' },
  { n: 'hardware key', i: 'H' },
]

const DAY = 24 * 3600
function hms(s: number) {
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const x = Math.floor(s % 60)
  return [h, m, x].map((v) => String(v).padStart(2, '0')).join(':')
}

export default function VaultStory() {
  const track = useRef<HTMLDivElement>(null)
  const stage = useRef<HTMLDivElement>(null)
  const clock = useRef<HTMLElement>(null)
  const [step, setStep] = useState(0)
  const [votes, setVotes] = useState(0)
  const [reduced, setReduced] = useState(false)

  useEffect(() => {
    let lastP = -1
    const apply = (p: number) => {
      if (Math.abs(p - lastP) < 0.0005) return
      lastP = p
      const st = stage.current
      if (!st) return
      st.style.setProperty('--p', p.toFixed(4))
      // the cancel window starts at step 2; scrolling burns about 6 h of it
      const q = clamp((p - 0.25) / 0.5)
      const remaining = DAY - 1 - q * 6 * 3600
      st.style.setProperty('--left', (remaining / DAY).toFixed(4))
      if (clock.current) clock.current.textContent = hms(remaining)
      setStep(p < 0.25 ? 0 : p < 0.5 ? 1 : p < 0.76 ? 2 : 3)
      setVotes(p >= 0.68 ? 2 : p >= 0.58 ? 1 : 0)
    }

    if (reducedMotion()) {
      setReduced(true)
      apply(1)
      return
    }

    let raf = 0
    let ticking = false
    const measure = () => {
      ticking = false
      const el = track.current
      if (!el) return
      const r = el.getBoundingClientRect()
      const total = r.height - window.innerHeight
      apply(clamp(-r.top / Math.max(1, total)))
    }
    const onScroll = () => {
      if (ticking) return
      ticking = true
      raf = requestAnimationFrame(measure)
    }
    measure()
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('resize', onScroll, { passive: true })
    return () => {
      cancelAnimationFrame(raf)
      window.removeEventListener('scroll', onScroll)
      window.removeEventListener('resize', onScroll)
    }
  }, [])

  return (
    <div ref={track} className={`track${reduced ? ' is-static' : ''}`}>
      <div ref={stage} className="sticky" data-step={step}>
        <div className="xh-wrap grid">
          <ol className="steps">
            {STEPS.map((s, i) => (
              <li
                key={i}
                className={`${i === step || reduced ? 'on' : ''}${i < step ? ' past' : ''}`}
                aria-current={i === step ? 'step' : undefined}
              >
                <span className="st-t xh-mono">
                  0{i + 1} <i>//</i> {s.t}
                </span>
                <h3>{s.h}</h3>
                <p>{s.p}</p>
              </li>
            ))}
            <span className="rail" aria-hidden="true">
              <span className="rail-fill" />
            </span>
          </ol>

          <div
            className="device xh-panel"
            role="img"
            aria-label="Wallet view: a 48,000 tXDS transfer signed with a stolen key is queued by the Guardian Vault, cancelled by two of three guardians, and the balance stays 52,410 tXDS."
          >
            <div className="dv-top">
              <span className="avatar">A</span>
              <span className="who">
                <b>alice.xdevs</b>
                <span className="xh-mono">0x4f2c…9a10</span>
              </span>
              <span className="xh-chip vault-chip">
                <span className="xh-dot" />
                Vault on · 24 h
              </span>
            </div>

            <div className="dv-balance">
              <span className="cap">Balance</span>
              <span className="amt xh-num">
                52,410<span className="dec">.00</span> <span className="unit">tXDS</span>
              </span>
              <span className={`safe${step === 3 ? ' show' : ''}`}>
                <Check />
                Unchanged
              </span>
            </div>

            <div className={`tx s${step}`}>
              <div className="tx-row">
                <span className="tx-ico" aria-hidden="true">
                  <ArrowUpRight />
                </span>
                <span className="tx-main">
                  <b>
                    Send <span className="xh-num">48,000</span> tXDS
                  </b>
                  <span className="xh-mono">to 0x9d1e…77b4 · signed by owner key</span>
                </span>
                <span className="tx-state">
                  <span className="xh-chip is-alert st st0">
                    <span className="xh-dot" />
                    Submitted
                  </span>
                  <span className="xh-chip st st1">
                    <span className="xh-dot" />
                    Queued
                  </span>
                  <span className="xh-chip is-final st st3">
                    <span className="xh-dot" />
                    Cancelled
                  </span>
                </span>
              </div>

              <div className="window">
                <div className="win-meta xh-mono">
                  <span>
                    Executes in{' '}
                    <b ref={clock} className="xh-num">
                      23:59:59
                    </b>
                  </span>
                  <span className="win-why">over 1,000 tXDS limit</span>
                </div>
                <div className="bar">
                  <span className="bar-fill" />
                </div>
              </div>

              <div className="guards">
                <div className="g-head xh-mono">
                  <span>Guardians · 2 of 3 to cancel</span>
                  <span>
                    votes <b className="xh-num">{votes}/2</b>
                  </span>
                </div>
                <ul>
                  {GUARDIANS.map((g, i) => (
                    <li key={g.n} className={i < votes ? 'voted' : undefined}>
                      <span className="g-av">{g.i}</span>
                      <span className="g-n">{g.n}</span>
                      <span className="g-s xh-mono">{i < votes ? 'cancel' : 'notified'}</span>
                    </li>
                  ))}
                </ul>
              </div>
            </div>

            <div className={`off-try${step === 3 ? ' show' : ''}`}>
              <span className="xh-mono">setVaultPolicy(enabled: false)</span>
              <span className="xh-chip">
                <span className="xh-dot" />
                Queued · 24 h
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

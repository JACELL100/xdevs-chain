'use client'

import { useEffect, useRef, useState } from 'react'
import { Check, Cross } from './icons'
import { reducedMotion, useInView, wait } from './motion'

/* ---------- an approval draining to zero ---------- */

const SPAN = 30 * 24 * 60 // minutes in the default 30-day duration
const RUN_MS = 5200
function fmtLeft(min: number) {
  const d = Math.floor(min / 1440)
  const h = Math.floor((min % 1440) / 60)
  const m = Math.floor(min % 60)
  return `${d}d ${String(h).padStart(2, '0')}h ${String(m).padStart(2, '0')}m`
}

export function ApprovalWidget() {
  const ref = useRef<HTMLDivElement>(null)
  const left = useRef<HTMLSpanElement>(null)
  const inView = useInView(ref, '0px 0px -10% 0px')
  const [phase, setPhase] = useState<'reset' | 'run' | 'expired'>('expired')

  useEffect(() => {
    if (!inView || reducedMotion()) return
    let alive = true
    let raf = 0
    const setLeft = (min: number) => {
      if (left.current) left.current.textContent = `in ${fmtLeft(min)}`
    }
    ;(async () => {
      while (alive) {
        setPhase('reset')
        setLeft(SPAN - 1)
        await wait(700)
        if (!alive) return
        setPhase('run')
        const t0 = performance.now()
        await new Promise<void>((done) => {
          const tick = (t: number) => {
            if (!alive) return done()
            const k = Math.min(1, (t - t0) / RUN_MS)
            setLeft((SPAN - 1) * (1 - k))
            if (k < 1) raf = requestAnimationFrame(tick)
            else done()
          }
          raf = requestAnimationFrame(tick)
        })
        if (!alive) return
        setPhase('expired')
        await wait(2600)
      }
    })()
    return () => {
      alive = false
      cancelAnimationFrame(raf)
    }
  }, [inView])

  return (
    <div
      ref={ref}
      className={`widget appr is-${phase}`}
      role="img"
      aria-label="An approval of 5,000 USDx counting down from 30 days to zero, then reading as expired."
    >
      <dl className="rows">
        <div>
          <dt>spender</dt>
          <dd className="xh-mono">
            <span>
              0x3fA1…92c0 <span className="tag">dex router</span>
            </span>
          </dd>
        </div>
        <div>
          <dt>allowance</dt>
          <dd className="xh-mono xh-num">
            <span className="amt-live">5,000 USDx</span>
            <span className="amt-zero">0 USDx</span>
          </dd>
        </div>
        <div>
          <dt>expires</dt>
          <dd className="xh-mono xh-num">
            <span className="exp-live" ref={left}>
              in {fmtLeft(SPAN - 1)}
            </span>
            <span className="exp-zero">expired</span>
          </dd>
        </div>
      </dl>
      <div className="drain">
        <span />
      </div>
      <div className="console xh-mono">
        <span>
          <i>›</i> allowance(alice, router)
        </span>
        <span className="out">
          <span className="o-live">5000000000</span>
          <span className="o-zero">0</span>
        </span>
        <span className="err">
          <i>›</i> transferFrom(…) <b>reverted: AllowanceExpired</b>
        </span>
      </div>
    </div>
  )
}

/* ---------- look-alike names ---------- */

type Ex = { q: string; ok: boolean; why: string; detail: string }
const EXAMPLES: Ex[] = [
  { q: 'paypa1', ok: false, why: 'Looks like paypal.xdevs', detail: 'skeleton(paypa1) = paypal' },
  { q: 'g00gle', ok: false, why: 'Looks like google.xdevs', detail: 'skeleton(g00gle) = google' },
  { q: 'pаypal', ok: false, why: 'Non-ASCII character', detail: '“а” is Cyrillic U+0430' },
  { q: 'alice', ok: true, why: 'Available', detail: 'no look-alike registered' },
]

export function NamesWidget() {
  const ref = useRef<HTMLDivElement>(null)
  const inView = useInView(ref, '0px 0px -10% 0px')
  const [typed, setTyped] = useState(EXAMPLES[0].q)
  const [result, setResult] = useState<Ex | null>(EXAMPLES[0])

  useEffect(() => {
    if (!inView || reducedMotion()) return
    let alive = true
    ;(async () => {
      let i = 0
      while (alive) {
        const ex = EXAMPLES[i % EXAMPLES.length]
        setResult(null)
        setTyped('')
        await wait(450)
        for (let c = 1; c <= ex.q.length; c++) {
          if (!alive) return
          setTyped(ex.q.slice(0, c))
          await wait(85 + Math.random() * 50)
        }
        await wait(380)
        if (!alive) return
        setResult(ex)
        await wait(2300)
        i++
      }
    })()
    return () => {
      alive = false
    }
  }, [inView])

  return (
    <div
      ref={ref}
      className="widget names"
      role="img"
      aria-label="Registering paypa1.xdevs, g00gle.xdevs or a Cyrillic look-alike is rejected; alice.xdevs is available."
    >
      <ul className="taken xh-mono">
        <li>
          <span>paypal.xdevs</span>
          <span className="xh-chip">registered</span>
        </li>
        <li>
          <span>google.xdevs</span>
          <span className="xh-chip">registered</span>
        </li>
      </ul>
      <div className="field">
        <span className="xh-mono q">
          {typed}
          <span className="caret" />
        </span>
        <span className="xh-mono suffix">.xdevs</span>
      </div>
      <div className={`verdict${result ? ' show' : ''}${result?.ok ? ' ok' : ''}`}>
        {result && !result.ok ? <Cross /> : <Check />}
        <span className="v-why">{result?.why ?? ' '}</span>
        <span className="v-detail xh-mono">{result?.detail ?? ''}</span>
      </div>
    </div>
  )
}

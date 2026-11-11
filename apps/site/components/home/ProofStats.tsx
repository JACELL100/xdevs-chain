'use client'

/**
 * Measured numbers that count up once when they scroll into view.
 * Server HTML carries the final values; they only reset to 0 if the grid is
 * still below the fold at hydration. Only measured facts go here.
 */
import { useEffect, useRef, useState } from 'react'
import { reducedMotion, useInView } from './motion'
import { vars } from './util'

type Stat = { v: number; unit?: string; label: string; zero?: boolean }
const STATS: Stat[] = [
  { v: 5, unit: 'h', label: 'Soak test under load' },
  { v: 4, label: 'Validators, BFT consensus' },
  { v: 1, unit: 's', label: 'Block time during the soak' },
  { v: 360000, label: 'Transactions' },
  { v: 0, label: 'Forks', zero: true },
  { v: 0, label: 'State-root mismatches', zero: true },
]
const TESTS: Stat[] = [
  { v: 422, label: 'Rust tests' },
  { v: 340, label: 'Contract tests' },
]
const ALL = [...STATS, ...TESTS]
const fmt = (n: number) => n.toLocaleString('en-US')

export default function ProofStats() {
  const ref = useRef<HTMLDListElement>(null)
  const inView = useInView(ref, '0px 0px -15% 0px')
  const [shown, setShown] = useState(ALL.map((s) => s.v))
  const [done, setDone] = useState(true)
  const armed = useRef(false)

  useEffect(() => {
    const r = ref.current?.getBoundingClientRect()
    if (r && r.top > window.innerHeight && !reducedMotion()) {
      armed.current = true
      setDone(false)
      setShown(ALL.map(() => 0))
    }
  }, [])

  useEffect(() => {
    if (!inView || !armed.current) return
    armed.current = false
    let raf = 0
    const t0 = performance.now()
    const D = 1800
    const step = (t: number) => {
      const k = Math.min(1, (t - t0) / D)
      const e = 1 - Math.pow(1 - k, 4)
      setShown(ALL.map((s) => Math.round(s.v * e)))
      if (k < 1) raf = requestAnimationFrame(step)
      else setDone(true)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [inView])

  return (
    <>
      <dl ref={ref} className={`stats${done ? ' done' : ''}`}>
        {STATS.map((s, i) => (
          <div key={s.label} className={`stat${s.zero ? ' zero' : ''}`} data-reveal style={vars({ '--rv-delay': `${i * 60}ms` })}>
            <dt>{s.label}</dt>
            <dd>
              <span className="val xh-num">
                <span className="xh-metal">{fmt(shown[i])}</span>
              </span>
              {s.unit && <span className="u">{s.unit}</span>}
            </dd>
          </div>
        ))}
      </dl>

      <div className="foot">
        <dl className="tests">
          {TESTS.map((s, i) => (
            <div key={s.label} data-reveal style={vars({ '--rv-delay': `${i * 60}ms` })}>
              <dd className="xh-num">{fmt(shown[STATS.length + i])}</dd>
              <dt>{s.label}</dt>
            </div>
          ))}
          <div data-reveal style={vars({ '--rv-delay': '120ms' })}>
            <dd className="xh-num">1 of 4</dd>
            <dt>Validators offline, still finalizing</dt>
          </div>
        </dl>
        <p className="note xh-mono" data-reveal style={vars({ '--rv-delay': '160ms' })}>
          No TPS claims. Throughput and finality benchmarks on reference hardware will be published with Phase 5.
        </p>
      </div>
    </>
  )
}

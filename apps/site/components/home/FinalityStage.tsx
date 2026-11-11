'use client'

/**
 * Illustration of one BFT round: a block is proposed, 4 validators vote,
 * the vote ring passes 2/3 and the block locks "final", then joins the chain.
 * Every third round one validator is offline and the block still finalizes.
 */
import { useEffect, useRef, useState } from 'react'
import { Check } from './icons'
import { reducedMotion, useInView, wait } from './motion'
import { vars } from './util'

const C = 180
const R = 98
const RV = 156
const ANG = [-45, 45, 135, 225]
const rad = (d: number) => (d * Math.PI) / 180
const pt = (r: number, a: number): [number, number] => [C + r * Math.cos(rad(a)), C + r * Math.sin(rad(a))]
const f2 = (n: number) => n.toFixed(2)
const arc = (a: number) => {
  const [x1, y1] = pt(R, a - 37)
  const [x2, y2] = pt(R, a + 37)
  return `M${f2(x1)} ${f2(y1)}A${R} ${R} 0 0 1 ${f2(x2)} ${f2(y2)}`
}
const VALIDATORS = ANG.map((a, i) => {
  const [x, y] = pt(RV, a)
  const [lx1, ly1] = pt(R + 9, a)
  const [lx2, ly2] = pt(RV - 12, a)
  const right = Math.cos(rad(a)) > 0
  const below = Math.sin(rad(a)) > 0
  return {
    i,
    x,
    y,
    arc: arc(a),
    line: `M${f2(lx2)} ${f2(ly2)}L${f2(lx1)} ${f2(ly1)}`,
    from: [lx2 - lx1, ly2 - ly1],
    to: [lx1, ly1],
    label: `val-${i + 1}`,
    lx: x + (right ? 20 : -20),
    ly: y + (below ? 30 : -22),
    anchor: right ? 'start' : 'end',
  }
})

type Phase = 'propose' | 'vote' | 'final'
const BASE = 18204
const fmt = (n: number) => n.toLocaleString('en-US')

export default function FinalityStage() {
  const ref = useRef<HTMLDivElement>(null)
  const inView = useInView(ref, '0px 0px -15% 0px')
  const [height, setHeight] = useState(BASE)
  const [votes, setVotes] = useState([true, true, true, true])
  const [offline, setOffline] = useState(-1)
  const [phase, setPhase] = useState<Phase>('final')
  const [chain, setChain] = useState(() => [1, 2, 3, 4, 5, 6].map((k) => BASE - k))

  useEffect(() => {
    if (!inView || reducedMotion()) return
    let alive = true
    ;(async () => {
      let round = 0
      let h = BASE
      while (alive) {
        const off = round % 3 === 2 ? 2 : -1
        setPhase('propose')
        setVotes([false, false, false, false])
        setOffline(off)
        await wait(650)
        if (!alive) return
        setPhase('vote')
        const order = [0, 1, 2, 3].filter((i) => i !== off).sort(() => Math.random() - 0.5)
        let count = 0
        for (const i of order) {
          setVotes((v) => v.map((x, j) => (j === i ? true : x)))
          count++
          await wait(300)
          if (!alive) return
          if (count === 3) setPhase('final')
        }
        await wait(1100)
        if (!alive) return
        const done = h
        setChain((c) => [done, ...c].slice(0, 7))
        h += 1
        setHeight(h)
        round++
        await wait(260)
      }
    })()
    return () => {
      alive = false
    }
  }, [inView])

  const count = votes.filter(Boolean).length
  const final = phase === 'final'

  return (
    <div data-reveal style={vars({ '--rv-delay': '80ms' })}>
    <div ref={ref} className={`xh-panel stage is-${phase}`}>
      <div className="track" aria-hidden="true">
        {chain.map((hgt, idx) => (
          <div key={hgt} className="blk" style={vars({ '--k': idx })}>
            <span className="blk-h xh-mono">#{fmt(hgt)}</span>
            <Check className="blk-ok" />
          </div>
        ))}
      </div>

      <div className="unit">
        <svg
          viewBox="0 0 360 360"
          className="unit-svg"
          role="img"
          aria-label={`Block ${fmt(height)}: ${count} of 4 validators voted${final ? ', final' : ''}`}
        >
          <defs>
            <radialGradient id="fin-glow" cx="50%" cy="50%" r="50%">
              <stop offset="0" stopColor="#4fe3cf" stopOpacity=".22" />
              <stop offset="1" stopColor="#4fe3cf" stopOpacity="0" />
            </radialGradient>
          </defs>
          <circle className="glow" cx="180" cy="180" r="150" fill="url(#fin-glow)" />
          <circle cx="180" cy="180" r={R + 22} className="orbit" />

          {VALIDATORS.map((v) => (
            <g key={v.i} className={`val${votes[v.i] ? ' voted' : ''}${offline === v.i ? ' off' : ''}`}>
              <path d={v.line} className="link" />
              <path d={v.arc} className="arc" />
              <circle
                className="pulse"
                r="3"
                cx={v.to[0]}
                cy={v.to[1]}
                style={vars({ '--fx': `${v.from[0]}px`, '--fy': `${v.from[1]}px` })}
              />
              <circle cx={v.x} cy={v.y} r="11" className="node" />
              <circle cx={v.x} cy={v.y} r="3.5" className="core" />
              <text x={v.lx} y={v.ly} textAnchor={v.anchor as 'start' | 'end'} className="vlabel">
                {v.label}
                {offline === v.i && <tspan className="vlabel-off"> offline</tspan>}
              </text>
            </g>
          ))}

          <g className="block" key={height}>
            <rect x="124" y="124" width="112" height="112" rx="20" className="block-rect" />
            <text x="180" y="174" textAnchor="middle" className="block-h">
              #{fmt(height)}
            </text>
            <text x="180" y="200" textAnchor="middle" className="block-s">
              {final ? 'final' : phase === 'vote' ? 'voting' : 'proposed'}
            </text>
          </g>
        </svg>
      </div>

      <div className="status xh-mono">
        <span>
          height <b className="xh-num">{fmt(height)}</b>
        </span>
        <span>
          precommits <b className="xh-num">{count}/4</b>
        </span>
        <span className="status-end">
          {offline >= 0 && <span className="xh-chip">1 of 4 offline</span>}
          <span className={`xh-chip${final ? ' is-final' : ''}`}>
            <span className="xh-dot" />
            {final ? 'final' : 'voting'}
          </span>
        </span>
      </div>
    </div>
    </div>
  )
}

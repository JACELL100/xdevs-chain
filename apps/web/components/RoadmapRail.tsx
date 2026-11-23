'use client'

/** The roadmap as a vertical rail whose line fills as you scroll through it. */
import { useRef } from 'react'
import { m, useScroll, useSpring } from 'motion/react'
import { PHASES } from '@/lib/roadmap'
import { rv } from '@/lib/rv'

export default function RoadmapRail({ detailed = false }: { detailed?: boolean }) {
  const ref = useRef<HTMLOListElement>(null)
  const { scrollYProgress } = useScroll({ target: ref, offset: ['start 0.75', 'end 0.6'] })
  const fill = useSpring(scrollYProgress, { stiffness: 120, damping: 30, mass: 0.4 })
  return (
    <ol ref={ref} className="road">
      <span className="road-line" aria-hidden="true">
        <m.i style={{ scaleY: fill }} />
      </span>
      {PHASES.map((p, i) => (
        <li key={p.n} className="phase" data-s={p.status} {...rv(i * 40)}>
          <span className="node" aria-hidden="true">
            {p.n}
          </span>
          <div className="ph-body">
            <div>
              <div className="ph-title">
                <h3>
                  <span className="sr-only">Phase {p.n}: </span>
                  {p.title}
                </h3>
                <span className={`chip ${p.status === 'live' ? 'is-live' : p.status === 'next' ? 'is-off' : ''}`}>
                  {p.status === 'live' ? <span className="dot is-pulse" /> : null}
                  {p.badge}
                </span>
              </div>
              <p className="ph-sum">{p.summary}</p>
            </div>
            {detailed || i >= 3 ? (
              <ul>
                {p.points.map((x) => (
                  <li key={x}>{x}</li>
                ))}
              </ul>
            ) : (
              <ul>
                {p.points.slice(0, 2).map((x) => (
                  <li key={x}>{x}</li>
                ))}
              </ul>
            )}
          </div>
        </li>
      ))}
    </ol>
  )
}

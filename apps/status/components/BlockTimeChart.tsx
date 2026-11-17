'use client'

// Rolling block-time chart: one bar per block interval, plain SVG (no chart library).
// Single series -> no legend; the card title names it. Hover/focus shows the exact value;
// a visually hidden table carries the same data for screen readers.

import { useEffect, useRef, useState } from 'react'
import type { BlockInterval } from '@/lib/stats'

const H = 168
const PAD = { top: 12, right: 8, bottom: 22, left: 30 }

export default function BlockTimeChart({ intervals, target = 2 }: { intervals: BlockInterval[]; target?: number }) {
  const wrap = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(640)
  const [hover, setHover] = useState<number | null>(null)

  useEffect(() => {
    const el = wrap.current
    if (!el) return
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width
      if (w) setWidth(Math.max(240, Math.round(w)))
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const maxV = Math.max(target * 2, ...intervals.map((i) => i.seconds))
  const yMax = Math.ceil(maxV)
  const innerW = width - PAD.left - PAD.right
  const innerH = H - PAD.top - PAD.bottom
  const slots = Math.max(intervals.length, 30)
  const slotW = innerW / slots
  const barW = Math.max(2, slotW - 2) // 2px surface gap between bars
  const y = (v: number) => PAD.top + innerH - (v / yMax) * innerH
  const ticks = [...new Set([0, target, yMax])].sort((a, b) => a - b)
  const offset = slots - intervals.length // newest bars hug the right edge
  const h = hover !== null ? intervals[hover] : undefined

  return (
    <div className="st-chart" ref={wrap}>
      <svg width={width} height={H} role="img" aria-label={`Block times for the last ${intervals.length} blocks`} onMouseLeave={() => setHover(null)}>
        {ticks.map((t) => (
          <g key={t}>
            <line
              x1={PAD.left}
              x2={width - PAD.right}
              y1={y(t)}
              y2={y(t)}
              className={t === target ? 'st-chart-target' : 'st-chart-grid'}
            />
            <text x={PAD.left - 8} y={y(t)} dy="0.32em" textAnchor="end" className="st-chart-tick">
              {t}s
            </text>
          </g>
        ))}
        {intervals.map((iv, i) => {
          const x = PAD.left + (offset + i) * slotW + (slotW - barW) / 2
          const top = y(Math.max(iv.seconds, 0.05))
          const bh = PAD.top + innerH - top
          const r = Math.min(4, barW / 2, bh)
          // Rounded data end (top), square at the baseline.
          const d = `M${x},${PAD.top + innerH} V${top + r} Q${x},${top} ${x + r},${top} H${x + barW - r} Q${x + barW},${top} ${x + barW},${top + r} V${PAD.top + innerH} Z`
          return <path key={iv.number} d={d} className={`st-bar${hover === i ? ' is-hover' : ''}${hover !== null && hover !== i ? ' is-dim' : ''}`} />
        })}
        {/* Hit targets: full-height slots, wider than the bars. */}
        {intervals.map((iv, i) => (
          <rect
            key={`hit-${iv.number}`}
            x={PAD.left + (offset + i) * slotW}
            y={PAD.top}
            width={slotW}
            height={innerH}
            fill="transparent"
            onMouseEnter={() => setHover(i)}
            onTouchStart={() => setHover(i)}
          />
        ))}
        <text x={width - PAD.right} y={H - 4} textAnchor="end" className="st-chart-tick">
          newest
        </text>
        {intervals[0] ? (
          <text x={PAD.left + offset * slotW} y={H - 4} className="st-chart-tick">
            #{intervals[0].number.toLocaleString('en-US')}
          </text>
        ) : null}
      </svg>
      {h && hover !== null ? (
        <div
          className="st-tip"
          style={{
            left: Math.min(width - 150, Math.max(0, PAD.left + (offset + hover) * slotW - 60)),
            top: Math.max(0, y(h.seconds) - 54),
          }}
          role="status"
        >
          <span className="st-tip-k">block #{h.number.toLocaleString('en-US')}</span>
          <span className="st-tip-v">{h.seconds.toFixed(h.seconds % 1 ? 1 : 0)} s</span>
        </div>
      ) : null}
      <table className="xh-sr">
        <caption>Seconds between consecutive blocks</caption>
        <thead>
          <tr>
            <th>Block</th>
            <th>Seconds since previous</th>
          </tr>
        </thead>
        <tbody>
          {intervals.map((iv) => (
            <tr key={iv.number}>
              <td>{iv.number}</td>
              <td>{iv.seconds}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

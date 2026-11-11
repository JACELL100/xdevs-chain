'use client'

/**
 * The xDEVS Chain mark: two chevron links (">" and "<") interlocked into an X.
 * ">" passes over "<" at the top crossing, "<" over ">" at the bottom.
 * Geometry matches public/brand/xdevs-chain-mark.svg.
 * animate: stroke draw-in -> metallic fill -> sheen sweep -> idle shimmer
 * (the shimmer pauses off-screen; reduced motion shows the still mark).
 */
import { useRef } from 'react'
import { useInView } from './motion'
import { vars } from './util'
import './mark.css'

const GT_OUT = 'M2.69 5.4 21.59 5.4 52.45 32 21.59 58.6 2.69 58.6 33.55 32Z'
const GT_IN = 'M11.31 8.6 20.41 8.6 47.55 32 20.41 55.4 11.31 55.4 38.45 32Z'
const LT_OUT = 'M61.31 5.4 30.45 32 61.31 58.6 42.41 58.6 11.55 32 42.41 5.4Z'
const LT_IN = 'M52.69 8.6 25.55 32 52.69 55.4 43.59 55.4 16.45 32 43.59 8.6Z'
const KO_A = 'M-1.61 3.8 22.19 3.8 58.61 35.2 34.81 35.2Z' // ">" upper arm, knocked out of "<"
const KO_B = 'M5.39 28.8 41.81 60.2 65.61 60.2 29.19 28.8Z' // "<" lower arm, knocked out of ">"

export default function AnimatedMark({ uid = 'xhm', animate = true }: { uid?: string; animate?: boolean }) {
  const ref = useRef<SVGSVGElement>(null)
  const inView = useInView(ref)
  const id = (s: string) => `${uid}-${s}`
  const url = (s: string) => `url(#${id(s)})`

  return (
    <svg ref={ref} className={`xmark${animate ? ' is-anim' : ''}`} viewBox="0 0 64 64" role="img" aria-label="xDEVS Chain">
      <defs>
        <linearGradient id={id('g')} gradientUnits="userSpaceOnUse" x1="6" y1="4" x2="58" y2="60">
          <stop offset="0" stopColor="#ffffff" />
          <stop offset=".18" stopColor="#d9dde3" />
          <stop offset=".38" stopColor="#f4f6f8" />
          <stop offset=".55" stopColor="#a7aeb8" />
          <stop offset=".72" stopColor="#e6e9ed" />
          <stop offset=".88" stopColor="#b4bbc4" />
          <stop offset="1" stopColor="#7f8792" />
        </linearGradient>
        <linearGradient id={id('sh')} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#fff" stopOpacity="0" />
          <stop offset=".5" stopColor="#fff" stopOpacity=".85" />
          <stop offset="1" stopColor="#fff" stopOpacity="0" />
        </linearGradient>
        <mask id={id('a')} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
          <rect x="-8" y="-8" width="80" height="80" fill="#fff" />
          <path d={KO_A} />
        </mask>
        <mask id={id('b')} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
          <rect x="-8" y="-8" width="80" height="80" fill="#fff" />
          <path d={KO_B} />
        </mask>
        {animate && (
          <mask id={id('s')} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
            <path fill="#fff" fillRule="evenodd" d={LT_OUT + LT_IN} mask={url('a')} />
            <path fill="#fff" fillRule="evenodd" d={GT_OUT + GT_IN} mask={url('b')} />
          </mask>
        )}
      </defs>

      <g className="m-fill">
        <path d={LT_IN} fill={url('g')} fillOpacity=".1" mask={url('a')} />
        <path fillRule="evenodd" d={LT_OUT + LT_IN} fill={url('g')} mask={url('a')} />
        <path d={GT_IN} fill={url('g')} fillOpacity=".1" mask={url('b')} />
        <path fillRule="evenodd" d={GT_OUT + GT_IN} fill={url('g')} mask={url('b')} />
      </g>

      {animate && (
        <>
          <g className="m-line" fill="none" stroke={url('g')} strokeWidth=".5" aria-hidden="true">
            <g mask={url('a')}>
              <path d={LT_OUT} pathLength={1} style={vars({ '--i': 1 })} />
              <path d={LT_IN} pathLength={1} style={vars({ '--i': 3 })} />
            </g>
            <g mask={url('b')}>
              <path d={GT_OUT} pathLength={1} style={vars({ '--i': 0 })} />
              <path d={GT_IN} pathLength={1} style={vars({ '--i': 2 })} />
            </g>
          </g>
          <g mask={url('s')} aria-hidden="true">
            <g className={`m-sheen${inView ? '' : ' is-paused'}`}>
              <rect x="-34" y="-12" width="22" height="88" fill={url('sh')} transform="skewX(-22)" />
            </g>
          </g>
        </>
      )}
    </svg>
  )
}

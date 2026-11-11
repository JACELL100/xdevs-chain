'use client'

/** Phone confirming a transfer with a face-scan passkey: idle -> scan -> signed. */
import { useEffect, useRef, useState } from 'react'
import { reducedMotion, useInView, wait } from './motion'

type Phase = 'idle' | 'scan' | 'ok'

export default function PasskeyPhone() {
  const ref = useRef<HTMLDivElement>(null)
  const inView = useInView(ref, '0px 0px -10% 0px')
  const [phase, setPhase] = useState<Phase>('ok')

  useEffect(() => {
    if (!inView || reducedMotion()) return
    let alive = true
    ;(async () => {
      while (alive) {
        setPhase('idle')
        await wait(500)
        if (!alive) return
        setPhase('scan')
        await wait(1700)
        if (!alive) return
        setPhase('ok')
        await wait(2600)
      }
    })()
    return () => {
      alive = false
    }
  }, [inView])

  return (
    <div
      ref={ref}
      className={`phone is-${phase}`}
      role="img"
      aria-label="Phone confirming a transfer of 120 tXDS to ana.xdevs with a face-scan passkey, then showing Signed."
    >
      <span className="island" aria-hidden="true" />
      <div className="screen">
        <div className="sc-head">
          <span className="xh-mono">alice.xdevs</span>
          <span className="xh-mono sc-net">chain 7117</span>
        </div>

        <div className="scan" aria-hidden="true">
          <svg viewBox="0 0 120 120">
            <g className="corners" fill="none" strokeWidth="3" strokeLinecap="round">
              <path d="M14 38V24a10 10 0 0 1 10-10h14" />
              <path d="M82 14h14a10 10 0 0 1 10 10v14" />
              <path d="M106 82v14a10 10 0 0 1-10 10H82" />
              <path d="M38 106H24a10 10 0 0 1-10-10V82" />
            </g>
            <g className="face" fill="none" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
              <path d="M42 46v8M78 46v8" />
              <path d="M60 46v20h-5" />
              <path d="M45 78c8 7 22 7 30 0" />
            </g>
            <path
              className="check"
              d="M38 61 53 76 83 45"
              fill="none"
              strokeWidth="4"
              strokeLinecap="round"
              strokeLinejoin="round"
              pathLength={1}
            />
            <rect className="beam" x="16" y="0" width="88" height="2" rx="1" />
          </svg>
        </div>

        <div className="sc-copy">
          <span className="sc-t">
            <span className="lbl lbl-a">Confirm with passkey</span>
            <span className="lbl lbl-b">Signed</span>
          </span>
          <span className="sc-s">
            Send <b>120 tXDS</b> to <b>ana.xdevs</b>
          </span>
        </div>

        <div className="sc-foot xh-mono">
          <span className="sig">P-256 signature · verified by precompile 0x100</span>
        </div>
      </div>
    </div>
  )
}

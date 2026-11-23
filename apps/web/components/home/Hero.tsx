'use client'

import { useEffect, useRef, useState } from 'react'
import Link from 'next/link'
import { AnimatePresence, m, useReducedMotion, useScroll, useTransform } from 'motion/react'
import Mark from '@/components/Mark'
import { Arrow, Check } from '@/components/icons'
import { fmtInt, useChain } from '@/lib/chain'
import { LINKS } from '@/lib/site'

const EASE = [0.2, 0.8, 0.2, 1] as const
const TICKS = 60

function Seal({ height }: { height: number | null }) {
  // Without live data the ring still advances every 2 s (the Enclave block time), unnumbered.
  const [local, setLocal] = useState(0)
  const [flash, setFlash] = useState(0)
  const reduce = useReducedMotion()
  useEffect(() => {
    if (height != null || reduce) return
    const t = setInterval(() => setLocal((n) => n + 1), 2000)
    return () => clearInterval(t)
  }, [height, reduce])
  const head = (height ?? local) % TICKS
  useEffect(() => setFlash((f) => f + 1), [head])

  return (
    <div className="seal" aria-hidden="true">
      <svg className="rings" viewBox="0 0 600 600">
        <defs>
          <linearGradient id="ring-g" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#fff" stopOpacity=".28" />
            <stop offset=".5" stopColor="#fff" stopOpacity=".04" />
            <stop offset="1" stopColor="#4fe3cf" stopOpacity=".3" />
          </linearGradient>
        </defs>
        <circle cx="300" cy="300" r="292" fill="none" stroke="url(#ring-g)" strokeWidth="1" />
        <g className="ring-spin">
          <circle cx="300" cy="300" r="246" fill="none" stroke="rgba(255,255,255,.08)" strokeWidth="1" strokeDasharray="2 10" />
          <circle cx="300" cy="54" r="3" fill="#4fe3cf" />
        </g>
        <g className="ring-spin is-rev">
          <circle cx="300" cy="300" r="206" fill="none" stroke="rgba(255,255,255,.06)" strokeWidth="1" />
          <circle cx="94" cy="300" r="2.2" fill="#c4c9d1" />
        </g>
        {Array.from({ length: TICKS }, (_, i) => {
          const a = (i / TICKS) * Math.PI * 2 - Math.PI / 2
          const age = (head - i + TICKS) % TICKS
          const lit = age < 10
          const r1 = 268
          const r2 = i % 5 === 0 ? 282 : 277
          return (
            <line
              key={i}
              className="tick"
              x1={300 + Math.cos(a) * r1}
              y1={300 + Math.sin(a) * r1}
              x2={300 + Math.cos(a) * r2}
              y2={300 + Math.sin(a) * r2}
              stroke={lit ? '#4fe3cf' : 'rgba(255,255,255,.16)'}
              strokeOpacity={lit ? 1 - age / 11 : 1}
              strokeWidth={age === 0 ? 2.4 : 1.4}
              strokeLinecap="round"
            />
          )
        })}
      </svg>
      <div key={flash} className={`flash${flash > 1 ? ' is-on' : ''}`} />
      <div className="mark-wrap">
        <Mark uid="hero" animate label="" />
      </div>
    </div>
  )
}

function BlockStream() {
  const { blocks, status } = useChain()
  const shown = blocks.slice(0, 6)
  return (
    <div className="stream">
      <ol aria-label="Latest Enclave blocks">
        <AnimatePresence initial={false} mode="popLayout">
          {shown.length
            ? shown.map((b, i) => (
                <m.li
                  key={b.number}
                  layout
                  initial={{ opacity: 0, x: -40, scale: 0.92 }}
                  animate={{ opacity: 1 - i * 0.08, x: 0, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.9 }}
                  transition={{ duration: 0.7, ease: EASE }}
                  className={`blk-card${i === 0 ? ' is-new' : ''}`}
                >
                  <span className="n">
                    #{fmtInt(b.number)}
                    <span className="fin">
                      <Check />
                      final
                    </span>
                  </span>
                  <span className="meta">
                    <span>
                      {b.txs} {b.txs === 1 ? 'tx' : 'txs'}
                    </span>
                    <span>
                      {b.hash.slice(0, 6)}…{b.hash.slice(-4)}
                    </span>
                  </span>
                </m.li>
              ))
            : Array.from({ length: 6 }, (_, i) => (
                <li key={`g${i}`} className="blk-card is-ghost" style={{ opacity: 1 - i * 0.12 }}>
                  <span className="n">#000,000</span>
                  <span className="meta">
                    <span>0 txs</span>
                    <span>0x0000</span>
                  </span>
                </li>
              ))}
        </AnimatePresence>
      </ol>
      <p className="stream-note">
        {status === 'offline' ? (
          <>
            Can’t reach the Enclave RPC from here right now. <a href={LINKS.status}>Check the status page</a>.
          </>
        ) : status === 'live' ? (
          <>
            Live from <span className="mono">rpc.xdevschain.tech</span>. Every block is final the moment it is committed.
          </>
        ) : (
          <>Connecting to the Enclave testnet…</>
        )}
      </p>
    </div>
  )
}

export default function Hero() {
  const ref = useRef<HTMLElement>(null)
  const { height, status } = useChain()
  const { scrollYProgress } = useScroll({ target: ref, offset: ['start start', 'end start'] })
  const copyY = useTransform(scrollYProgress, [0, 1], [0, -120])
  const copyO = useTransform(scrollYProgress, [0, 0.45], [1, 0])
  const sealScale = useTransform(scrollYProgress, [0, 1], [1, 1.18])
  const sealY = useTransform(scrollYProgress, [0, 1], [0, 90])

  const item = (d: number) => ({
    initial: { opacity: 0, y: 24, filter: 'blur(8px)' },
    animate: { opacity: 1, y: 0, filter: 'blur(0px)' },
    transition: { duration: 1, delay: d, ease: EASE },
  })

  return (
    <section ref={ref} className="hero" aria-labelledby="hero-title">
      <div className="hero-bg" aria-hidden="true">
        <div className="grid" />
        <div className="aura" />
      </div>
      <div className="wrap">
        <m.div className="hero-inner" style={{ y: copyY, opacity: copyO }}>
          <m.div {...item(0.05)}>
            <Link href="/enclave" className="hero-badge">
              <span className="live">
                <span className="dot is-pulse" />
                Enclave public testnet is live
              </span>
              <span className="blk">{status === 'live' && height ? `#${fmtInt(height)}` : 'chain id 717171'}</span>
            </Link>
          </m.div>
          <m.h1 id="hero-title" className="h-hero" {...item(0.15)}>
            <span className="metal">The chain that</span> <span className="metal">has your back.</span>
          </m.h1>
          <m.p className="lead" {...item(0.3)}>
            xDEVS Chain is an EVM-equivalent Layer 1 that protects people at the protocol level. Passkeys instead of seed
            phrases, a vault that holds large transfers for review, approvals that expire and names that can’t be faked.
          </m.p>
          <m.div className="btns" {...item(0.42)}>
            <Link className="btn is-primary" href="/enclave">
              Start on Enclave <Arrow />
            </Link>
            <a className="btn is-ghost" href={LINKS.wallet}>
              Open the wallet
            </a>
          </m.div>
        </m.div>
        <m.div className="hero-stage" style={{ scale: sealScale, y: sealY }}>
          <m.div
            style={{ position: 'absolute', inset: 0 }}
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 1.4, delay: 0.25, ease: EASE }}
          >
            <Seal height={status === 'live' ? height : null} />
            <div className="seal-readout" aria-hidden="true">
              <span>{status === 'live' ? 'latest block · final' : 'block time · 2 s'}</span>
              <b>{status === 'live' && height ? `#${fmtInt(height)}` : 'Enclave'}</b>
            </div>
          </m.div>
        </m.div>
        <BlockStream />
      </div>
    </section>
  )
}

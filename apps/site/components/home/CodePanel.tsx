'use client'

/**
 * Code panel that types itself once when scrolled into view.
 * Server HTML has the full text (no-JS, SEO); the panel only starts empty if
 * it is still below the fold at hydration, so nothing visibly clears.
 */
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { reducedMotion, useInView, wait } from './motion'
import { vars } from './util'

type Seg = [cls: string, text: string]
type Line = { kind: 'cmd' | 'out' | 'code'; segs: Seg[] }
const cmd = (...segs: Seg[]): Line => ({ kind: 'cmd', segs })
const out = (...segs: Seg[]): Line => ({ kind: 'out', segs })
const code = (...segs: Seg[]): Line => ({ kind: 'code', segs })

const TABS: { id: string; label: string; file: string; lines: Line[] }[] = [
  {
    id: 'foundry',
    label: 'Foundry',
    file: 'zsh',
    lines: [
      cmd(['', 'xdevs run --dev']),
      out(['c', '# chain id 7117 · JSON-RPC on 127.0.0.1:8545']),
      cmd(['', 'cast chain-id --rpc-url '], ['v', '$RPC']),
      out(['n', '7117']),
      cmd(['', 'forge create src/Counter.sol:Counter \\']),
      cmd(['', '  --rpc-url '], ['v', '$RPC'], ['', ' --private-key '], ['v', '$PK'], ['', ' --broadcast']),
      out(['d', 'Deployer:    '], ['', '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266']),
      out(['d', 'Deployed to: '], ['', '0x5FbDB2315678afecb367f032d93F642f64180aa3']),
      cmd(['', 'cast call 0x5FbD…0aa3 '], ['s', '"number()(uint256)"'], ['', ' --rpc-url '], ['v', '$RPC']),
      out(['n', '0']),
    ],
  },
  {
    id: 'viem',
    label: 'viem',
    file: 'client.ts',
    lines: [
      code(['k', 'import'], ['', ' { createPublicClient, defineChain, http } '], ['k', 'from'], ['s', " 'viem'"]),
      code(['', '']),
      code(['k', 'const'], ['', ' xdevsDev = '], ['f', 'defineChain'], ['', '({']),
      code(['', '  id: '], ['n', '7117'], ['', ',']),
      code(['', '  name: '], ['s', "'xDEVS Dev (local)'"], ['', ',']),
      code(['', '  nativeCurrency: { name: '], ['s', "'tXDS'"], ['', ', symbol: '], ['s', "'tXDS'"], ['', ', decimals: '], ['n', '18'], ['', ' },']),
      code(['', '  rpcUrls: { default: { http: ['], ['s', "'http://127.0.0.1:8545'"], ['', '] } },']),
      code(['', '})']),
      code(['', '']),
      code(['k', 'const'], ['', ' client = '], ['f', 'createPublicClient'], ['', '({ chain: xdevsDev, transport: '], ['f', 'http'], ['', '() })']),
      code(['k', 'await'], ['', ' client.'], ['f', 'getChainId'], ['', '() '], ['c', '// 7117']),
    ],
  },
]

const lineLen = (l: Line) => l.segs.reduce((n, s) => n + s[1].length, 0)

function visibleSegs(l: Line, i: number, lineIdx: number, charIdx: number): Seg[] {
  if (i < lineIdx) return l.segs
  if (i > lineIdx) return []
  let left = charIdx
  const res: Seg[] = []
  for (const [c, t] of l.segs) {
    if (left <= 0) break
    res.push([c, t.slice(0, left)])
    left -= t.length
  }
  return res
}

export default function CodePanel() {
  const ref = useRef<HTMLDivElement>(null)
  const inView = useInView(ref, '0px 0px -12% 0px')
  const [active, setActive] = useState(0)
  const [pos, setPos] = useState({ line: Infinity, ch: 0 })
  const typed = useRef(new Set<number>())
  const run = useRef(0)

  // start empty only if the panel is below the fold at hydration
  useEffect(() => {
    const r = ref.current?.getBoundingClientRect()
    if (r && r.top > window.innerHeight && !reducedMotion()) setPos({ line: 0, ch: 0 })
    else typed.current.add(0)
  }, [])

  const type = async (tab: number) => {
    const my = ++run.current
    const alive = () => my === run.current
    const ls = TABS[tab].lines
    setPos({ line: 0, ch: 0 })
    await wait(350)
    for (let i = 0; i < ls.length; i++) {
      if (!alive()) return
      const l = ls[i]
      const len = lineLen(l)
      if (l.kind === 'out') {
        setPos({ line: i, ch: len })
        await wait(140)
        continue
      }
      const per = l.kind === 'cmd' ? 26 : 14
      const stepBy = l.kind === 'code' ? 2 : 1
      for (let c = 0; c <= len; c += stepBy) {
        if (!alive()) return
        setPos({ line: i, ch: c })
        await wait(per)
      }
      setPos({ line: i, ch: len })
      await wait(l.kind === 'cmd' ? 260 : 40)
    }
    if (alive()) setPos({ line: Infinity, ch: 0 })
  }

  const start = (tab: number) => {
    if (reducedMotion() || typed.current.has(tab)) {
      run.current++
      setPos({ line: Infinity, ch: 0 })
      return
    }
    typed.current.add(tab)
    type(tab)
  }

  useEffect(() => {
    if (inView && !typed.current.has(active)) start(active)
  }, [inView])

  useEffect(() => () => void run.current++, [])

  const select = (i: number) => {
    if (i === active) return
    setActive(i)
    start(i)
  }
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return
    const n = (active + (e.key === 'ArrowRight' ? 1 : TABS.length - 1)) % TABS.length
    select(n)
    document.getElementById(`evm-tab-${TABS[n].id}`)?.focus()
  }

  const tab = TABS[active]
  return (
    <div ref={ref} className="xh-panel code" data-reveal style={vars({ '--rv-delay': '80ms' })}>
      <div className="bar">
        <div className="tabs" role="tablist" aria-label="Examples" onKeyDown={onKey}>
          {TABS.map((t, i) => (
            <button
              key={t.id}
              id={`evm-tab-${t.id}`}
              role="tab"
              type="button"
              aria-selected={i === active}
              aria-controls="evm-panel"
              tabIndex={i === active ? 0 : -1}
              className={i === active ? 'on' : undefined}
              onClick={() => select(i)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <span className="file xh-mono">{tab.file}</span>
      </div>
      <pre id="evm-panel" role="tabpanel" aria-labelledby={`evm-tab-${tab.id}`} className="xh-mono" tabIndex={0}>
        <code>
          {tab.lines.map((l, i) => (
            <span key={`${active}-${i}`} className={`ln ${l.kind}${i > pos.line ? ' hidden' : ''}`}>
              {l.kind === 'cmd' && <span className="ps">$ </span>}
              {visibleSegs(l, i, pos.line, pos.ch).map((s, j) => (
                <span key={j} className={s[0] ? `t-${s[0]}` : undefined}>
                  {s[1]}
                </span>
              ))}
              {i === pos.line && <span className="cur" aria-hidden="true" />}
              {'\n'}
            </span>
          ))}
        </code>
      </pre>
    </div>
  )
}

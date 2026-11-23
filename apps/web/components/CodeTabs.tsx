'use client'

/** Tabbed code panel with light, hand-written highlighting (no runtime highlighter shipped). */
import { useState } from 'react'
import { AnimatePresence, m } from 'motion/react'
import { CopyButton } from './Buttons'

export type Snippet = { id: string; label: string; code: string; note?: string }

// Minimal tokeniser: comments, strings, numbers, a few keywords.
const KW = /\b(import|from|const|await|export|function|return|new|contract|uint256|public|pragma|solidity)\b/
function highlight(line: string) {
  if (!line.trim()) return ' '
  if (/^\s*(#|\/\/)/.test(line)) return <span className="c">{line}</span>
  const parts = line.split(/("[^"]*"|'[^']*'|`[^`]*`|\b0x[0-9a-fA-F]+\b|\b\d[\d_]*\b)/g)
  return parts.map((p, i) => {
    if (!p) return null
    if (/^["'`]/.test(p)) return <span key={i} className="s">{p}</span>
    if (/^(0x[0-9a-fA-F]+|\d[\d_]*)$/.test(p)) return <span key={i} className="n">{p}</span>
    return p.split(KW).map((q, j) => (KW.test(q) ? <span key={`${i}-${j}`} className="k">{q}</span> : q))
  })
}

export default function CodeTabs({ snippets }: { snippets: Snippet[] }) {
  const [tab, setTab] = useState(snippets[0].id)
  const s = snippets.find((x) => x.id === tab) ?? snippets[0]
  return (
    <div className="code">
      <div className="code-tabs" role="tablist" aria-label="Code examples">
        {snippets.map((x) => (
          <button
            key={x.id}
            type="button"
            role="tab"
            id={`tab-${x.id}`}
            aria-selected={x.id === tab}
            aria-controls={`panel-${x.id}`}
            onClick={() => setTab(x.id)}
          >
            {x.label}
          </button>
        ))}
      </div>
      <AnimatePresence mode="wait" initial={false}>
        <m.pre
          key={s.id}
          id={`panel-${s.id}`}
          role="tabpanel"
          aria-labelledby={`tab-${s.id}`}
          tabIndex={0}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -6 }}
          transition={{ duration: 0.25 }}
        >
          <code>
            {s.code.split('\n').map((l, i) => (
              <span key={i} style={{ display: 'block' }}>
                {highlight(l) || ' '}
              </span>
            ))}
          </code>
        </m.pre>
      </AnimatePresence>
      <div className="code-foot">
        <span>{s.note ?? ''}</span>
        <CopyButton value={s.code} label="Copy code" />
      </div>
    </div>
  )
}

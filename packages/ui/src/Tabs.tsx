'use client'

import { type KeyboardEvent, type ReactNode, useId, useRef, useState } from 'react'
import { cx } from './format'

export type TabItem = {
  id: string
  label: ReactNode
  /** Small count after the label (e.g. number of transactions). */
  count?: ReactNode
  /** Panel content; omit when you render the active panel yourself (controlled mode). */
  content?: ReactNode
}

export type TabsProps = {
  items: TabItem[]
  /** Controlled active id. */
  value?: string
  /** Uncontrolled initial id (defaults to the first item). */
  defaultValue?: string
  onChange?: (id: string) => void
  /** Accessible name of the tab list. */
  label?: string
  className?: string
}

/** Accessible tabs (roving focus with arrow keys / Home / End). */
export function Tabs({ items, value, defaultValue, onChange, label, className }: TabsProps) {
  const [inner, setInner] = useState(defaultValue ?? items[0]?.id)
  const active = value ?? inner
  const base = useId().replace(/[^a-zA-Z0-9_-]/g, '')
  const refs = useRef<(HTMLButtonElement | null)[]>([])

  const select = (id: string) => {
    if (value === undefined) setInner(id)
    onChange?.(id)
  }
  const onKey = (e: KeyboardEvent, i: number) => {
    const n = items.length
    const next = e.key === 'ArrowRight' ? (i + 1) % n : e.key === 'ArrowLeft' ? (i - 1 + n) % n : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : -1
    if (next < 0) return
    e.preventDefault()
    const item = items[next]
    if (!item) return
    select(item.id)
    refs.current[next]?.focus()
  }
  const current = items.find((t) => t.id === active)

  return (
    <div className={className}>
      <div className="xu-tabs-list" role="tablist" aria-label={label}>
        {items.map((t, i) => (
          <button
            key={t.id}
            ref={(el) => {
              refs.current[i] = el
            }}
            type="button"
            role="tab"
            id={`${base}-tab-${t.id}`}
            aria-selected={t.id === active}
            aria-controls={`${base}-panel`}
            tabIndex={t.id === active ? 0 : -1}
            className="xu-tab"
            onClick={() => select(t.id)}
            onKeyDown={(e) => onKey(e, i)}
          >
            {t.label}
            {t.count !== undefined ? <span className="xu-tab-count">{t.count}</span> : null}
          </button>
        ))}
      </div>
      {current?.content !== undefined ? (
        <div className={cx('xu-tabs-panel')} role="tabpanel" id={`${base}-panel`} aria-labelledby={`${base}-tab-${current.id}`}>
          {current.content}
        </div>
      ) : null}
    </div>
  )
}

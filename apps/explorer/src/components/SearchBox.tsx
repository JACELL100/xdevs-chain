'use client'

import { useRouter } from 'next/navigation'
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { classifySearch } from '../lib/format'
import { browserStore, recordAddress } from '../lib/lookalike'
import { paths } from '../lib/routes'

/** Where a search goes without a round trip; `null` means "ask the search page". */
export function searchTarget(q: string): string | null {
  const t = classifySearch(q)
  if (t.type === 'empty') return null
  if (t.type === 'block') return paths.block(t.id)
  if (t.type === 'address') return paths.address(t.id)
  if (t.type === 'hash') return paths.tx(t.id)
  if (t.type === 'name') return paths.name(t.id)
  return paths.search(t.id)
}

/** Header search: block number, tx hash, address or .xdevs name. "/" focuses it from anywhere. */
export function SearchBox({ variant = 'header', autoFocus }: { variant?: 'header' | 'hero'; autoFocus?: boolean }) {
  const [q, setQ] = useState('')
  const router = useRouter()
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (variant !== 'header') return
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null
      if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey || (el && /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) || el?.isContentEditable) return
      e.preventDefault()
      input.current?.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [variant])

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const to = searchTarget(q)
    if (!to) return
    const t = classifySearch(q)
    if (t.type === 'address') recordAddress(browserStore(), t.id, 'search', Date.now())
    // Placeholder-served pages need a document load; the search page is a normal client route.
    if (to.startsWith('/search')) router.push(to)
    else window.location.assign(to)
  }

  const id = variant === 'hero' ? 'q-hero' : 'q'
  return (
    <form role="search" className={`x-search is-${variant}`} onSubmit={submit}>
      <label htmlFor={id} className="xh-sr">
        Search by block number, transaction hash, address or .xdevs name
      </label>
      <svg className="x-search-icon" viewBox="0 0 16 16" aria-hidden="true">
        <circle cx="7" cy="7" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.4" />
        <path d="m10.5 10.5 3.2 3.2" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      </svg>
      <input
        ref={input}
        id={id}
        type="search"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={variant === 'hero' ? 'Search block, tx hash, address or name.xdevs' : 'Search'}
        autoComplete="off"
        spellCheck={false}
        enterKeyHint="search"
        autoFocus={autoFocus}
      />
      {variant === 'header' ? (
        <kbd className="x-kbd" aria-hidden="true">
          /
        </kbd>
      ) : (
        <button type="submit" className="xh-btn xh-btn-primary xu-btn is-sm">
          Search
        </button>
      )}
    </form>
  )
}

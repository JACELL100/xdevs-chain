'use client'

import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { useRouter } from 'next/navigation'

/*
 * Static full-text search over the docs. The Pagefind index is built after
 * `next build` (lib/docs/build-search.mjs) into <out>/pagefind/. Nothing is
 * downloaded until the dialog opens; then only pagefind.js + the index
 * fragments that match the query are fetched.
 */

type PagefindSub = { title: string; url: string; excerpt: string }
type PagefindData = { url: string; excerpt: string; meta: Record<string, string>; sub_results?: PagefindSub[] }
type Pagefind = {
  options: (o: Record<string, unknown>) => Promise<void>
  init: () => Promise<void>
  debouncedSearch: (q: string, o?: object, ms?: number) => Promise<{ results: { id: string; data: () => Promise<PagefindData> }[] } | null>
}

type Hit = { key: string; url: string; title: string; excerpt: string; section?: string; kind: 'page' | 'heading' }

let pagefindPromise: Promise<Pagefind> | undefined
function loadPagefind(): Promise<Pagefind> {
  if (!pagefindPromise) {
    const url = '/pagefind/pagefind.js'
    pagefindPromise = (import(/* webpackIgnore: true */ /* turbopackIgnore: true */ url) as Promise<Pagefind>).then(
      async (pf) => {
        await pf.options({ excerptLength: 22 })
        await pf.init()
        return pf
      },
    )
    pagefindPromise.catch(() => {
      pagefindPromise = undefined
    })
  }
  return pagefindPromise
}

function cleanUrl(u: string) {
  return u.replace(/\.html(?=$|#)/, '').replace(/\/index(?=$|#)/, '/') || '/'
}

export default function Search() {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLUListElement>(null)
  const router = useRouter()
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<Hit[]>([])
  const [status, setStatus] = useState<'idle' | 'loading' | 'ready' | 'error'>('idle')
  const [sel, setSel] = useState(0)
  const [mac, setMac] = useState(true)
  const listId = useId()

  const open = useCallback(() => {
    const d = dialogRef.current
    if (!d || d.open) return
    d.showModal()
    inputRef.current?.select()
    setStatus((s) => (s === 'ready' ? s : 'loading'))
    loadPagefind().then(
      () => setStatus('ready'),
      () => setStatus('error'),
    )
  }, [])

  const close = useCallback(() => dialogRef.current?.close(), [])

  useEffect(() => {
    setMac(/Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent))
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      const typing = !!t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
      if ((e.key === 'k' || e.key === 'K') && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        if (dialogRef.current?.open) close()
        else open()
      } else if (e.key === '/' && !typing && !dialogRef.current?.open) {
        e.preventDefault()
        open()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, close])

  useEffect(() => {
    if (status !== 'ready') return
    const q = query.trim()
    if (!q) {
      setHits([])
      return
    }
    let cancelled = false
    loadPagefind()
      .then((pf) => pf.debouncedSearch(q, {}, 120))
      .then(async (res) => {
        if (!res || cancelled) return
        const data = await Promise.all(res.results.slice(0, 8).map((r) => r.data()))
        if (cancelled) return
        const out: Hit[] = []
        for (const d of data) {
          const page = cleanUrl(d.url)
          const title = d.meta.title || page
          const section = d.meta.section
          const subs = (d.sub_results ?? []).filter((s) => s.url.includes('#') && s.title !== title).slice(0, 3)
          out.push({ key: page, url: page, title, excerpt: subs.length ? '' : d.excerpt, section, kind: 'page' })
          for (const s of subs) {
            const u = cleanUrl(s.url)
            out.push({ key: u, url: u, title: s.title, excerpt: s.excerpt, kind: 'heading' })
          }
        }
        setHits(out)
        setSel(0)
      })
      .catch(() => !cancelled && setStatus('error'))
    return () => {
      cancelled = true
    }
  }, [query, status])

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${sel}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [sel])

  function go(hit: Hit | undefined) {
    if (!hit) return
    close()
    router.push(hit.url)
  }

  function onInputKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setSel((s) => Math.min(s + 1, hits.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setSel((s) => Math.max(s - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      go(hits[sel])
    }
  }

  const q = query.trim()
  return (
    <>
      <button type="button" className="d-search-btn" onClick={open} aria-haspopup="dialog" aria-label="Search documentation">
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
          <circle cx="7" cy="7" r="4.75" fill="none" stroke="currentColor" strokeWidth="1.4" />
          <path d="M10.5 10.5 14 14" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
        <span className="d-search-btn-text">Search docs</span>
        <kbd className="d-kbd">{mac ? '⌘' : 'Ctrl'} K</kbd>
      </button>

      <dialog
        ref={dialogRef}
        className="d-search"
        aria-label="Search documentation"
        onClick={(e) => {
          if (e.target === dialogRef.current) close()
        }}
        onClose={() => setSel(0)}
      >
        <div className="d-search-panel">
          <div className="d-search-field">
            <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
              <circle cx="7" cy="7" r="4.75" fill="none" stroke="currentColor" strokeWidth="1.4" />
              <path d="M10.5 10.5 14 14" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
            </svg>
            <input
              ref={inputRef}
              type="search"
              placeholder="Search the docs"
              autoComplete="off"
              spellCheck={false}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onInputKey}
              role="combobox"
              aria-expanded={hits.length > 0}
              aria-controls={listId}
              aria-activedescendant={hits[sel] ? `${listId}-${sel}` : undefined}
            />
            <button type="button" className="d-kbd d-search-esc" onClick={close} aria-label="Close search">
              Esc
            </button>
          </div>

          <div className="d-search-body">
            {status === 'error' ? (
              <p className="d-search-empty">Search isn’t available here. The index is built by <code>npm run build</code>.</p>
            ) : !q ? (
              <p className="d-search-empty">Search guides, concepts, RPC methods and node settings.</p>
            ) : status === 'loading' ? (
              <p className="d-search-empty">Loading index…</p>
            ) : hits.length === 0 ? (
              <p className="d-search-empty">
                No results for <b>“{q}”</b>
              </p>
            ) : (
              <ul ref={listRef} id={listId} role="listbox" className="d-search-list">
                {hits.map((h, i) => (
                  <li
                    key={h.key + i}
                    id={`${listId}-${i}`}
                    role="option"
                    aria-selected={i === sel}
                    data-index={i}
                    data-kind={h.kind}
                    onMouseMove={() => sel !== i && setSel(i)}
                    onClick={() => go(h)}
                  >
                    <span className="d-search-hit-title">
                      {h.kind === 'heading' ? <span className="d-search-hash">#</span> : null}
                      {h.title}
                      {h.section ? <span className="d-search-section">{h.section}</span> : null}
                    </span>
                    {h.excerpt ? <span className="d-search-excerpt" dangerouslySetInnerHTML={{ __html: h.excerpt }} /> : null}
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="d-search-foot" aria-hidden="true">
            <span>
              <kbd className="d-kbd">↑</kbd>
              <kbd className="d-kbd">↓</kbd> to navigate
            </span>
            <span>
              <kbd className="d-kbd">↵</kbd> to open
            </span>
            <span className="d-search-by">Static search · Pagefind</span>
          </div>
        </div>
      </dialog>
    </>
  )
}

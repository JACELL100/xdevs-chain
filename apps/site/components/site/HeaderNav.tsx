'use client'

import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { useEffect, useRef, useState } from 'react'
import { GITHUB_URL, NAV, NAV_MORE, type NavLink } from './nav'

function isActive(path: string, l: NavLink) {
  return !!l.activeMatch && path.startsWith(l.activeMatch)
}

function ItemLink({ l, className, onClick }: { l: NavLink; className?: string; onClick?: () => void }) {
  if (l.external)
    return (
      <a href={l.href} className={className} target="_blank" rel="noopener" onClick={onClick}>
        {l.text}
        <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true" className="xh-ext">
          <path d="M5 11 11 5M6 5h5v5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        </svg>
      </a>
    )
  return (
    <Link href={l.href} className={className} onClick={onClick}>
      {l.text}
    </Link>
  )
}

export default function HeaderNav() {
  const path = usePathname() || '/'
  const [more, setMore] = useState(false)
  const [sheet, setSheet] = useState(false)
  const moreRef = useRef<HTMLLIElement>(null)

  // close menus on navigation
  useEffect(() => {
    setMore(false)
    setSheet(false)
  }, [path])

  useEffect(() => {
    if (!more) return
    const onDoc = (e: MouseEvent) => {
      if (!moreRef.current?.contains(e.target as Node)) setMore(false)
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setMore(false)
    document.addEventListener('mousedown', onDoc)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDoc)
      document.removeEventListener('keydown', onKey)
    }
  }, [more])

  useEffect(() => {
    if (!sheet) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setSheet(false)
    document.addEventListener('keydown', onKey)
    document.documentElement.classList.add('xh-noscroll')
    return () => {
      document.removeEventListener('keydown', onKey)
      document.documentElement.classList.remove('xh-noscroll')
    }
  }, [sheet])

  const moreActive = NAV_MORE.some((l) => isActive(path, l))

  return (
    <>
      <nav className="xh-nav" aria-label="Main">
        <ul>
          {NAV.map((l) => (
            <li key={l.href}>
              <ItemLink l={l} className={isActive(path, l) ? 'is-active' : undefined} />
            </li>
          ))}
          <li className="xh-more" ref={moreRef}>
            <button
              type="button"
              aria-expanded={more}
              aria-haspopup="true"
              className={moreActive ? 'is-active' : undefined}
              onClick={() => setMore((v) => !v)}
            >
              More
              <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.5" />
              </svg>
            </button>
            <div className="xh-more-menu" hidden={!more}>
              {NAV_MORE.map((l) => (
                <ItemLink key={l.href} l={l} className={isActive(path, l) ? 'is-active' : undefined} />
              ))}
            </div>
          </li>
        </ul>
      </nav>

      <button
        type="button"
        className="xh-burger"
        aria-label={sheet ? 'Close menu' : 'Open menu'}
        aria-expanded={sheet}
        aria-controls="xh-sheet"
        onClick={() => setSheet((v) => !v)}
      >
        <span />
        <span />
      </button>

      <div id="xh-sheet" className="xh-sheet" data-open={sheet} hidden={!sheet}>
        <nav aria-label="Main">
          {[...NAV, ...NAV_MORE].map((l) => (
            <ItemLink key={l.href} l={l} className={isActive(path, l) ? 'is-active' : undefined} />
          ))}
          <a href={GITHUB_URL} target="_blank" rel="noopener">
            GitHub
          </a>
        </nav>
      </div>
    </>
  )
}

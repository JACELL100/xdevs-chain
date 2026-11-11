'use client'

import { useEffect, useRef } from 'react'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { DOCS_NAV } from '@/lib/docs/nav'
import { useDrawer } from './DocsShell'

export default function Sidebar() {
  const pathname = usePathname().replace(/\.html$/, '').replace(/\/$/, '') || '/'
  const { open, setOpen } = useDrawer()
  const navRef = useRef<HTMLElement>(null)

  // Keep the active link in view inside the scrolling sidebar.
  useEffect(() => {
    const active = navRef.current?.querySelector<HTMLElement>('[aria-current="page"]')
    if (!active || !navRef.current) return
    const box = navRef.current.getBoundingClientRect()
    const r = active.getBoundingClientRect()
    if (r.top < box.top || r.bottom > box.bottom) active.scrollIntoView({ block: 'center' })
  }, [pathname])

  useEffect(() => {
    if (open) navRef.current?.querySelector<HTMLElement>('a')?.focus()
  }, [open])

  return (
    <>
      <div className="d-backdrop" data-open={open || undefined} onClick={() => setOpen(false)} aria-hidden="true" />
      <aside id="docs-sidebar" className="d-sidebar" data-open={open || undefined} aria-label="Documentation">
        <nav ref={navRef} className="d-sidebar-scroll">
          {DOCS_NAV.map((group) => (
            <div className="d-nav-group" key={group.text}>
              <p className="d-nav-title">{group.text}</p>
              <ul>
                {group.items.map((item) => {
                  const active = item.href === pathname
                  return (
                    <li key={item.href}>
                      <Link href={item.href} className="d-nav-link" aria-current={active ? 'page' : undefined}>
                        {item.text}
                      </Link>
                    </li>
                  )
                })}
              </ul>
            </div>
          ))}
        </nav>
      </aside>
    </>
  )
}

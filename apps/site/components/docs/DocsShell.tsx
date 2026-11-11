'use client'

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from 'react'
import { usePathname } from 'next/navigation'

type DrawerState = { open: boolean; setOpen: (v: boolean) => void; toggle: () => void }
const DrawerContext = createContext<DrawerState>({ open: false, setOpen: () => {}, toggle: () => {} })

export function useDrawer() {
  return useContext(DrawerContext)
}

/** Holds the mobile sidebar drawer state shared by the header button and the sidebar. */
export default function DocsShell({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const pathname = usePathname()
  const toggle = useCallback(() => setOpen((v) => !v), [])

  useEffect(() => setOpen(false), [pathname])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false)
        document.getElementById('docs-menu-button')?.focus()
      }
    }
    const mq = window.matchMedia('(min-width: 960px)')
    const onMq = () => mq.matches && setOpen(false)
    document.addEventListener('keydown', onKey)
    mq.addEventListener('change', onMq)
    document.documentElement.classList.add('d-drawer-open')
    return () => {
      document.removeEventListener('keydown', onKey)
      mq.removeEventListener('change', onMq)
      document.documentElement.classList.remove('d-drawer-open')
    }
  }, [open])

  return <DrawerContext.Provider value={{ open, setOpen, toggle }}>{children}</DrawerContext.Provider>
}

export function MenuButton() {
  const { open, toggle } = useDrawer()
  return (
    <button
      id="docs-menu-button"
      type="button"
      className="d-menu-btn"
      aria-expanded={open}
      aria-controls="docs-sidebar"
      aria-label={open ? 'Close docs navigation' : 'Open docs navigation'}
      onClick={toggle}
    >
      <svg viewBox="0 0 18 18" width="18" height="18" aria-hidden="true">
        <path d="M3 5.5h12M3 9h12M3 12.5h7" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      </svg>
    </button>
  )
}

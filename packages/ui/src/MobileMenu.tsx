'use client'

import { type ReactNode, useEffect, useState } from 'react'

/** Burger button + full-screen sheet for the AppShell nav below 900px. Closes on link click and Escape. */
export function MobileMenu({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('keydown', onKey)
    document.documentElement.classList.add('xu-noscroll')
    return () => {
      document.removeEventListener('keydown', onKey)
      document.documentElement.classList.remove('xu-noscroll')
    }
  }, [open])

  return (
    <>
      <button
        type="button"
        className="xu-burger"
        aria-label={open ? 'Close menu' : 'Open menu'}
        aria-expanded={open}
        aria-controls="xu-sheet"
        onClick={() => setOpen((v) => !v)}
      >
        <span />
        <span />
      </button>
      <div
        id="xu-sheet"
        className="xu-sheet"
        hidden={!open}
        onClick={(e) => {
          if ((e.target as HTMLElement).closest('a')) setOpen(false)
        }}
      >
        {children}
      </div>
    </>
  )
}

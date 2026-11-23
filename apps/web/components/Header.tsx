'use client'

import { useEffect, useState } from 'react'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { AnimatePresence, m } from 'motion/react'
import { LINKS, NAV } from '@/lib/site'
import { ArrowUpRight, GitHub } from './icons'

export default function Header() {
  const pathname = usePathname()
  const [scrolled, setScrolled] = useState(false)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 8)
    on()
    window.addEventListener('scroll', on, { passive: true })
    return () => window.removeEventListener('scroll', on)
  }, [])

  useEffect(() => setOpen(false), [pathname])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('keydown', onKey)
    document.documentElement.style.overflow = 'hidden'
    return () => {
      document.removeEventListener('keydown', onKey)
      document.documentElement.style.overflow = ''
    }
  }, [open])

  const current = (href: string) => (href === pathname ? 'page' : undefined)

  return (
    <>
      <header className={`header${scrolled ? ' is-scrolled' : ''}${open ? ' is-open' : ''}`}>
        <a className="skip" href="#main">
          Skip to content
        </a>
        <div className="header-inner">
          <Link href="/" className="brand" aria-label="xDEVS Chain home">
            <img src="/brand/xdevs-chain-mark.svg" width={26} height={26} alt="" />
            <span>
              xDEVS <span>Chain</span>
            </span>
          </Link>
          <nav className="nav" aria-label="Main">
            {NAV.map((n) =>
              n.external ? (
                <a key={n.href} href={n.href}>
                  {n.label}
                  <ArrowUpRight />
                </a>
              ) : (
                <Link key={n.href} href={n.href} aria-current={current(n.href)}>
                  {n.label}
                </Link>
              ),
            )}
          </nav>
          <div className="header-actions">
            <a className="icon-btn gh" href={LINKS.github} aria-label="xDEVS Chain on GitHub">
              <GitHub width={18} height={18} />
            </a>
            <a className="btn is-primary is-sm" href={LINKS.wallet}>
              Launch app
            </a>
            <button
              type="button"
              className="icon-btn menu-btn"
              aria-expanded={open}
              aria-controls="site-menu"
              aria-label={open ? 'Close menu' : 'Open menu'}
              onClick={() => setOpen((o) => !o)}
            >
              <span className="burger" aria-hidden="true">
                <i />
                <i />
              </span>
            </button>
          </div>
        </div>
      </header>
      <AnimatePresence>
        {open && (
          <m.div
            id="site-menu"
            className="sheet"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.25 }}
          >
            <nav aria-label="Mobile">
              {[{ label: 'Home', href: '/' }, ...NAV, { label: 'GitHub', href: LINKS.github, external: true }].map((n, i) => (
                <m.div
                  key={n.href}
                  initial={{ opacity: 0, y: 16 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: 0.04 * i, duration: 0.45, ease: [0.2, 0.8, 0.2, 1] }}
                >
                  {'external' in n && n.external ? (
                    <a href={n.href}>
                      {n.label} <ArrowUpRight width={20} height={20} />
                    </a>
                  ) : (
                    <Link href={n.href} aria-current={current(n.href)}>
                      {n.label}
                    </Link>
                  )}
                </m.div>
              ))}
            </nav>
            <div className="btns">
              <a className="btn is-primary" href={LINKS.wallet}>
                Open the wallet
              </a>
              <a className="btn is-ghost" href={LINKS.faucet}>
                Get tXDS
              </a>
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </>
  )
}

'use client'
// Header account pill + menu: active account, secondary pages (names, EOA lock, settings), account switching.
import { AddressChip, cx } from '@xdevs/ui'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { useEffect, useRef, useState } from 'react'
import { config, explorerAddress } from '../config.ts'
import { shortAddr } from '../lib/format.ts'
import { useWallet } from '../wallet/WalletProvider.tsx'
import { IconChevron, IconDown, IconGear, IconLink, IconLock, IconPlus, IconTag } from './icons.tsx'
import { Avatar } from './ui.tsx'

export function AccountMenu() {
  const { ready, rec, persisted, selectAccount, demo, setDemo, primaryName } = useWallet()
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  const path = usePathname()

  useEffect(() => setOpen(false), [path])
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  if (!ready || !rec) return null
  const name = primaryName ?? rec.label
  return (
    <div className="w-acct" ref={box}>
      <button type="button" className={cx('w-acct-btn', open && 'is-open')} aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen((v) => !v)}>
        <Avatar address={rec.address} size={24} />
        <span className="w-acct-name">{name}</span>
        <IconDown size={14} />
      </button>
      {open ? (
        <div className="w-acct-menu" role="menu">
          <div className="w-acct-head">
            <Avatar address={rec.address} size={40} />
            <div>
              <p className="w-acct-title">{name}</p>
              <AddressChip address={rec.address} href={explorerAddress(rec.address)} />
            </div>
          </div>
          <nav className="w-acct-links" aria-label="Account">
            <Link href="/names" role="menuitem">
              <IconTag /> .xdevs name <IconChevron size={14} />
            </Link>
            <Link href="/eoa-lock" role="menuitem">
              <IconLock /> MetaMask EOA lock <IconChevron size={14} />
            </Link>
            <Link href="/settings" role="menuitem">
              <IconGear /> Settings <IconChevron size={14} />
            </Link>
            <a href={explorerAddress(rec.address)} target="_blank" rel="noopener noreferrer" role="menuitem">
              <IconLink /> View in explorer <IconChevron size={14} />
            </a>
          </nav>
          {!demo && persisted.accounts.length > 1 ? (
            <div className="w-acct-list">
              <p className="w-acct-list-title">On this device</p>
              {persisted.accounts.map((a, i) => (
                <button key={a.address} type="button" role="menuitemradio" aria-checked={i === persisted.active} className={cx(i === persisted.active && 'is-on')} onClick={() => selectAccount(i)}>
                  <Avatar address={a.address} size={20} />
                  <span>{a.label}</span>
                  <code>{shortAddr(a.address)}</code>
                </button>
              ))}
            </div>
          ) : null}
          <div className="w-acct-foot">
            {demo ? (
              !config.demo ? (
                <button type="button" onClick={() => setDemo(false)}>
                  Exit demo
                </button>
              ) : (
                <span className="w-muted">Demo build</span>
              )
            ) : (
              <Link href="/welcome">
                <IconPlus size={14} /> Add or create an account
              </Link>
            )}
          </div>
        </div>
      ) : null}
    </div>
  )
}

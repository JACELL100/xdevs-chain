'use client'
// Wallet UI primitives on top of @xdevs/ui (styles in src/app/wallet.css, prefix `w-`).
import { cx } from '@xdevs/ui'
import { type ReactNode, useEffect, useId, useRef, useState } from 'react'
import { fmtCountdown, groupAddr } from '../lib/format.ts'
import { IconAlert, IconCheck, IconInfo, IconShield } from './icons.tsx'

export function Page({ title, sub, eyebrow, actions, children, narrow }: { title: ReactNode; sub?: ReactNode; eyebrow?: ReactNode; actions?: ReactNode; children: ReactNode; narrow?: boolean }) {
  return (
    <div className={cx('xu-page w-page', narrow && 'is-narrow')}>
      <header className="w-page-head">
        <div>
          {eyebrow ? <p className="w-eyebrow">{eyebrow}</p> : null}
          <h1 className="xu-page-title">{title}</h1>
          {sub ? <p className="xu-page-sub">{sub}</p> : null}
        </div>
        {actions ? <div className="w-page-actions">{actions}</div> : null}
      </header>
      {children}
    </div>
  )
}

export type Tone = 'info' | 'ok' | 'warn' | 'alert'

export function Notice({ tone = 'info', title, children, action, icon }: { tone?: Tone; title?: ReactNode; children?: ReactNode; action?: ReactNode; icon?: ReactNode }) {
  const glyph = icon ?? (tone === 'ok' ? <IconCheck /> : tone === 'info' ? <IconInfo /> : <IconAlert />)
  return (
    <div className={cx('w-notice', `is-${tone}`)} role={tone === 'alert' ? 'alert' : undefined}>
      <span className="w-notice-icon">{glyph}</span>
      <div className="w-notice-body">
        {title ? <p className="w-notice-title">{title}</p> : null}
        {children ? <div className="w-notice-text">{children}</div> : null}
      </div>
      {action ? <div className="w-notice-action">{action}</div> : null}
    </div>
  )
}

export function Field({ label, hint, error, children, htmlFor, aside }: { label: ReactNode; hint?: ReactNode; error?: ReactNode; children: ReactNode; htmlFor?: string; aside?: ReactNode }) {
  return (
    <div className={cx('w-field', error ? 'has-error' : null)}>
      <div className="w-field-top">
        <label htmlFor={htmlFor}>{label}</label>
        {aside ? <span className="w-field-aside">{aside}</span> : null}
      </div>
      {children}
      {error ? (
        <p className="w-field-error" role="alert">
          {error}
        </p>
      ) : hint ? (
        <p className="w-field-hint">{hint}</p>
      ) : null}
    </div>
  )
}

export function Toggle({ checked, onChange, label, hint, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: ReactNode; hint?: ReactNode; disabled?: boolean }) {
  const id = useId()
  return (
    <div className="w-toggle">
      <div>
        <label htmlFor={id} className="w-toggle-label">
          {label}
        </label>
        {hint ? <p className="w-toggle-hint">{hint}</p> : null}
      </div>
      <input id={id} type="checkbox" role="switch" className="w-switch" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
    </div>
  )
}

export function Segmented<T extends string>({ options, value, onChange, label }: { options: { id: T; label: ReactNode }[]; value: T; onChange: (v: T) => void; label: string }) {
  return (
    <div className="w-seg" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button key={o.id} type="button" role="radio" aria-checked={o.id === value} className={cx('w-seg-item', o.id === value && 'is-on')} onClick={() => onChange(o.id)}>
          {o.label}
        </button>
      ))}
    </div>
  )
}

export function Meter({ value, max, tone = 'final', label }: { value: number; max: number; tone?: 'final' | 'warn' | 'alert'; label: string }) {
  const pct = max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0
  return (
    <div className={cx('w-meter', `is-${tone}`)} role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={max} aria-valuenow={value}>
      <span style={{ width: `${pct}%` }} />
    </div>
  )
}

/** Full address split into 4-character groups; first and last groups emphasised (what poisoning scams copy). */
export function AddressFull({ address, className }: { address: string; className?: string }) {
  const g = groupAddr(address)
  return (
    <span className={cx('w-addr-full', className)} aria-label={address}>
      <span className="w-addr-0x">0x</span>
      {g.map((part, i) => (
        <span key={i} className={i === 0 || i === g.length - 1 ? 'is-key' : undefined}>
          {part}
        </span>
      ))}
    </span>
  )
}

/** Deterministic, decorative avatar from an address (two hues from its bytes). */
export function Avatar({ address, size = 32 }: { address: string; size?: number }) {
  const h = address.toLowerCase().replace(/^0x/, '')
  const a = (Number.parseInt(h.slice(0, 6) || '0', 16) % 360 + 360) % 360
  const b = (a + 40 + (Number.parseInt(h.slice(-4) || '0', 16) % 80)) % 360
  return (
    <span
      className="w-avatar"
      aria-hidden="true"
      style={{ width: size, height: size, background: `radial-gradient(120% 120% at 20% 15%, hsl(${a} 40% 72%) 0%, hsl(${b} 30% 34%) 55%, #0b0d12 100%)` }}
    />
  )
}

export function TokenGlyph({ symbol, size = 36 }: { symbol: string; size?: number }) {
  const native = symbol === 'tXDS'
  return (
    <span className={cx('w-token', native && 'is-native')} style={{ width: size, height: size }} aria-hidden="true">
      {native ? (
        <svg viewBox="0 0 24 24" width={size * 0.5} height={size * 0.5}>
          <path d="M6 6l12 12M18 6L6 18" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
        </svg>
      ) : (
        symbol.replace(/^[^A-Za-z]+/, '').slice(0, 2)
      )}
    </span>
  )
}

export function useNow(intervalMs = 1_000): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000))
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), intervalMs)
    return () => clearInterval(t)
  }, [intervalMs])
  return now
}

export function Countdown({ until }: { until: number }) {
  const now = useNow()
  return <span className="xh-num">{fmtCountdown(until - now)}</span>
}

export function DemoTag() {
  return (
    <span className="w-demo-tag" title="Demo data: nothing here is real or on-chain">
      Demo
    </span>
  )
}

/** Accessible modal confirmation built on <dialog> (focus trap and Escape handled by the browser). */
export function ConfirmDialog({
  open,
  title,
  children,
  confirmLabel,
  tone = 'primary',
  busy,
  onConfirm,
  onClose,
}: {
  open: boolean
  title: ReactNode
  children: ReactNode
  confirmLabel: ReactNode
  tone?: 'primary' | 'danger'
  busy?: boolean
  onConfirm: () => void
  onClose: () => void
}) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const d = ref.current
    if (!d) return
    if (open && !d.open) d.showModal()
    if (!open && d.open) d.close()
  }, [open])
  return (
    <dialog ref={ref} className="w-dialog" onClose={onClose} onCancel={(e) => busy && e.preventDefault()}>
      <h2 className="w-dialog-title">{title}</h2>
      <div className="w-dialog-body">{children}</div>
      <div className="w-dialog-actions">
        <button type="button" className="xh-btn xh-btn-ghost xu-btn is-md" disabled={busy} onClick={onClose}>
          Back
        </button>
        <button type="button" className={cx('xh-btn xu-btn is-md', tone === 'danger' ? 'xu-btn-danger' : 'xh-btn-primary')} disabled={busy} onClick={onConfirm} autoFocus>
          {confirmLabel}
        </button>
      </div>
    </dialog>
  )
}

export function KV({ rows }: { rows: [ReactNode, ReactNode][] }) {
  return (
    <dl className="w-kv">
      {rows.map(([k, v], i) => (
        <div key={i}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  )
}

export function SectionTitle({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="w-section-title">
      <h2>{children}</h2>
      {action}
    </div>
  )
}

export function ShieldBadge({ on }: { on: boolean }) {
  return (
    <span className={cx('w-shield', on ? 'is-on' : 'is-off')}>
      <IconShield size={14} />
      {on ? 'Guardian Vault on' : 'Guardian Vault off'}
    </span>
  )
}

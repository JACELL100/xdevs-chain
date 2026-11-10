import { useState, type MouseEvent, type ReactNode } from 'react'
import { navigate, paths } from '../lib/router.ts'
import { shortAddress, shortHash, type Tone } from '../lib/format.ts'
import { addressParts } from '../lib/lookalike.ts'
import { ApiError } from '../lib/api.ts'

export function Link({ to, children, className, title }: { to: string; children: ReactNode; className?: string; title?: string }) {
  const onClick = (e: MouseEvent<HTMLAnchorElement>) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
    e.preventDefault()
    navigate(to)
  }
  return (
    <a href={to} onClick={onClick} className={className} title={title}>
      {children}
    </a>
  )
}

export function Badge({ tone = 'neutral', children, title }: { tone?: Tone; children: ReactNode; title?: string }) {
  return (
    <span className={`badge badge-${tone}`} title={title}>
      {children}
    </span>
  )
}

/** Full address with the first/last 4 hex chars emphasised, so a look-alike's different middle is visible. */
export function FullAddress({ address }: { address: string }) {
  const p = addressParts(address)
  return (
    <span className="mono addr-full" aria-label={address}>
      <strong>{p.head}</strong>
      <span className="addr-mid">{p.middle}</span>
      <strong>{p.tail}</strong>
    </span>
  )
}

export function AddressLink({ address, name, label, full = false }: { address: string | null | undefined; name?: string | null; label?: string | null; full?: boolean }) {
  if (!address) return <span className="muted">—</span>
  return (
    <span className="addr-link">
      <Link to={paths.address(address)} title={address}>
        {name ? <span className="xns-name">{name}</span> : null}
        {name ? ' ' : null}
        {full ? <FullAddress address={address} /> : <span className="mono">{shortAddress(address)}</span>}
      </Link>
      {label ? <span className="muted small"> ({label})</span> : null}
    </span>
  )
}

export function TxLink({ hash, short = true }: { hash: string; short?: boolean }) {
  return (
    <Link to={paths.tx(hash)} className="mono" title={hash}>
      {short ? shortHash(hash) : hash}
    </Link>
  )
}

export function BlockLink({ n }: { n: number }) {
  return <Link to={paths.block(n)}>{n.toLocaleString('en-US')}</Link>
}

export function Copy({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      type="button"
      className="btn-icon"
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setDone(true)
          setTimeout(() => setDone(false), 1500)
        })
      }}
      aria-label={`${label}: ${text}`}
    >
      {done ? 'Copied' : label}
    </button>
  )
}

export function Loading({ what = 'Loading' }: { what?: string }) {
  return (
    <p className="loading" role="status" aria-live="polite">
      {what}…
    </p>
  )
}

export function ErrorBox({ error, onRetry }: { error: Error; onRetry?: () => void }) {
  const notFound = error instanceof ApiError && error.status === 404
  return (
    <div className={`notice ${notFound ? 'notice-info' : 'notice-danger'}`} role="alert">
      <p>{notFound ? `Not found: ${error.message}` : error.message}</p>
      {onRetry && !notFound ? (
        <button type="button" className="btn" onClick={onRetry}>
          Try again
        </button>
      ) : null}
    </div>
  )
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="empty">{children}</p>
}

export function KV({ rows }: { rows: [ReactNode, ReactNode][] }) {
  return (
    <dl className="kv">
      {rows.map(([k, v], i) => (
        <div className="kv-row" key={i}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  )
}

export function Section({ title, children, actions, id }: { title: ReactNode; children: ReactNode; actions?: ReactNode; id?: string }) {
  return (
    <section className="card" aria-labelledby={id}>
      <header className="card-head">
        <h2 id={id}>{title}</h2>
        {actions}
      </header>
      {children}
    </section>
  )
}

export function LoadMore({ hasMore, loading, onClick }: { hasMore: boolean; loading: boolean; onClick: () => void }) {
  if (!hasMore && !loading) return null
  return (
    <div className="load-more">
      <button type="button" className="btn" onClick={onClick} disabled={loading || !hasMore}>
        {loading ? 'Loading…' : 'Load more'}
      </button>
    </div>
  )
}

export function SpamBadges({ reasons, lookAlikeOf }: { reasons: string[]; lookAlikeOf?: string | null }) {
  if (!reasons.length) return null
  return (
    <span className="badges">
      {reasons.map((r) => (
        <Badge key={r} tone={r === 'look-alike' || r === 'impersonating-token' ? 'danger' : 'warn'} title={r === 'look-alike' && lookAlikeOf ? `Imitates ${lookAlikeOf}` : undefined}>
          {spamLabel(r)}
        </Badge>
      ))}
    </span>
  )
}

const spamLabel = (r: string) =>
  ({ 'zero-value': 'Zero value', dust: 'Dust', 'look-alike': 'Look-alike', 'unknown-token': 'Unknown token', 'impersonating-token': 'Fake token' })[r] ?? r

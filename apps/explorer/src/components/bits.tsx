'use client'
// Small explorer building blocks on top of @xdevs/ui.

import NextLink from 'next/link'
import { useEffect, useState, type AnchorHTMLAttributes, type ReactNode } from 'react'
import { Badge, CopyButton, Hash, type BadgeTone } from '@xdevs/ui'
import { config } from '../lib/config'
import { formatDate, formatNative, timeAgo, type Tone } from '../lib/format'
import { addressParts } from '../lib/lookalike'
import { isDynamicHref, paths } from '../lib/routes'
import type { Source } from '../lib/types'
import { knownLabel } from '../lib/labels'

/** next/link for static pages; a plain <a> (full load) for placeholder-served dynamic pages. */
export function Link({ href, children, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  if (isDynamicHref(href))
    return (
      <a href={href} {...rest}>
        {children}
      </a>
    )
  return (
    <NextLink href={href} {...rest}>
      {children}
    </NextLink>
  )
}

export const toneOf = (t: Tone): BadgeTone => ({ ok: 'final', warn: 'warn', danger: 'alert', info: 'neutral', neutral: 'dim' } as const)[t]

export function Pill({ tone = 'neutral', children, title, dot }: { tone?: Tone; children: ReactNode; title?: string; dot?: boolean }) {
  return (
    <Badge tone={toneOf(tone)} title={title} dot={dot}>
      {children}
    </Badge>
  )
}

/** Address with optional XNS name / label; links to its page; copy button. */
export function Addr({ address, name, full = false, link = true, copy = true, plain = false }: { address: string | null | undefined; name?: string | null; full?: boolean; link?: boolean; copy?: boolean; plain?: boolean }) {
  if (!address) return <span className="x-muted">—</span>
  const label = plain ? null : (name ?? knownLabel(address))
  if (full) {
    const p = addressParts(address)
    return (
      <span className="x-addr-full">
        {label ? <span className="x-xns">{label}</span> : null}
        <span className="xu-hash is-full">
          <a href={link ? paths.address(address) : undefined} className="xu-hash-text" aria-label={address} title={address}>
            <b>{p.head}</b>
            <span className="x-addr-mid">{p.middle}</span>
            <b>{p.tail}</b>
          </a>
          {copy ? <CopyButton value={address} title="Copy address" /> : null}
        </span>
      </span>
    )
  }
  return (
    <Hash value={address} href={link ? paths.address(address) : undefined} head={6} tail={6} copy={copy}>
      {label ? <span className="x-xns">{label}</span> : undefined}
    </Hash>
  )
}

export function TxHash({ hash, copy = false, full = false }: { hash: string; copy?: boolean; full?: boolean }) {
  return <Hash value={hash} href={paths.tx(hash)} head={8} tail={6} copy={copy} full={full} />
}

export function BlockNum({ n }: { n: number }) {
  return (
    <a href={paths.block(n)} className="x-num x-link">
      {n.toLocaleString('en-US')}
    </a>
  )
}

/** "12 s ago" that keeps ticking; the absolute UTC time is in the tooltip. */
export function Age({ ts, now }: { ts: number; now: number }) {
  return (
    <time dateTime={new Date(ts * 1000).toISOString()} title={formatDate(ts)} className="x-age">
      {now ? timeAgo(ts, now) : ' '}
    </time>
  )
}

export function Native({ value, digits = 6, strong }: { value: string | bigint; digits?: number; strong?: boolean }) {
  const [num, sym] = [formatNative(value, '', digits).trim(), config.nativeSymbol]
  const Tag = strong ? 'strong' : 'span'
  return (
    <Tag className="x-num">
      {num} <span className="x-unit">{sym}</span>
    </Tag>
  )
}

/** Seconds since epoch, ticking every `ms`. Starts at 0 so the server render and first client render agree. */
export function useNow(ms = 1000): number {
  const [now, setNow] = useState(0)
  useEffect(() => {
    setNow(Math.floor(Date.now() / 1000))
    const id = setInterval(() => setNow(Math.floor(Date.now() / 1000)), ms)
    return () => clearInterval(id)
  }, [ms])
  return now
}

export function PageHead({ eyebrow, title, children, actions }: { eyebrow?: ReactNode; title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="x-head">
      <div className="x-head-main">
        {eyebrow ? <p className="x-eyebrow">{eyebrow}</p> : null}
        <h1 className="x-title">{title}</h1>
        {children ? <div className="x-head-sub">{children}</div> : null}
      </div>
      {actions ? <div className="x-head-actions">{actions}</div> : null}
    </header>
  )
}

/** Definition list: label column + value column, stacked on narrow screens. */
export function KV({ rows }: { rows: ([ReactNode, ReactNode] | null | false)[] }) {
  return (
    <dl className="x-kv">
      {rows.filter(Boolean).map((r, i) => {
        const [k, v] = r as [ReactNode, ReactNode]
        return (
          <div className="x-kv-row" key={i}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        )
      })}
    </dl>
  )
}

const SOURCE_TEXT: Record<Source, string> = {
  api: 'Indexed by the explorer API',
  rpc: 'Read directly from the node (indexer offline)',
  demo: 'Demo data',
}

/** Where the page's data came from (small, right-aligned under the content). */
export function SourceNote({ source, text }: { source: Source; text?: string }) {
  if (source === 'api') return null
  return (
    <p className={`x-source is-${source}`}>
      <span className="x-source-dot" aria-hidden="true" />
      {text ?? SOURCE_TEXT[source]}
    </p>
  )
}

export function Notice({ tone = 'info', title, children, role }: { tone?: 'info' | 'warn' | 'danger' | 'ok'; title?: ReactNode; children: ReactNode; role?: 'alert' | 'note' | 'status' }) {
  return (
    <div className={`x-notice is-${tone}`} role={role ?? (tone === 'danger' ? 'alert' : 'note')}>
      {title ? <p className="x-notice-title">{title}</p> : null}
      <div className="x-notice-body">{children}</div>
    </div>
  )
}

export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={`x-mono ${className ?? ''}`}>{children}</span>
}

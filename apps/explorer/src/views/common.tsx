'use client'

import { EmptyState, ErrorState, Skeleton } from '@xdevs/ui'
import { useEffect, type ReactNode } from 'react'
import { NotReachable } from '../components/NotReachable'
import { config } from '../lib/config'
import { NotFoundError } from '../lib/data'
import { isUnreachable, networkDown, useNet } from '../lib/net'

export function Page({ children, wide }: { children: ReactNode; wide?: boolean }) {
  return <div className={`xu-page x-page${wide ? ' is-wide' : ''}`}>{children}</div>
}

/** Sets the document title (static export: one HTML file serves every value of a dynamic route). */
export function useTitle(title: string | null) {
  useEffect(() => {
    if (title) document.title = `${title} · xDEVS Explorer`
  }, [title])
}

/**
 * Error / unreachable / not-found handling shared by the detail views. Returns null when there is nothing to say.
 */
export function Problem({ error, onRetry, what }: { error: Error | null; onRetry?: () => void; what?: string }) {
  const net = useNet()
  if (!error) return networkDown(net) ? <NotReachable what={what} /> : null
  if (error instanceof NotFoundError) return <EmptyState title="Not found" description={error.message} action={<a className="xh-btn xh-btn-ghost xu-btn is-sm" href="/">Back to the overview</a>} />
  if (!config.demo && (isUnreachable(error) || networkDown(net))) return <NotReachable what={what} />
  return <ErrorState title="Could not load this page" description="The node or the indexer answered with an error." error={error} onRetry={onRetry} />
}

export function DetailSkeleton({ rows = 8 }: { rows?: number }) {
  return (
    <div className="x-skel-detail" aria-busy="true" aria-live="polite">
      <span className="xh-sr">Loading…</span>
      <Skeleton width={120} height={12} />
      <div style={{ height: 10 }} />
      <Skeleton width="min(520px, 80%)" height={30} />
      <div className="x-skel-card">
        {Array.from({ length: rows }, (_, i) => (
          <div key={i} className="x-skel-row">
            <Skeleton width={110} height={12} />
            <Skeleton width={`${40 + ((i * 37) % 45)}%`} height={12} />
          </div>
        ))}
      </div>
    </div>
  )
}

/** Invalid / missing route value (someone opened /tx/_ directly). */
export function MissingParam({ what, example }: { what: string; example: string }) {
  return <EmptyState title={`No ${what} in the address bar`} description={<>Open a {what} from a list or search for one, for example <code className="x-mono">{example}</code>.</>} />
}

/** Loading state; turns into the "not reachable" state when both services are found down meanwhile. */
export function Pending({ what, rows }: { what?: string; rows?: number }) {
  const net = useNet()
  return networkDown(net) ? <NotReachable what={what} /> : <DetailSkeleton rows={rows} />
}

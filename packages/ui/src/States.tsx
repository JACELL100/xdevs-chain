import type { ReactNode } from 'react'
import { cx } from './format'

export type EmptyStateProps = {
  title: ReactNode
  description?: ReactNode
  /** Icon in the circle (defaults to an empty-box glyph). */
  icon?: ReactNode
  /** Buttons or links under the text. */
  action?: ReactNode
  className?: string
}

/** "Nothing here yet" block for empty lists and searches without results. */
export function EmptyState({ title, description, icon, action, className }: EmptyStateProps) {
  return (
    <div className={cx('xu-state', className)}>
      <span className="xu-state-icon" aria-hidden="true">
        {icon ?? (
          <svg viewBox="0 0 16 16" width="18" height="18">
            <path d="M2.5 6 8 3l5.5 3v5L8 14l-5.5-3Z M2.5 6 8 9l5.5-3M8 9v5" fill="none" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        )}
      </span>
      <p className="xu-state-title">{title}</p>
      {description ? <p className="xu-state-text">{description}</p> : null}
      {action ? <div className="xu-state-actions">{action}</div> : null}
    </div>
  )
}

export type ErrorStateProps = {
  title?: ReactNode
  description?: ReactNode
  /** The caught error: its message is shown in small mono text. */
  error?: unknown
  /** Shows a "Try again" button (only from client components: functions cannot be passed from the server). */
  onRetry?: () => void
  retryLabel?: string
  action?: ReactNode
  className?: string
}

function errorMessage(error: unknown): string | undefined {
  if (error == null) return undefined
  if (error instanceof Error) return (error as Error & { shortMessage?: string }).shortMessage ?? error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error)
  } catch {
    return String(error)
  }
}

/** Failure block (RPC unreachable, not found, …) with optional retry. */
export function ErrorState({
  title = 'Something went wrong',
  description,
  error,
  onRetry,
  retryLabel = 'Try again',
  action,
  className,
}: ErrorStateProps) {
  const detail = errorMessage(error)
  return (
    <div className={cx('xu-state is-error', className)} role="alert">
      <span className="xu-state-icon" aria-hidden="true">
        <svg viewBox="0 0 16 16" width="18" height="18">
          <path d="M8 4.5v4.5M8 11v.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      </span>
      <p className="xu-state-title">{title}</p>
      {description ? <p className="xu-state-text">{description}</p> : null}
      {detail ? <p className="xu-state-detail">{detail}</p> : null}
      {onRetry || action ? (
        <div className="xu-state-actions">
          {onRetry ? (
            <button type="button" className="xh-btn xh-btn-ghost xu-btn is-sm" onClick={onRetry}>
              {retryLabel}
            </button>
          ) : null}
          {action}
        </div>
      ) : null}
    </div>
  )
}

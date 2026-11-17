import type { HTMLAttributes } from 'react'
import { cx } from './format'

export type BadgeTone = 'neutral' | 'final' | 'warn' | 'alert' | 'dim'

export type BadgeProps = HTMLAttributes<HTMLSpanElement> & {
  /** `final` (teal) = success/final/safe, `warn` = pending/degraded, `alert` = failed/danger. */
  tone?: BadgeTone
  /** Leading status dot. */
  dot?: boolean
}

/** Small mono pill for states such as "final", "pending", "reverted". */
export function Badge({ tone = 'neutral', dot, className, children, ...rest }: BadgeProps) {
  return (
    <span {...rest} className={cx('xu-badge', tone !== 'neutral' && `is-${tone}`, className)}>
      {dot ? <span className="xh-dot" aria-hidden="true" /> : null}
      {children}
    </span>
  )
}

import type { ReactNode } from 'react'
import { cx } from './format'
import { Skeleton } from './Skeleton'

export type StatProps = {
  label: ReactNode
  value: ReactNode
  hint?: ReactNode
  /** Shows a skeleton instead of the value. */
  loading?: boolean
  className?: string
}

/** Label + big tabular value (e.g. "block height 1,204,881"). */
export function Stat({ label, value, hint, loading, className }: StatProps) {
  return (
    <div className={cx('xu-stat', className)}>
      <span className="xu-stat-label">{label}</span>
      <span className="xu-stat-value">{loading ? <Skeleton width="60%" height={26} /> : value}</span>
      {hint ? <span className="xu-stat-hint">{hint}</span> : null}
    </div>
  )
}

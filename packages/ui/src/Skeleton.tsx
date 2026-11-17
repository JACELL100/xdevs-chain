import type { CSSProperties } from 'react'
import { cx } from './format'

export type SkeletonProps = {
  width?: number | string
  height?: number | string
  radius?: number | string
  /** Render N stacked lines instead of one block. */
  lines?: number
  className?: string
}

/** Shimmering placeholder while data loads. Purely visual (aria-hidden). */
export function Skeleton({ width = '100%', height = 14, radius, lines, className }: SkeletonProps) {
  const style: CSSProperties = { width, height, ...(radius !== undefined ? { borderRadius: radius } : {}) }
  if (lines && lines > 1)
    return (
      <span style={{ display: 'grid', gap: 8 }} aria-hidden="true">
        {Array.from({ length: lines }, (_, i) => (
          <span key={i} className={cx('xu-skel', className)} style={{ ...style, width: i === lines - 1 ? '70%' : width }} />
        ))}
      </span>
    )
  return <span className={cx('xu-skel', className)} style={style} aria-hidden="true" />
}

import type { Key, ReactNode } from 'react'
import { cx } from './format'
import { Skeleton } from './Skeleton'

export type TableColumn<T> = {
  key: string
  header: ReactNode
  /** Cell renderer (defaults to `row[key]`). */
  render?: (row: T, index: number) => ReactNode
  align?: 'left' | 'right' | 'center'
  /** Monospace cell text (hashes, numbers). */
  mono?: boolean
  width?: number | string
}

export type TableProps<T> = {
  columns: TableColumn<T>[]
  rows: readonly T[]
  rowKey: (row: T, index: number) => Key
  /** Shows `skeletonRows` placeholder rows. */
  loading?: boolean
  skeletonRows?: number
  /** Rendered in a full-width cell when `rows` is empty (and not loading). */
  empty?: ReactNode
  caption?: string
  className?: string
}

/** Data table with horizontal scroll on small screens, skeleton rows and an empty state. */
export function Table<T>({ columns, rows, rowKey, loading, skeletonRows = 5, empty, caption, className }: TableProps<T>) {
  const cellClass = (c: TableColumn<T>) => cx(c.align === 'right' && 'is-right', c.align === 'center' && 'is-center', c.mono && 'is-mono')
  return (
    <div className={cx('xu-table-wrap', className)}>
      <table className="xu-table">
        {caption ? <caption className="xh-sr">{caption}</caption> : null}
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c.key} scope="col" className={cellClass(c) || undefined} style={c.width !== undefined ? { width: c.width } : undefined}>
                {c.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {loading
            ? Array.from({ length: skeletonRows }, (_, i) => (
                <tr key={`skel-${i}`}>
                  {columns.map((c) => (
                    <td key={c.key}>
                      <Skeleton width={c.align === 'right' ? '50%' : '80%'} />
                    </td>
                  ))}
                </tr>
              ))
            : rows.length === 0 && empty
              ? (
                  <tr>
                    <td colSpan={columns.length}>{empty}</td>
                  </tr>
                )
              : rows.map((row, i) => (
                  <tr key={rowKey(row, i)}>
                    {columns.map((c) => (
                      <td key={c.key} className={cellClass(c) || undefined}>
                        {c.render ? c.render(row, i) : ((row as Record<string, ReactNode>)[c.key] ?? null)}
                      </td>
                    ))}
                  </tr>
                ))}
        </tbody>
      </table>
    </div>
  )
}

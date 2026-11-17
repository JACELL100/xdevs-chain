import type { HTMLAttributes, ReactNode } from 'react'
import { cx } from './format'

export type CardProps = Omit<HTMLAttributes<HTMLElement>, 'title'> & {
  title?: ReactNode
  /** Right side of the header (links, buttons, badges). */
  actions?: ReactNode
  /** Remove body padding (e.g. for a full-bleed <Table>). */
  flush?: boolean
  as?: 'section' | 'div' | 'article'
}

/** Panel surface with an optional title bar. */
export function Card({ title, actions, flush, as: Tag = 'section', className, children, ...rest }: CardProps) {
  return (
    <Tag {...rest} className={cx('xu-card', className)}>
      {title || actions ? (
        <header className="xu-card-head">
          {title ? <h2 className="xu-card-title">{title}</h2> : <span />}
          {actions}
        </header>
      ) : null}
      <div className={cx('xu-card-body', flush && 'is-flush')}>{children}</div>
    </Tag>
  )
}

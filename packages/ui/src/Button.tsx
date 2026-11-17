import type { AnchorHTMLAttributes, ButtonHTMLAttributes, ReactNode } from 'react'
import { cx } from './format'
import type { LinkComponent } from './types'

type Common = {
  /** `primary` = platinum pill, `ghost` = outline pill, `quiet` = text button, `danger` = destructive. */
  variant?: 'primary' | 'ghost' | 'quiet' | 'danger'
  /** `sm` 32px, `md` 40px (default), `lg` 48px (the site's hero size). */
  size?: 'sm' | 'md' | 'lg'
  /** Optional trailing icon (e.g. <ArrowIcon />). */
  icon?: ReactNode
  className?: string
  children?: ReactNode
}

export type ButtonAsButton = Common & { href?: undefined } & ButtonHTMLAttributes<HTMLButtonElement>
export type ButtonAsLink = Common & {
  href: string
  /** Opens in a new tab (target=_blank, rel=noopener). */
  external?: boolean
  /** e.g. next/link's Link (client components only). */
  linkComponent?: LinkComponent
} & AnchorHTMLAttributes<HTMLAnchorElement>
export type ButtonProps = ButtonAsButton | ButtonAsLink

const VARIANT = {
  primary: 'xh-btn-primary',
  ghost: 'xh-btn-ghost',
  quiet: 'xu-btn-quiet',
  danger: 'xu-btn-danger',
} as const

function classes(p: Common) {
  const { variant = 'primary', size = 'md', className } = p
  return cx('xh-btn xu-btn', VARIANT[variant], size !== 'lg' && `is-${size}`, className)
}

/** Brand pill button. Renders an <a> when `href` is set, otherwise a <button type="button">. */
export function Button(props: ButtonProps) {
  if (props.href !== undefined) {
    const { variant, size, icon, className, children, external, linkComponent: L, href, ...rest } = props
    const cls = classes({ variant, size, className })
    const content = (
      <>
        {children}
        {icon}
      </>
    )
    if (external)
      return (
        <a {...rest} href={href} className={cls} target="_blank" rel="noopener">
          {content}
        </a>
      )
    return L ? (
      <L {...rest} href={href} className={cls}>
        {content}
      </L>
    ) : (
      <a {...rest} href={href} className={cls}>
        {content}
      </a>
    )
  }
  const { variant, size, icon, className, children, href: _href, type = 'button', ...rest } = props
  return (
    <button {...rest} type={type} className={classes({ variant, size, className })}>
      {children}
      {icon}
    </button>
  )
}

/** Small right arrow used in CTAs. */
export function ArrowIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M3 8h10M9 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  )
}

/** Small external-link glyph. */
export function ExternalIcon() {
  return (
    <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true" style={{ display: 'inline-block', opacity: 0.6 }}>
      <path d="M5 11 11 5M6 5h5v5" fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  )
}

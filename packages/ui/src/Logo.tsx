import { useId } from 'react'
import { cx } from './format'

export type LogoProps = {
  /** Height of the mark in px (the hollow mark needs >= 24; smaller sizes switch to the solid mark). */
  size?: number
  /** `metal` = brushed silver (dark backgrounds), `mono` = currentColor outline, `solid` = filled currentColor. */
  variant?: 'metal' | 'mono' | 'solid'
  /** Render "xDEVS Chain" next to the mark. */
  wordmark?: boolean
  /** Accessible name; pass "" when the logo sits inside a labelled link. */
  title?: string
  className?: string
}

const OUTER_A = 'M30.45 32 61.31 58.6 42.41 58.6 11.55 32 42.41 5.4 61.31 5.4ZM25.55 32 52.69 55.4 43.59 55.4 16.45 32 43.59 8.6 52.69 8.6Z'
const INNER_A = 'M25.55 32 52.69 55.4 43.59 55.4 16.45 32 43.59 8.6 52.69 8.6Z'
const OUTER_B = 'M2.69 5.4 21.59 5.4 52.45 32 21.59 58.6 2.69 58.6 33.55 32ZM11.31 8.6 20.41 8.6 47.55 32 20.41 55.4 11.31 55.4 38.45 32Z'
const INNER_B = 'M11.31 8.6 20.41 8.6 47.55 32 20.41 55.4 11.31 55.4 38.45 32Z'
const MASK_A = 'M-1.61 3.8 22.19 3.8 58.61 35.2 34.81 35.2Z'
const MASK_B = 'M5.39 28.8 41.81 60.2 65.61 60.2 29.19 28.8Z'

/** The xDEVS Chain mark (two interlocked chevrons forming an X), as inline SVG: no asset file needed. */
export function Mark({ size = 26, variant = 'metal', title = 'xDEVS Chain', className }: Omit<LogoProps, 'wordmark'>) {
  // useId is identical on server and client; strip characters that would break url(#…) references.
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '')
  const g = `xm-g-${uid}`
  const a = `xm-a-${uid}`
  const b = `xm-b-${uid}`
  const label = title ? { role: 'img', 'aria-label': title } : { 'aria-hidden': true as const }
  const solid = variant === 'solid' || size < 24

  if (solid)
    return (
      <svg viewBox="0 0 64 64" width={size} height={size} className={className} {...label}>
        {title ? <title>{title}</title> : null}
        <defs>
          <mask id={a} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
            <rect x="-8" y="-8" width="80" height="80" fill="#fff" />
            <path d="M0 4.4 21.97 4.4 57 34.6 35.03 34.6Z" />
          </mask>
          <mask id={b} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
            <rect x="-8" y="-8" width="80" height="80" fill="#fff" />
            <path d="M7 29.4 42.03 59.6 64 59.6 28.97 29.4Z" />
          </mask>
        </defs>
        <path fillRule="evenodd" d="M28 32 57 57 43 57 14 32 43 7 57 7Z" fill="currentColor" mask={`url(#${a})`} />
        <path fillRule="evenodd" d="M7 7 21 7 50 32 21 57 7 57 36 32Z" fill="currentColor" mask={`url(#${b})`} />
      </svg>
    )

  const fill = variant === 'metal' ? `url(#${g})` : 'currentColor'
  return (
    <svg viewBox="0 0 64 64" width={size} height={size} className={className} {...label}>
      {title ? <title>{title}</title> : null}
      <defs>
        {variant === 'metal' ? (
          <linearGradient id={g} gradientUnits="userSpaceOnUse" x1="6" y1="4" x2="58" y2="60">
            <stop offset="0" stopColor="#ffffff" />
            <stop offset="0.18" stopColor="#d9dde3" />
            <stop offset="0.38" stopColor="#f4f6f8" />
            <stop offset="0.55" stopColor="#a7aeb8" />
            <stop offset="0.72" stopColor="#e6e9ed" />
            <stop offset="0.88" stopColor="#b4bbc4" />
            <stop offset="1" stopColor="#7f8792" />
          </linearGradient>
        ) : null}
        <mask id={a} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
          <rect x="-8" y="-8" width="80" height="80" fill="#fff" />
          <path d={MASK_A} />
        </mask>
        <mask id={b} maskUnits="userSpaceOnUse" x="-8" y="-8" width="80" height="80">
          <rect x="-8" y="-8" width="80" height="80" fill="#fff" />
          <path d={MASK_B} />
        </mask>
      </defs>
      <path d={INNER_A} fill={fill} fillOpacity={0.1} mask={`url(#${a})`} />
      <path fillRule="evenodd" d={OUTER_A} fill={fill} mask={`url(#${a})`} />
      <path d={INNER_B} fill={fill} fillOpacity={0.1} mask={`url(#${b})`} />
      <path fillRule="evenodd" d={OUTER_B} fill={fill} mask={`url(#${b})`} />
    </svg>
  )
}

/** Mark + optional "xDEVS Chain" wordmark (HTML text in Geist, so it inherits the app font). */
export function Logo({ size = 26, variant = 'metal', wordmark = true, title, className }: LogoProps) {
  if (!wordmark) return <Mark size={size} variant={variant} title={title ?? 'xDEVS Chain'} className={className} />
  return (
    <span className={cx('xu-logo', className)}>
      <Mark size={size} variant={variant} title="" />
      <span className="xu-brand-word">
        xDEVS <span>Chain</span>
      </span>
    </span>
  )
}

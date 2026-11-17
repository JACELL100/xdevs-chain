import type { ReactNode } from 'react'
import { CopyButton } from './CopyButton'
import { cx, truncateHex } from './format'
import type { LinkComponent } from './types'

export type HashProps = {
  /** Full hex value (tx hash, block hash, address, calldata…). */
  value: string
  /** Makes the text a link (e.g. `/tx/0x…`). */
  href?: string
  /** Characters kept after `0x` / at the end when truncated. */
  head?: number
  tail?: number
  /** Show the full value (wraps) instead of truncating. */
  full?: boolean
  /** Show a copy button (default true). */
  copy?: boolean
  /** Replaces the hex text (e.g. an XNS name); the full value stays in the tooltip and copy button. */
  children?: ReactNode
  /** e.g. next/link's Link (client components only). */
  linkComponent?: LinkComponent
  className?: string
}

/** Monospace hex value: truncated `0x1234…abcd` with tooltip, optional link and copy button. */
export function Hash({ value, href, head = 6, tail = 6, full, copy = true, children, linkComponent: L, className }: HashProps) {
  const text = children ?? (full ? value : truncateHex(value, head, tail))
  const node = href ? (
    L ? (
      <L href={href} className="xu-hash-text" title={value}>
        {text}
      </L>
    ) : (
      <a href={href} className="xu-hash-text" title={value}>
        {text}
      </a>
    )
  ) : (
    <span className="xu-hash-text" title={value}>
      {text}
    </span>
  )
  return (
    <span className={cx('xu-hash', full && 'is-full', className)}>
      {node}
      {copy ? <CopyButton value={value} title="Copy" /> : null}
    </span>
  )
}

export type AddressChipProps = Omit<HashProps, 'value' | 'children'> & {
  address: string
  /** Human label shown instead of the hex (XNS name, "Staking", "You"); the hex stays in the tooltip. */
  label?: ReactNode
}

/** Address shown as `0x1234…abcd` (or a label) with copy button and optional link to its page. */
export function AddressChip({ address, label, head = 4, tail = 4, ...rest }: AddressChipProps) {
  return (
    <Hash value={address} head={head} tail={tail} {...rest}>
      {label ? <span className="xu-addr-label">{label}</span> : undefined}
    </Hash>
  )
}

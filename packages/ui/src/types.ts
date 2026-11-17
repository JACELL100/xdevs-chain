import type { AnchorHTMLAttributes, ComponentType } from 'react'

/**
 * Anything that renders like an <a>: pass `next/link`'s `Link` to get client-side navigation.
 * Only pass it from a client component (functions cannot cross the server -> client boundary).
 */
export type LinkComponent = ComponentType<AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }>

/** One header navigation entry. */
export type NavItem = {
  label: string
  href: string
  /** Path prefix that marks this item active (defaults to `href`; `/` only matches exactly). */
  match?: string
  /** Opens in a new tab with an external-link glyph. */
  external?: boolean
}

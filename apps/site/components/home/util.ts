import type { CSSProperties } from 'react'

/** Props for a scroll-reveal element with an optional stagger delay. */
export function rv(delay = 0): { 'data-reveal': true; style?: CSSProperties } {
  return delay
    ? { 'data-reveal': true, style: { ['--rv-delay' as string]: `${delay}ms` } as CSSProperties }
    : { 'data-reveal': true }
}

/** Typed helper for CSS custom properties in style props. */
export const vars = (o: Record<string, string | number>) => o as CSSProperties

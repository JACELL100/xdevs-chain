// URL helpers. Dynamic routes (/block/123, /tx/0x…, /address/0x…, /token/0x…, /name/alice.xdevs) are exported once
// with the placeholder param "_" and served for every value by Cloudflare Pages rewrites (public/_redirects). The
// page reads the real value from window.location after hydration.

import { useEffect, useState } from 'react'

export const paths = {
  home: () => '/',
  blocks: () => '/blocks',
  txs: () => '/txs',
  validators: () => '/validators',
  block: (id: string | number) => `/block/${id}`,
  tx: (h: string) => `/tx/${h}`,
  address: (a: string, tab?: string) => `/address/${a}${tab ? `?tab=${tab}` : ''}`,
  token: (a: string) => `/token/${a}`,
  name: (n: string) => `/name/${encodeURIComponent(n)}`,
  search: (q: string) => `/search?q=${encodeURIComponent(q)}`,
}

export const DYNAMIC_SEGMENTS = ['block', 'tx', 'address', 'token', 'name'] as const
export type DynamicSegment = (typeof DYNAMIC_SEGMENTS)[number]

/** True for URLs served by a placeholder page (they need a full page load, not a client-side transition). */
export function isDynamicHref(href: string): boolean {
  const seg = href.split(/[?#]/)[0]!.split('/').filter(Boolean)
  return seg.length === 2 && (DYNAMIC_SEGMENTS as readonly string[]).includes(seg[0]!)
}

/** Value of `/<segment>/<value>` in `pathname`, decoded; `null` for the placeholder or a malformed path. */
export function paramFromPath(pathname: string, segment: DynamicSegment): string | null {
  const parts = pathname.replace(/\/+$/, '').split('/').filter(Boolean)
  // Pages may also be served as /block/_.html or /block/_ directly (no rewrite): no value then.
  const i = parts.indexOf(segment)
  const raw = i >= 0 ? parts[i + 1] : undefined
  if (!raw) return null
  let v: string
  try {
    v = decodeURIComponent(raw)
  } catch {
    v = raw
  }
  v = v.replace(/\.html$/, '')
  return v === '_' || v === '' ? null : v
}

/** Client-side param: `undefined` before hydration, then the value (or null when absent). */
export function useRouteParam(segment: DynamicSegment): string | null | undefined {
  const [v, setV] = useState<string | null | undefined>(undefined)
  useEffect(() => {
    const read = () => setV(paramFromPath(window.location.pathname, segment))
    read()
    window.addEventListener('popstate', read)
    return () => window.removeEventListener('popstate', read)
  }, [segment])
  return v
}

export function useQueryParam(name: string): string | null | undefined {
  const [v, setV] = useState<string | null | undefined>(undefined)
  useEffect(() => {
    const read = () => setV(new URLSearchParams(window.location.search).get(name))
    read()
    window.addEventListener('popstate', read)
    return () => window.removeEventListener('popstate', read)
  }, [name])
  return v
}

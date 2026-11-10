// A minimal History-API router (no dependency). Cloudflare Pages serves index.html for unknown paths when the build
// has no 404.html, so deep links work.

import { useSyncExternalStore } from 'react'

export type Route =
  | { page: 'home' }
  | { page: 'blocks' }
  | { page: 'txs' }
  | { page: 'block'; id: string }
  | { page: 'tx'; hash: string }
  | { page: 'address'; address: string; tab: string | null }
  | { page: 'name'; name: string }
  | { page: 'search'; q: string }
  | { page: 'notfound' }

export function parseRoute(pathname: string, search = ''): Route {
  const path = pathname.replace(/\/+$/, '') || '/'
  const seg = path.split('/').filter(Boolean).map((s) => {
    try {
      return decodeURIComponent(s)
    } catch {
      return s
    }
  })
  const params = new URLSearchParams(search)
  if (seg.length === 0) return { page: 'home' }
  const [head, id] = seg
  if (seg.length === 1 && head === 'blocks') return { page: 'blocks' }
  if (seg.length === 1 && head === 'txs') return { page: 'txs' }
  if (seg.length === 1 && head === 'search') return { page: 'search', q: params.get('q') ?? '' }
  if (seg.length === 2 && id) {
    if (head === 'block') return { page: 'block', id }
    if (head === 'tx') return { page: 'tx', hash: id }
    if (head === 'address') return { page: 'address', address: id, tab: params.get('tab') }
    if (head === 'name') return { page: 'name', name: id }
  }
  return { page: 'notfound' }
}

const listeners = new Set<() => void>()
const subscribe = (fn: () => void) => {
  listeners.add(fn)
  window.addEventListener('popstate', fn)
  return () => {
    listeners.delete(fn)
    window.removeEventListener('popstate', fn)
  }
}
const snapshot = () => window.location.pathname + window.location.search

export function navigate(to: string, opts: { replace?: boolean } = {}) {
  if (to === snapshot()) return
  if (opts.replace) window.history.replaceState(null, '', to)
  else window.history.pushState(null, '', to)
  for (const fn of listeners) fn()
  if (!opts.replace) window.scrollTo(0, 0)
}

export function useLocation(): string {
  return useSyncExternalStore(subscribe, snapshot, () => '/')
}

export const paths = {
  home: () => '/',
  blocks: () => '/blocks',
  txs: () => '/txs',
  block: (id: string | number) => `/block/${id}`,
  tx: (h: string) => `/tx/${h}`,
  address: (a: string, tab?: string) => `/address/${a}${tab ? `?tab=${tab}` : ''}`,
  name: (n: string) => `/name/${encodeURIComponent(n)}`,
  search: (q: string) => `/search?q=${encodeURIComponent(q)}`,
}

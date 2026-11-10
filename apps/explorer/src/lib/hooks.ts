import { useCallback, useEffect, useRef, useState } from 'react'

export interface AsyncState<T> {
  data: T | null
  error: Error | null
  loading: boolean
  reload: () => void
}

/** Runs `fn` when `deps` change; aborts the previous run. `refreshMs` re-runs it while the tab is visible. */
export function useAsync<T>(fn: (signal: AbortSignal) => Promise<T>, deps: unknown[], refreshMs = 0): AsyncState<T> {
  const [state, setState] = useState<{ data: T | null; error: Error | null; loading: boolean }>({ data: null, error: null, loading: true })
  const [tick, setTick] = useState(0)
  const fnRef = useRef(fn)
  fnRef.current = fn

  useEffect(() => {
    const ctrl = new AbortController()
    setState((s) => ({ ...s, loading: true }))
    fnRef.current(ctrl.signal).then(
      (data) => !ctrl.signal.aborted && setState({ data, error: null, loading: false }),
      (error: Error) => !ctrl.signal.aborted && error.name !== 'AbortError' && setState((s) => ({ data: s.data, error, loading: false })),
    )
    return () => ctrl.abort()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick])

  useEffect(() => {
    if (!refreshMs) return
    const id = setInterval(() => {
      if (document.visibilityState === 'visible') setTick((t) => t + 1)
    }, refreshMs)
    return () => clearInterval(id)
  }, [refreshMs])

  // Reset when the identity (deps) changes, so a new page never shows the previous page's data.
  const key = JSON.stringify(deps)
  const lastKey = useRef(key)
  if (lastKey.current !== key) {
    lastKey.current = key
    if (state.data !== null) setState({ data: null, error: null, loading: true })
  }

  const reload = useCallback(() => setTick((t) => t + 1), [])
  return { ...state, reload }
}

export interface PagedState<T> {
  items: T[]
  hiddenSpam: number | null
  error: Error | null
  loading: boolean
  hasMore: boolean
  loadMore: () => void
}

/** Cursor pagination with "Load more". Resets when `deps` change. */
export function usePaged<T>(
  fetchPage: (cursor: string | null, signal: AbortSignal) => Promise<{ items: T[]; nextCursor: string | null; hiddenSpam?: number | null }>,
  deps: unknown[],
): PagedState<T> {
  const [items, setItems] = useState<T[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [next, setNext] = useState<string | null>(null)
  const [hiddenSpam, setHidden] = useState<number | null>(null)
  const [error, setError] = useState<Error | null>(null)
  const [loading, setLoading] = useState(true)
  const fnRef = useRef(fetchPage)
  fnRef.current = fetchPage
  const key = JSON.stringify(deps)

  useEffect(() => {
    setItems([])
    setCursor(null)
    setNext(null)
    setHidden(null)
  }, [key])

  useEffect(() => {
    const ctrl = new AbortController()
    setLoading(true)
    setError(null)
    fnRef.current(cursor, ctrl.signal).then(
      (p) => {
        if (ctrl.signal.aborted) return
        setItems((prev) => (cursor ? [...prev, ...p.items] : p.items))
        setNext(p.nextCursor)
        if (!cursor) setHidden(p.hiddenSpam ?? null)
        setLoading(false)
      },
      (e: Error) => {
        if (ctrl.signal.aborted || e.name === 'AbortError') return
        setError(e)
        setLoading(false)
      },
    )
    return () => ctrl.abort()
  }, [key, cursor])

  return { items, hiddenSpam, error, loading, hasMore: !!next, loadMore: () => next && setCursor(next) }
}

/** Seconds since epoch, ticking every `ms`. */
export function useNow(ms = 1000): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000))
  useEffect(() => {
    const id = setInterval(() => setNow(Math.floor(Date.now() / 1000)), ms)
    return () => clearInterval(id)
  }, [ms])
  return now
}

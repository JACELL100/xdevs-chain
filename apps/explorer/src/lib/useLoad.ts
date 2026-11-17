'use client'
// Data hook for the views: runs `fn` when `key` changes (aborting the previous run), never shows the previous
// key's data, optional refresh while the tab is visible.

import { useCallback, useEffect, useRef, useState } from 'react'

export interface Load<T> {
  data: T | null
  error: Error | null
  loading: boolean
  reload: () => void
}

export function useLoad<T>(fn: ((signal: AbortSignal) => Promise<T>) | null, key: string, refreshMs = 0): Load<T> {
  const [state, setState] = useState<{ key: string; data: T | null; error: Error | null; loading: boolean }>({ key, data: null, error: null, loading: !!fn })
  const [tick, setTick] = useState(0)
  const fnRef = useRef(fn)
  fnRef.current = fn

  useEffect(() => {
    const f = fnRef.current
    if (!f) return
    const ctrl = new AbortController()
    setState((s) => (s.key === key ? { ...s, loading: true } : { key, data: null, error: null, loading: true }))
    f(ctrl.signal).then(
      (data) => !ctrl.signal.aborted && setState({ key, data, error: null, loading: false }),
      (error: Error) => !ctrl.signal.aborted && error?.name !== 'AbortError' && setState((s) => ({ key, data: s.key === key ? s.data : null, error, loading: false })),
    )
    return () => ctrl.abort()
  }, [key, tick, fn === null])

  useEffect(() => {
    if (!refreshMs) return
    const id = setInterval(() => document.visibilityState === 'visible' && setTick((t) => t + 1), refreshMs)
    return () => clearInterval(id)
  }, [refreshMs])

  const reload = useCallback(() => setTick((t) => t + 1), [])
  const same = state.key === key
  return { data: same ? state.data : null, error: same ? state.error : null, loading: same ? state.loading : true, reload }
}

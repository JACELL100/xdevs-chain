'use client'
import { useCallback, useEffect, useRef, useState } from 'react'
import { humanError } from './WalletProvider.tsx'

/** Loads async data, re-running when `key` changes; keeps the previous data while reloading. */
export function useLoad<T>(fn: (() => Promise<T>) | null, key: unknown[]) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const fnRef = useRef(fn)
  fnRef.current = fn
  const seq = useRef(0)
  const reload = useCallback(async () => {
    const f = fnRef.current
    if (!f) return
    const my = ++seq.current
    setLoading(true)
    try {
      const v = await f()
      if (my === seq.current) {
        setData(v)
        setError(null)
      }
    } catch (e) {
      if (my === seq.current) setError(humanError(e))
    } finally {
      if (my === seq.current) setLoading(false)
    }
  }, [])
  useEffect(() => {
    void reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, key)
  return { data, error, loading, reload }
}

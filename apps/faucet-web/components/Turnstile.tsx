'use client'

// Cloudflare Turnstile, explicit rendering. The script is injected only when this component mounts,
// so it loads on the faucet page and nowhere else. The token is verified server-side by the Worker
// (apps/faucet/src/turnstile.ts), which also checks the action name below.

import { useEffect, useRef, useState } from 'react'

const SCRIPT_URL = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'
/** Must match TURNSTILE_ACTION in apps/faucet/src/turnstile.ts. */
export const TURNSTILE_ACTION = 'claim'

type RenderOptions = {
  sitekey: string
  action?: string
  theme?: 'light' | 'dark' | 'auto'
  size?: 'normal' | 'compact' | 'flexible'
  appearance?: 'always' | 'execute' | 'interaction-only'
  callback?: (token: string) => void
  'expired-callback'?: () => void
  'error-callback'?: () => void
  'timeout-callback'?: () => void
}
type TurnstileApi = {
  render(el: HTMLElement, opts: RenderOptions): string
  reset(id?: string): void
  remove(id?: string): void
}
declare global {
  interface Window {
    turnstile?: TurnstileApi
  }
}

let loading: Promise<TurnstileApi> | null = null

function loadTurnstile(): Promise<TurnstileApi> {
  if (window.turnstile) return Promise.resolve(window.turnstile)
  if (loading) return loading
  loading = new Promise<TurnstileApi>((resolve, reject) => {
    const s = document.createElement('script')
    s.src = SCRIPT_URL
    s.async = true
    s.onload = () => (window.turnstile ? resolve(window.turnstile) : reject(new Error('turnstile missing')))
    s.onerror = () => {
      loading = null
      s.remove()
      reject(new Error('turnstile script failed to load'))
    }
    document.head.appendChild(s)
  })
  return loading
}

export type TurnstileProps = {
  siteKey: string
  /** Receives a fresh token, or null when it expired / failed / was reset. */
  onToken: (token: string | null) => void
  /** Change this number to reset the widget (tokens are single use). */
  resetSignal: number
}

export default function Turnstile({ siteKey, onToken, resetSignal }: TurnstileProps) {
  const host = useRef<HTMLDivElement>(null)
  const widget = useRef<string | null>(null)
  const tokenCb = useRef(onToken)
  tokenCb.current = onToken
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    setFailed(false)
    loadTurnstile()
      .then((ts) => {
        if (cancelled || !host.current) return
        widget.current = ts.render(host.current, {
          sitekey: siteKey,
          action: TURNSTILE_ACTION,
          theme: 'dark',
          size: window.innerWidth < 400 ? 'compact' : 'flexible',
          callback: (t) => tokenCb.current(t),
          'expired-callback': () => tokenCb.current(null),
          'timeout-callback': () => tokenCb.current(null),
          'error-callback': () => tokenCb.current(null),
        })
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
      if (widget.current) {
        try {
          window.turnstile?.remove(widget.current)
        } catch {
          /* already gone */
        }
        widget.current = null
      }
    }
  }, [siteKey])

  useEffect(() => {
    if (resetSignal === 0 || !widget.current) return
    try {
      window.turnstile?.reset(widget.current)
    } catch {
      /* ignore */
    }
    tokenCb.current(null)
  }, [resetSignal])

  if (failed) {
    return (
      <p className="fx-captcha-fail" role="alert">
        The captcha could not load. Check that challenges.cloudflare.com is not blocked, then reload the page.
      </p>
    )
  }
  return <div ref={host} className="fx-turnstile" />
}

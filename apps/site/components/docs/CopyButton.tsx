'use client'

import { useEffect, useRef, useState } from 'react'

/** Copies the code of the surrounding `.d-code` block. Reads the DOM, so the code is not duplicated in the payload. */
export default function CopyButton() {
  const ref = useRef<HTMLButtonElement>(null)
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)

  useEffect(() => () => clearTimeout(timer.current), [])

  async function copy() {
    const code = ref.current?.closest('.d-code')?.querySelector('pre code') as HTMLElement | null
    if (!code) return
    const text = code.innerText.replace(/\n$/, '')
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.setAttribute('readonly', '')
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      ta.remove()
    }
    setCopied(true)
    clearTimeout(timer.current)
    timer.current = setTimeout(() => setCopied(false), 1600)
  }

  return (
    <button ref={ref} type="button" className="d-copy" data-copied={copied || undefined} onClick={copy} aria-label={copied ? 'Copied' : 'Copy code'}>
      {copied ? (
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      ) : (
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <rect x="5.25" y="5.25" width="8" height="8" rx="1.75" fill="none" stroke="currentColor" strokeWidth="1.3" />
          <path d="M10.75 3.25v-.5a1.5 1.5 0 0 0-1.5-1.5h-5a1.5 1.5 0 0 0-1.5 1.5v5a1.5 1.5 0 0 0 1.5 1.5h.5" fill="none" stroke="currentColor" strokeWidth="1.3" />
        </svg>
      )}
      <span className="d-copy-text" aria-live="polite">
        {copied ? 'Copied' : 'Copy'}
      </span>
    </button>
  )
}

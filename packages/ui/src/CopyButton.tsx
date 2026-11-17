'use client'

import { useEffect, useRef, useState } from 'react'
import { cx } from './format'

export type CopyButtonProps = {
  /** Text put on the clipboard. */
  value: string
  /** Visible label (icon-only when omitted). */
  label?: string
  /** Accessible name / tooltip for the icon-only button. */
  title?: string
  className?: string
}

async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    /* fall back below (insecure context, denied permission) */
  }
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.setAttribute('readonly', '')
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    const ok = document.execCommand('copy')
    ta.remove()
    return ok
  } catch {
    return false
  }
}

/** Copies `value` to the clipboard and confirms with a check mark for 1.5 s. */
export function CopyButton({ value, label, title = 'Copy', className }: CopyButtonProps) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => () => clearTimeout(timer.current), [])

  return (
    <button
      type="button"
      className={cx('xu-copy', label && 'has-label', className)}
      data-copied={copied}
      aria-label={label ? undefined : copied ? 'Copied' : title}
      title={copied ? 'Copied' : title}
      onClick={async (e) => {
        e.preventDefault()
        e.stopPropagation()
        if (await copyText(value)) {
          setCopied(true)
          clearTimeout(timer.current)
          timer.current = setTimeout(() => setCopied(false), 1500)
        }
      }}
    >
      {copied ? (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3 8.5 6.5 12 13 4.5" fill="none" stroke="currentColor" strokeWidth="1.6" />
        </svg>
      ) : (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="5.5" y="5.5" width="8" height="8" rx="1.6" fill="none" stroke="currentColor" strokeWidth="1.3" />
          <path d="M10.5 3.5v-.4A1.6 1.6 0 0 0 8.9 1.5H4.1A1.6 1.6 0 0 0 2.5 3.1v4.8a1.6 1.6 0 0 0 1.6 1.6h.4" fill="none" stroke="currentColor" strokeWidth="1.3" />
        </svg>
      )}
      {label ? <span aria-live="polite">{copied ? 'Copied' : label}</span> : null}
    </button>
  )
}

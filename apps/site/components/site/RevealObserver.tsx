'use client'

import { usePathname } from 'next/navigation'
import { useEffect } from 'react'

/**
 * Reveals every `[data-reveal]` element once it scrolls into view by adding
 * `.is-in` (styles in app/globals.css). A delay can be set per element with
 * `style={{ '--rv-delay': '120ms' }}`. Render once per page that uses reveals.
 */
export default function RevealObserver() {
  const path = usePathname()
  useEffect(() => {
    const els = Array.from(document.querySelectorAll<HTMLElement>('[data-reveal]:not(.is-in)'))
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      els.forEach((e) => e.classList.add('is-in'))
      return
    }
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            e.target.classList.add('is-in')
            io.unobserve(e.target)
          }
        }
      },
      { rootMargin: '0px 0px -10% 0px', threshold: 0.12 },
    )
    els.forEach((e) => io.observe(e))
    return () => io.disconnect()
  }, [path])
  return null
}

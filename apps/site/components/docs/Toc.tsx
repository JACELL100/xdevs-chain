'use client'

import { useEffect, useState } from 'react'
import type { TocItem } from '@/lib/docs/markdown'

/** "On this page" with scroll-spy: the active item is the last heading above the reading line. */
export default function Toc({ items }: { items: TocItem[] }) {
  const [active, setActive] = useState<string | undefined>(items[0]?.id)

  useEffect(() => {
    const headings = items
      .map((i) => document.getElementById(i.id))
      .filter((h): h is HTMLElement => !!h)
    if (!headings.length) return
    let frame = 0
    const update = () => {
      frame = 0
      const line = 120
      let current = headings[0].id
      for (const h of headings) {
        if (h.getBoundingClientRect().top - line <= 0) current = h.id
        else break
      }
      // At the very bottom, the last section is the one being read.
      if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 4) {
        current = headings[headings.length - 1].id
      }
      setActive(current)
    }
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(update)
    }
    update()
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('resize', onScroll)
    return () => {
      window.removeEventListener('scroll', onScroll)
      window.removeEventListener('resize', onScroll)
      if (frame) cancelAnimationFrame(frame)
    }
  }, [items])

  if (!items.length) return null
  return (
    <nav className="d-toc" aria-label="On this page">
      <p className="d-toc-title">On this page</p>
      <ul>
        {items.map((i) => (
          <li key={i.id} data-depth={i.depth}>
            <a href={`#${i.id}`} aria-current={active === i.id ? 'location' : undefined}>
              {i.text}
            </a>
          </li>
        ))}
      </ul>
    </nav>
  )
}

'use client'

/**
 * Site-wide motion runtime, mounted once in the root layout:
 * - Lenis smooth scrolling for wheel input (native scroll stays underneath, so sticky
 *   sections, anchors and keyboard scrolling work; off for reduced motion and touch);
 * - scroll reveals: every [data-rv] element gets `.in` when it enters the viewport;
 * - the pointer-following highlight on `.glow` cards (CSS variables, no re-render).
 */
import { useEffect } from 'react'
import { usePathname } from 'next/navigation'
import Lenis from 'lenis'

let lenis: Lenis | null = null

export default function MotionRuntime() {
  const pathname = usePathname()

  useEffect(() => {
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reduce) return
    lenis = new Lenis({ lerp: 0.11, smoothWheel: true, anchors: { offset: -80 } })
    let raf = 0
    const loop = (t: number) => {
      lenis?.raf(t)
      raf = requestAnimationFrame(loop)
    }
    raf = requestAnimationFrame(loop)

    const onMove = (e: PointerEvent) => {
      const el = (e.target as Element | null)?.closest?.('.glow') as HTMLElement | null
      if (!el) return
      const r = el.getBoundingClientRect()
      el.style.setProperty('--mx', `${e.clientX - r.left}px`)
      el.style.setProperty('--my', `${e.clientY - r.top}px`)
    }
    document.addEventListener('pointermove', onMove, { passive: true })
    return () => {
      cancelAnimationFrame(raf)
      document.removeEventListener('pointermove', onMove)
      lenis?.destroy()
      lenis = null
    }
  }, [])

  useEffect(() => {
    if (!window.location.hash) lenis?.scrollTo(0, { immediate: true, force: true })
    const els = Array.from(document.querySelectorAll<HTMLElement>('[data-rv]:not(.in)'))
    if (!('IntersectionObserver' in window)) {
      els.forEach((el) => el.classList.add('in'))
      return
    }
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            const el = e.target as HTMLElement
            el.classList.add('in')
            io.unobserve(el)
            // Once revealed, hand transitions back to the element's own styles (hover etc.).
            setTimeout(() => {
              el.removeAttribute('data-rv')
              el.style.removeProperty('--d')
            }, 1000 + Number(el.dataset.d || 0))
          }
        }
      },
      { rootMargin: '0px 0px -8% 0px', threshold: 0.01 },
    )
    els.forEach((el) => {
      if (el.dataset.d) el.style.setProperty('--d', `${el.dataset.d}ms`)
      io.observe(el)
    })
    return () => io.disconnect()
  }, [pathname])

  return null
}

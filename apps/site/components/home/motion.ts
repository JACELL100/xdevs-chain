import { useEffect, useState, type RefObject } from 'react'

/** True when the visitor asked the OS for reduced motion. Call only in effects/handlers. */
export function reducedMotion(): boolean {
  return typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
}

/** "Is this element on screen" flag, used to pause off-screen work. */
export function useInView(ref: RefObject<Element | null>, rootMargin = '0px'): boolean {
  const [inView, setInView] = useState(false)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const io = new IntersectionObserver(([e]) => setInView(e.isIntersecting), { rootMargin })
    io.observe(el)
    return () => io.disconnect()
  }, [ref, rootMargin])
  return inView
}

export const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))
export const clamp = (v: number, a = 0, b = 1) => Math.min(b, Math.max(a, v))

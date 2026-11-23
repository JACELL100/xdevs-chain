'use client'

/** A measured number that counts up once when it scrolls into view. The HTML carries the final value. */
import { useEffect, useRef } from 'react'
import { animate, useInView } from 'motion/react'

export default function CountUp({ to, suffix = '' }: { to: number; suffix?: string }) {
  const ref = useRef<HTMLSpanElement>(null)
  const inView = useInView(ref, { once: true, margin: '0px 0px -10% 0px' })
  useEffect(() => {
    const el = ref.current
    if (!el || !inView || to === 0 || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return
    const c = animate(0, to, {
      duration: 1.6,
      ease: [0.2, 0.8, 0.2, 1],
      onUpdate: (v) => {
        el.textContent = Math.round(v).toLocaleString('en-US') + suffix
      },
    })
    return () => c.stop()
  }, [inView, to, suffix])
  return <span ref={ref}>{to.toLocaleString('en-US') + suffix}</span>
}

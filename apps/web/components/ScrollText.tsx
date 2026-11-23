'use client'

/**
 * A statement that lights up word by word as it scrolls through the viewport
 * (opacity only, driven by scroll position). Words wrapped in *asterisks* turn teal.
 */
import { useRef } from 'react'
import { m, useScroll, useTransform, type MotionValue } from 'motion/react'

function Word({ text, i, n, p, accent }: { text: string; i: number; n: number; p: MotionValue<number>; accent: boolean }) {
  const start = i / n
  const o = useTransform(p, [start, Math.min(1, start + 1.5 / n)], [0.16, 1])
  return (
    <m.span className={`w${accent ? ' is-accent' : ''}`} style={{ opacity: o }}>
      {text}
    </m.span>
  )
}

export default function ScrollText({ text, as: Tag = 'p', className = '' }: { text: string; as?: 'p' | 'h2'; className?: string }) {
  const ref = useRef<HTMLParagraphElement>(null)
  const { scrollYProgress } = useScroll({ target: ref, offset: ['start 0.85', 'end 0.45'] })
  const words = text.split(' ')
  return (
    <Tag ref={ref} className={`scroll-text ${className}`}>
      <span className="sr-only">{text.replaceAll('*', '')}</span>
      <span aria-hidden="true">
        {words.map((w, i) => {
          const accent = w.startsWith('*')
          return (
            <Word key={i} i={i} n={words.length} p={scrollYProgress} accent={accent} text={w.replaceAll('*', '') + (i < words.length - 1 ? ' ' : '')} />
          )
        })}
      </span>
    </Tag>
  )
}

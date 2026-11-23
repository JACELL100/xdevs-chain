'use client'

import type { ReactNode } from 'react'
import { LazyMotion, MotionConfig } from 'motion/react'

// The animation features load after first paint, so they never block it.
const features = () => import('./motion-features').then((r) => r.default)

export default function Providers({ children }: { children: ReactNode }) {
  return (
    <LazyMotion features={features} strict>
      <MotionConfig reducedMotion="user">{children}</MotionConfig>
    </LazyMotion>
  )
}

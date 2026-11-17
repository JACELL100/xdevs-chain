// Biometric glyph for passkey moments: idle (soft), scanning (rings + sweep), done (check). CSS-only motion,
// disabled under prefers-reduced-motion. Decorative: the surrounding text says what is happening.
import { cx } from '@xdevs/ui'

export type PasskeyState = 'idle' | 'scanning' | 'done' | 'error'

export function PasskeyVisual({ state = 'idle', size = 132 }: { state?: PasskeyState; size?: number }) {
  return (
    <div className={cx('w-pk', `is-${state}`)} style={{ width: size, height: size }} aria-hidden="true">
      <span className="w-pk-ring r1" />
      <span className="w-pk-ring r2" />
      <span className="w-pk-core">
        {state === 'done' ? (
          <svg viewBox="0 0 48 48" className="w-pk-check">
            <path d="M14 25l7 7 14-16" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : (
          <svg viewBox="0 0 48 48" className="w-pk-print">
            <g fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
              <path d="M13.5 18.5a12 12 0 0 1 21 0" />
              <path d="M10.5 26a14 14 0 0 1 .9-5" />
              <path d="M37.5 26c0-1.8-.3-3.5-.9-5" />
              <path d="M16.5 34.5c-1.3-2.4-2-5-2-8.5a9.5 9.5 0 0 1 19 0c0 1.5-.1 2.8-.4 4" />
              <path d="M20.6 37.2c-1.6-2.9-2.6-6.6-2.6-11.2a6 6 0 0 1 12 0c0 4.1-.8 7.8-2.4 10.8" />
              <path d="M24 26c0 4.9-1 9-3 12.4" />
              <path d="M26.6 32.3c-.3 1.8-.8 3.5-1.5 5" />
              <path d="M31.4 33.8c-.4 1.5-1 2.9-1.6 4.2" />
              <path d="M17 11.6A15.7 15.7 0 0 1 24 10c2.6 0 5 .6 7.1 1.7" />
            </g>
          </svg>
        )}
        <span className="w-pk-sweep" />
      </span>
    </div>
  )
}

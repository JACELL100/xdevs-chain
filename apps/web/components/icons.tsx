/** Line icons (16/24 grid, 1.5 stroke) used across the site. Decorative: always aria-hidden. */
import type { SVGProps } from 'react'

type P = SVGProps<SVGSVGElement>
const base = (p: P) => ({
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  'aria-hidden': true,
  ...p,
})

export const Arrow = (p: P) => (
  <svg {...base(p)} className={`arrow ${p.className ?? ''}`}>
    <path d="M5 12h14M13 6l6 6-6 6" />
  </svg>
)
export const ArrowUpRight = (p: P) => (
  <svg {...base(p)}>
    <path d="M7 17 17 7M8 7h9v9" />
  </svg>
)
export const Check = (p: P) => (
  <svg {...base(p)}>
    <path d="m5 12.5 4.5 4.5L19 7.5" />
  </svg>
)
export const CheckCircle = (p: P) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="m8 12.3 2.8 2.7L16 9.5" />
  </svg>
)
export const Warn = (p: P) => (
  <svg {...base(p)}>
    <path d="M12 3.5 2.5 20h19L12 3.5Z" />
    <path d="M12 10v4.5M12 17.3v.2" />
  </svg>
)
export const Play = (p: P) => (
  <svg viewBox="0 0 24 24" aria-hidden {...p}>
    <path d="M7 4.5v15l12.5-7.5L7 4.5Z" fill="currentColor" />
  </svg>
)
export const Plus = (p: P) => (
  <svg {...base(p)}>
    <path d="M12 5v14M5 12h14" />
  </svg>
)
export const Copy = (p: P) => (
  <svg {...base(p)}>
    <rect x="8.5" y="8.5" width="11" height="11" rx="2.5" />
    <path d="M15.5 8.5V6a1.5 1.5 0 0 0-1.5-1.5H6A1.5 1.5 0 0 0 4.5 6v8A1.5 1.5 0 0 0 6 15.5h2.5" />
  </svg>
)
export const Fingerprint = (p: P) => (
  <svg {...base(p)}>
    <path d="M7 19.5c1.5-2 2-4.3 2-7.5a3 3 0 0 1 6 0c0 1.2-.1 2.3-.3 3.4" />
    <path d="M12 12c0 3.5-.8 6.3-2.6 8.5M15.9 18.8c-.3.7-.6 1.3-1 1.9" />
    <path d="M4.5 15.5c.3-1.1.5-2.3.5-3.5a7 7 0 0 1 12.7-4" />
    <path d="M19 11c.1.8.1 1.6 0 2.5" />
  </svg>
)
export const Vault = (p: P) => (
  <svg {...base(p)}>
    <rect x="3.5" y="4.5" width="17" height="15" rx="3" />
    <circle cx="12" cy="12" r="3.5" />
    <path d="M12 8.5V7M12 17v-1.5M15.5 12H17M7 12h1.5" />
  </svg>
)
export const Shield = (p: P) => (
  <svg {...base(p)}>
    <path d="M12 3 5 6v5.5c0 4.3 3 7.9 7 9.5 4-1.6 7-5.2 7-9.5V6l-7-3Z" />
    <path d="m9 12 2.2 2.2L15.5 10" />
  </svg>
)
export const Users = (p: P) => (
  <svg {...base(p)}>
    <circle cx="9" cy="8.5" r="3.5" />
    <path d="M2.5 19.5c.8-3.2 3.4-5 6.5-5s5.7 1.8 6.5 5" />
    <path d="M15.5 5.2a3.5 3.5 0 0 1 0 6.6M18 14.8c1.8.7 3 2.3 3.5 4.7" />
  </svg>
)
export const Eye = (p: P) => (
  <svg {...base(p)}>
    <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z" />
    <circle cx="12" cy="12" r="3" />
  </svg>
)
export const Clock = (p: P) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </svg>
)
export const Tag = (p: P) => (
  <svg {...base(p)}>
    <path d="M3.5 12.6V4.5a1 1 0 0 1 1-1h8.1l8 8a1.5 1.5 0 0 1 0 2.1l-6.9 6.9a1.5 1.5 0 0 1-2.1 0l-8.1-7.9Z" />
    <circle cx="8.5" cy="8.5" r="1.5" />
  </svg>
)
export const Bolt = (p: P) => (
  <svg {...base(p)}>
    <path d="M13 2.5 4.5 13.5H12l-1 8 8.5-11H12l1-8Z" />
  </svg>
)
export const Lock = (p: P) => (
  <svg {...base(p)}>
    <rect x="4.5" y="10.5" width="15" height="10" rx="2.5" />
    <path d="M8 10.5V8a4 4 0 0 1 8 0v2.5" />
  </svg>
)
export const Layers = (p: P) => (
  <svg {...base(p)}>
    <path d="m12 3.5 9 5-9 5-9-5 9-5Z" />
    <path d="m3 13 9 5 9-5" />
  </svg>
)
export const Gauge = (p: P) => (
  <svg {...base(p)}>
    <path d="M4 17a8.5 8.5 0 1 1 16 0" />
    <path d="m12 13 4-4.5" />
    <circle cx="12" cy="13.5" r="1.2" />
  </svg>
)
export const Parallel = (p: P) => (
  <svg {...base(p)}>
    <path d="M4 6h10M4 12h16M4 18h12" />
    <path d="m17 4 2 2-2 2M19 16l2 2-2 2" />
  </svg>
)
export const Code = (p: P) => (
  <svg {...base(p)}>
    <path d="m8.5 7-5 5 5 5M15.5 7l5 5-5 5M13.5 4.5l-3 15" />
  </svg>
)
export const Terminal = (p: P) => (
  <svg {...base(p)}>
    <rect x="2.5" y="4" width="19" height="16" rx="3" />
    <path d="m7 9.5 3 2.5-3 2.5M12.5 15H17" />
  </svg>
)
export const Cube = (p: P) => (
  <svg {...base(p)}>
    <path d="m12 2.8 8 4.6v9.2l-8 4.6-8-4.6V7.4l8-4.6Z" />
    <path d="M4 7.4 12 12l8-4.6M12 12v9.2" />
  </svg>
)
export const Drop = (p: P) => (
  <svg {...base(p)}>
    <path d="M12 3.5s6 6.4 6 10.8a6 6 0 0 1-12 0C6 9.9 12 3.5 12 3.5Z" />
  </svg>
)
export const Pulse = (p: P) => (
  <svg {...base(p)}>
    <path d="M2.5 12h4l2.5-6 4 12 2.5-6h6" />
  </svg>
)
export const Plug = (p: P) => (
  <svg {...base(p)}>
    <path d="M9 3.5v4M15 3.5v4M6.5 7.5h11v3a5.5 5.5 0 0 1-11 0v-3ZM12 16v4.5" />
  </svg>
)
export const Box = (p: P) => (
  <svg {...base(p)}>
    <path d="M3.5 8 12 3.5 20.5 8v8L12 20.5 3.5 16V8Z" />
    <path d="m3.5 8 8.5 4.5L20.5 8M12 12.5v8" />
  </svg>
)
export const Book = (p: P) => (
  <svg {...base(p)}>
    <path d="M4.5 5.5A2 2 0 0 1 6.5 3.5h13v15h-13a2 2 0 0 0-2 2v-15Z" />
    <path d="M4.5 20.5a2 2 0 0 1 2-2h13v2h-13" />
  </svg>
)
export const Bridge = (p: P) => (
  <svg {...base(p)}>
    <path d="M2.5 16.5h19M5 16.5V10M19 16.5V10M2.5 10c4 0 5.5-3.5 9.5-3.5s5.5 3.5 9.5 3.5" />
  </svg>
)
export const Key = (p: P) => (
  <svg {...base(p)}>
    <circle cx="8" cy="15" r="4.5" />
    <path d="m11.2 11.8 8.3-8.3M16.5 6.5l2.5 2.5M14 9l2 2" />
  </svg>
)
export const Download = (p: P) => (
  <svg {...base(p)}>
    <path d="M12 4v11M7 10.5l5 5 5-5M5 19.5h14" />
  </svg>
)
export const GitHub = (p: P) => (
  <svg viewBox="0 0 16 16" aria-hidden {...p}>
    <path
      fill="currentColor"
      d="M8 0a8 8 0 0 0-2.53 15.59c.4.07.55-.17.55-.38v-1.33c-2.23.48-2.7-1.07-2.7-1.07-.36-.92-.89-1.17-.89-1.17-.73-.5.06-.49.06-.49.8.06 1.23.83 1.23.83.71 1.22 1.87.87 2.33.66.07-.52.28-.87.5-1.07-1.78-.2-3.65-.89-3.65-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.6 7.6 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.28.82 2.15 0 3.07-1.87 3.75-3.66 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0 0 8 0Z"
    />
  </svg>
)

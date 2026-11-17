// Small stroke icons (16px grid, currentColor). Decorative: always aria-hidden.
import type { ReactNode } from 'react'

function I({ children, size = 16 }: { children: ReactNode; size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {children}
    </svg>
  )
}

export const IconSend = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 13V3M4 7l4-4 4 4" />
  </I>
)
export const IconReceive = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 3v10M4 9l4 4 4-4" />
  </I>
)
export const IconDrop = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 2.5S4 7 4 9.6A4 4 0 0 0 12 9.6C12 7 8 2.5 8 2.5Z" />
  </I>
)
export const IconShield = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 1.8 2.8 3.8v4c0 3.1 2.2 5.4 5.2 6.4 3-1 5.2-3.3 5.2-6.4v-4Z" />
    <path d="m5.8 8 1.6 1.6 2.8-3" />
  </I>
)
export const IconClock = (p: { size?: number }) => (
  <I {...p}>
    <circle cx="8" cy="8" r="5.8" />
    <path d="M8 4.8V8l2.2 1.4" />
  </I>
)
export const IconUsers = (p: { size?: number }) => (
  <I {...p}>
    <circle cx="6" cy="5.5" r="2.3" />
    <path d="M1.8 13.2c.4-2.3 2.1-3.6 4.2-3.6s3.8 1.3 4.2 3.6" />
    <path d="M10.6 3.4a2.2 2.2 0 0 1 0 4.2M12 9.9c1.2.5 2 1.6 2.2 3.3" />
  </I>
)
export const IconKey = (p: { size?: number }) => (
  <I {...p}>
    <circle cx="5.2" cy="10.8" r="2.7" />
    <path d="m7.2 8.8 6-6M11.2 4.8l1.6 1.6M9.8 6.2l1.2 1.2" />
  </I>
)
export const IconCheck = (p: { size?: number }) => (
  <I {...p}>
    <path d="M3 8.5 6.5 12 13 4.5" />
  </I>
)
export const IconX = (p: { size?: number }) => (
  <I {...p}>
    <path d="m4 4 8 8M12 4l-8 8" />
  </I>
)
export const IconAlert = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 2 1.6 13.2h12.8Z" />
    <path d="M8 6.4v3.2M8 11.4v.2" />
  </I>
)
export const IconInfo = (p: { size?: number }) => (
  <I {...p}>
    <circle cx="8" cy="8" r="5.8" />
    <path d="M8 7.2v3.6M8 5.2v.2" />
  </I>
)
export const IconSnow = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 1.8v12.4M2.6 4.9l10.8 6.2M13.4 4.9 2.6 11.1" />
  </I>
)
export const IconTag = (p: { size?: number }) => (
  <I {...p}>
    <path d="M2.2 8.6V2.8a.6.6 0 0 1 .6-.6h5.8l5.2 5.2-6.4 6.4Z" />
    <circle cx="5.4" cy="5.4" r="1" />
  </I>
)
export const IconLock = (p: { size?: number }) => (
  <I {...p}>
    <rect x="3" y="7" width="10" height="7" rx="1.6" />
    <path d="M5.2 7V5.2a2.8 2.8 0 0 1 5.6 0V7" />
  </I>
)
export const IconGear = (p: { size?: number }) => (
  <I {...p}>
    <circle cx="8" cy="8" r="2.2" />
    <path d="M8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1" />
  </I>
)
export const IconChevron = (p: { size?: number }) => (
  <I {...p}>
    <path d="m6 3.5 4.5 4.5L6 12.5" />
  </I>
)
export const IconDown = (p: { size?: number }) => (
  <I {...p}>
    <path d="m3.5 6 4.5 4.5L12.5 6" />
  </I>
)
export const IconSpark = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 1.8v3M8 11.2v3M1.8 8h3M11.2 8h3M3.6 3.6l2 2M10.4 10.4l2 2M3.6 12.4l2-2M10.4 5.6l2-2" />
  </I>
)
export const IconPlus = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 3v10M3 8h10" />
  </I>
)
export const IconLink = (p: { size?: number }) => (
  <I {...p}>
    <path d="M6.6 9.4 9.4 6.6M7.2 4.4l1.2-1.2a2.6 2.6 0 0 1 3.7 3.7l-1.2 1.2M8.8 11.6l-1.2 1.2a2.6 2.6 0 0 1-3.7-3.7l1.2-1.2" />
  </I>
)
export const IconWallet = (p: { size?: number }) => (
  <I {...p}>
    <rect x="2" y="3.8" width="12" height="9.4" rx="1.8" />
    <path d="M2 6.6h12M10.6 9.6h1.2" />
  </I>
)
export const IconDownload = (p: { size?: number }) => (
  <I {...p}>
    <path d="M8 2.5v8M4.8 7.6 8 10.8l3.2-3.2M3 13.5h10" />
  </I>
)

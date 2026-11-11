import type { ReactNode } from 'react'

const LABEL: Record<string, string> = { tip: 'Tip', info: 'Note', warning: 'Warning', danger: 'Danger' }

function Icon({ type }: { type: string }) {
  const common = {
    viewBox: '0 0 16 16',
    width: 16,
    height: 16,
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.4,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    'aria-hidden': true,
  }
  switch (type) {
    case 'tip':
      return (
        <svg {...common}>
          <path d="M8 1.75v1.5M3.6 3.6l1.05 1.05M1.75 8h1.5M12.4 3.6l-1.05 1.05M14.25 8h-1.5" />
          <path d="M5.5 10.6A3.5 3.5 0 1 1 10.5 10.6c-.55.45-.8.95-.8 1.65v.25H6.3v-.25c0-.7-.25-1.2-.8-1.65Z" />
          <path d="M6.5 14.25h3" />
        </svg>
      )
    case 'warning':
      return (
        <svg {...common}>
          <path d="M7.13 2.5a1 1 0 0 1 1.74 0l5.4 9.5a1 1 0 0 1-.87 1.5H2.6a1 1 0 0 1-.87-1.5l5.4-9.5Z" />
          <path d="M8 6.25v3M8 11.4v.1" />
        </svg>
      )
    case 'danger':
      return (
        <svg {...common}>
          <path d="M5.6 1.75h4.8l3.85 3.85v4.8l-3.85 3.85H5.6L1.75 10.4V5.6L5.6 1.75Z" />
          <path d="M8 4.9v3.6M8 10.9v.1" />
        </svg>
      )
    default:
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6.25" />
          <path d="M8 7.25v4M8 4.9v.1" />
        </svg>
      )
  }
}

export function Callout({ type, title, children }: { type: string; title?: string; children?: ReactNode }) {
  const label = LABEL[type] ?? 'Note'
  return (
    <aside className="d-callout" data-type={type} role="note" aria-label={title ? `${label}: ${title}` : label}>
      <div className="d-callout-head">
        <span className="d-callout-icon">
          <Icon type={type} />
        </span>
        <span className="d-callout-kind">{label}</span>
        {title ? <span className="d-callout-title">{title}</span> : null}
      </div>
      <div className="d-callout-body">{children}</div>
    </aside>
  )
}

export function Details({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <details className="d-details">
      <summary>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
        {title}
      </summary>
      <div className="d-details-body">{children}</div>
    </details>
  )
}

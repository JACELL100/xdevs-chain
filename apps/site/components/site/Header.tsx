import type { ReactNode } from 'react'
import Link from 'next/link'
import HeaderNav from './HeaderNav'
import { GITHUB_URL } from './nav'
import './site.css'

export type HeaderProps = {
  /** Optional search trigger (rendered between the brand and the nav). */
  search?: ReactNode
  /**
   * Optional element shown only below 960px at the far left of the bar,
   * e.g. the docs sidebar drawer toggle.
   */
  mobileStart?: ReactNode
  /** 'landing' = translucent over the hero; 'docs' = solid bar, full width. */
  variant?: 'landing' | 'docs'
}

export function GitHubIcon() {
  return (
    <svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true">
      <path
        fill="currentColor"
        d="M8 0a8 8 0 0 0-2.53 15.59c.4.07.55-.17.55-.38v-1.33c-2.23.48-2.7-1.07-2.7-1.07-.36-.92-.89-1.17-.89-1.17-.73-.5.06-.49.06-.49.8.06 1.23.83 1.23.83.71 1.22 1.87.87 2.33.66.07-.52.28-.87.5-1.07-1.78-.2-3.65-.89-3.65-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.6 7.6 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.28.82 2.15 0 3.07-1.87 3.75-3.66 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0 0 8 0Z"
      />
    </svg>
  )
}

export default function Header({ search, mobileStart, variant = 'docs' }: HeaderProps) {
  return (
    <header className="xh-header" data-variant={variant}>
      <a className="xh-skip" href="#main">
        Skip to content
      </a>
      <div className="xh-header-inner">
        {mobileStart ? <div className="xh-header-mstart">{mobileStart}</div> : null}
        <Link href="/" className="xh-brand" aria-label="xDEVS Chain home">
          <img src="/brand/xdevs-chain-mark.svg" width={26} height={26} alt="" />
          <span className="xh-brand-word">
            xDEVS <span>Chain</span>
          </span>
        </Link>
        {search ? <div className="xh-header-search">{search}</div> : null}
        <HeaderNav />
        <a className="xh-header-gh" href={GITHUB_URL} target="_blank" rel="noopener" aria-label="xDEVS Chain on GitHub">
          <GitHubIcon />
        </a>
      </div>
    </header>
  )
}

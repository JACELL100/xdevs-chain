import type { ReactNode } from 'react'
import { ExternalIcon } from './Button'
import { Mark } from './Logo'
import { MobileMenu } from './MobileMenu'
import { NetworkBadge, type NetworkBadgeProps } from './NetworkBadge'
import { XDEVS_LINKS, type XdevsAppId } from './network'
import type { LinkComponent, NavItem } from './types'

export type AppShellProps = {
  /** App name shown next to the mark, e.g. "Explorer". */
  appName: string
  /** Which xDEVS app this is (marks its footer cross-link as current). */
  app?: XdevsAppId
  /** Header navigation. */
  nav?: NavItem[]
  /** Current pathname, for the active nav state (e.g. from usePathname() in a client wrapper). */
  currentPath?: string
  /** Link used for internal nav and the home link, e.g. next/link's Link. Defaults to <a>. */
  linkComponent?: LinkComponent
  /** Where the brand links to (default "/"). */
  homeHref?: string
  /** NetworkBadge props; default probes the Enclave RPC. Pass `false` to hide the badge. */
  network?: NetworkBadgeProps | false
  /** Extra header controls on the right (search, connect button…). */
  headerActions?: ReactNode
  /** Optional strip above the header (announcements, "testnet reset on …"). */
  banner?: ReactNode
  /** Footer extras shown above the legal line. */
  footerNote?: ReactNode
  children: ReactNode
}

const YEAR = 2026

const CROSS_LINKS: { id: XdevsAppId | 'site' | 'docs' | 'github'; label: string; href: string }[] = [
  { id: 'site', label: 'xdevschain.tech', href: XDEVS_LINKS.site },
  { id: 'explorer', label: 'Explorer', href: XDEVS_LINKS.explorer },
  { id: 'wallet', label: 'Wallet', href: XDEVS_LINKS.wallet },
  { id: 'faucet', label: 'Faucet', href: XDEVS_LINKS.faucet },
  { id: 'status', label: 'Status', href: XDEVS_LINKS.status },
  { id: 'docs', label: 'Docs', href: XDEVS_LINKS.docs },
  { id: 'github', label: 'GitHub', href: XDEVS_LINKS.github },
]

/** True when `path` is inside `item` (exact match for "/"). */
export function isNavActive(path: string | undefined, item: NavItem): boolean {
  if (!path || item.external) return false
  const m = item.match ?? item.href
  if (m === '/') return path === '/'
  return path === m || path.startsWith(m.endsWith('/') ? m : `${m}/`)
}

function NavLinks({ nav, currentPath, L }: { nav: NavItem[]; currentPath?: string; L?: LinkComponent }) {
  return nav.map((item) => {
    const active = isNavActive(currentPath, item)
    const props = { className: active ? 'is-active' : undefined, 'aria-current': active ? ('page' as const) : undefined }
    if (item.external)
      return (
        <a key={item.href} href={item.href} target="_blank" rel="noopener">
          {item.label}
          <ExternalIcon />
        </a>
      )
    return L ? (
      <L key={item.href} href={item.href} {...props}>
        {item.label}
      </L>
    ) : (
      <a key={item.href} href={item.href} {...props}>
        {item.label}
      </a>
    )
  })
}

/**
 * Page frame shared by every xDEVS web app: sticky header (mark + app name, nav, Enclave badge,
 * actions, mobile menu), <main id="main">, and the footer with cross-links and "Built by THE xDEVS".
 * Server-component safe; the burger menu and the network badge hydrate as small client islands.
 */
export function AppShell({
  appName,
  app,
  nav = [],
  currentPath,
  linkComponent: L,
  homeHref = '/',
  network = {},
  headerActions,
  banner,
  footerNote,
  children,
}: AppShellProps) {
  const brand = (
    <>
      <Mark size={26} title="" />
      <span className="xu-brand-word">
        xDEVS <span>Chain</span>
      </span>
      <span className="xu-brand-sep" aria-hidden="true" />
      <span className="xu-brand-app">{appName}</span>
    </>
  )
  return (
    <div className="xu-shell">
      <a className="xh-skip" href="#main">
        Skip to content
      </a>
      {banner ? (
        <div className="xu-banner">
          <div>{banner}</div>
        </div>
      ) : null}
      <header className="xu-header">
        <div className="xu-header-inner">
          {L ? (
            <L href={homeHref} className="xu-brand" aria-label={`xDEVS Chain ${appName} home`}>
              {brand}
            </L>
          ) : (
            <a href={homeHref} className="xu-brand" aria-label={`xDEVS Chain ${appName} home`}>
              {brand}
            </a>
          )}
          {nav.length ? (
            <nav className="xu-nav" aria-label="Main">
              <ul>
                {NavLinks({ nav, currentPath, L }).map((link) => (
                  <li key={link.key}>{link}</li>
                ))}
              </ul>
            </nav>
          ) : null}
          <div className="xu-header-end">
            {network !== false ? <NetworkBadge {...network} /> : null}
            {headerActions}
            {nav.length || network !== false ? (
              <MobileMenu>
                <nav aria-label="Main">
                  <NavLinks nav={nav} currentPath={currentPath} L={L} />
                  <a href={XDEVS_LINKS.site}>xdevschain.tech</a>
                </nav>
                {network !== false ? (
                  <div className="xu-sheet-net">
                    <NetworkBadge {...network} />
                  </div>
                ) : null}
              </MobileMenu>
            ) : null}
          </div>
        </div>
      </header>

      <main id="main" className="xu-main">
        {children}
      </main>

      <footer className="xu-foot">
        <div className="xh-wrap">
          <div className="xu-foot-top">
            <a href={XDEVS_LINKS.site} aria-label="xDEVS Chain" className="xu-logo">
              <Mark size={22} variant="solid" title="" />
              <span className="xu-brand-word">
                xDEVS <span>Chain</span>
              </span>
            </a>
            <nav aria-label="xDEVS Chain apps">
              <ul className="xu-foot-links">
                {CROSS_LINKS.map((l) => (
                  <li key={l.id}>
                    <a href={l.href} aria-current={l.id === app ? 'page' : undefined}>
                      {l.label}
                    </a>
                  </li>
                ))}
              </ul>
            </nav>
          </div>
          {footerNote ? <div style={{ marginTop: 20 }}>{footerNote}</div> : null}
          <div className="xu-foot-bottom">
            <p className="xu-foot-by">
              Built by{' '}
              <a href={XDEVS_LINKS.xdevs} target="_blank" rel="noopener">
                THE xDEVS
              </a>
            </p>
            <p className="xu-foot-legal">
              Enclave is a public testnet. EnclaveXDS (tXDS) has no monetary value. Apache-2.0 · © {YEAR} THE xDEVS
            </p>
          </div>
        </div>
      </footer>
    </div>
  )
}

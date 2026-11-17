'use client'

// Client wrapper so the shell knows the current path (active nav) and uses next/link for client-side navigation.
import { AppShell, type NavItem, XDEVS_LINKS } from '@xdevs/ui'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { config } from '@/lib/config'

const NAV: NavItem[] = [
  { label: 'Overview', href: '/' },
  { label: 'RPC endpoint', href: '/rpc' },
  { label: 'Docs', href: XDEVS_LINKS.endpoints, external: true },
]

export default function Shell({ children }: { children: React.ReactNode }) {
  const path = usePathname() ?? '/'
  return (
    <AppShell
      appName="Status"
      app="status"
      nav={NAV}
      currentPath={path}
      linkComponent={Link}
      network={config.demo ? { status: 'live', href: null } : { rpcUrl: config.rpcUrl, href: null }}
    >
      {children}
    </AppShell>
  )
}

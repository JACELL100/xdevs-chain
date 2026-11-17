'use client'

// Client wrapper so the shell knows the current path (active nav) and uses next/link for client-side navigation.
import { AppShell, type NavItem, XDEVS_LINKS } from '@xdevs/ui'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { config } from '@/lib/config'

const NAV: NavItem[] = [
  { label: 'Faucet', href: '/' },
  { label: 'Network status', href: config.statusUrl, external: true },
  { label: 'Docs', href: XDEVS_LINKS.endpoints, external: true },
]

export default function Shell({ children }: { children: React.ReactNode }) {
  const path = usePathname() ?? '/'
  return (
    <AppShell appName="Faucet" app="faucet" nav={NAV} currentPath={path} linkComponent={Link}>
      {children}
    </AppShell>
  )
}

'use client'

// Client frame: wallet state provider + the shared xDEVS AppShell (nav, network badge, account menu, alerts).
import { AppShell, type NavItem, XDEVS_LINKS } from '@xdevs/ui'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { AccountMenu } from '../components/AccountMenu.tsx'
import { Countdown } from '../components/ui.tsx'
import { config } from '../config.ts'
import { WalletProvider, useWallet } from '../wallet/WalletProvider.tsx'

const NAV: NavItem[] = [
  { label: 'Overview', href: '/' },
  { label: 'Send', href: '/send' },
  { label: 'Receive', href: '/receive' },
  { label: 'Vault', href: '/vault' },
  { label: 'Guardians', href: '/guardians' },
  { label: 'Approvals', href: '/approvals' },
]

function Banner({ path }: { path: string }) {
  const { demo, setDemo, queue } = useWallet()
  const open = queue.filter((a) => a.status === 'queued' || a.status === 'ready')
  const next = open.reduce<number | null>((m, a) => (m === null || Number(a.executeAfter) < m ? Number(a.executeAfter) : m), null)
  if (!demo && !open.length) return null
  return (
    <div className="w-banners">
      {demo ? (
        <p className="w-banner-demo">
          <span className="w-demo-tag">Demo</span>
          Sample account and data. Nothing is signed or sent, and no passkey is used.
          {!config.demo ? (
            <button type="button" className="w-banner-link" onClick={() => setDemo(false)}>
              Exit demo
            </button>
          ) : null}
        </p>
      ) : null}
      {open.length && path !== '/vault' ? (
        <p className="w-banner-vault">
          <span className="w-banner-dot" aria-hidden="true" />
          {open.length === 1 ? '1 action is' : `${open.length} actions are`} waiting in your Guardian Vault
          {next ? (
            <>
              {' '}
              (next executes in <Countdown until={next} />)
            </>
          ) : null}
          . Not you?
          <Link href="/vault" className="w-banner-link">
            Review and cancel
          </Link>
        </p>
      ) : null}
    </div>
  )
}

function Frame({ children }: { children: React.ReactNode }) {
  const path = usePathname() ?? '/'
  const { rec, chain, demo, ready, queue } = useWallet()
  const openCount = queue.filter((a) => a.status === 'queued' || a.status === 'ready').length
  const showBanner = ready && (demo || (openCount > 0 && path !== '/vault'))
  const status = demo ? 'live' : chain.kind === 'ok' ? 'live' : chain.kind === 'checking' ? 'checking' : chain.kind === 'wrong-chain' ? 'degraded' : 'down'
  return (
    <AppShell
      appName="Wallet"
      app="wallet"
      nav={ready && rec ? NAV : []}
      currentPath={path}
      linkComponent={Link}
      network={{ status, label: config.chainId === 717171 ? 'Enclave testnet' : config.chainName, href: XDEVS_LINKS.status }}
      headerActions={<AccountMenu />}
      banner={showBanner ? <Banner path={path} /> : null}
    >
      {children}
    </AppShell>
  )
}

export default function Shell({ children }: { children: React.ReactNode }) {
  return (
    <WalletProvider>
      <Frame>{children}</Frame>
    </WalletProvider>
  )
}

'use client'

// Client wrapper: the shared xDEVS AppShell with the explorer's nav, the search box, and a network badge driven by
// the explorer's own reachability state (no second RPC probe).

import { AppShell, type NavItem, type NetworkStatus } from '@xdevs/ui'
import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { useEffect } from 'react'
import { SearchBox } from '../src/components/SearchBox'
import { config } from '../src/lib/config'
import { startNetProbe, useNet } from '../src/lib/net'

const NAV: NavItem[] = [
  { label: 'Overview', href: '/' },
  { label: 'Blocks', href: '/blocks', match: '/block' },
  { label: 'Transactions', href: '/txs', match: '/tx' },
  { label: 'Validators', href: '/validators' },
]

export default function Shell({ children }: { children: React.ReactNode }) {
  const path = usePathname() ?? '/'
  const net = useNet()
  useEffect(() => startNetProbe(), [])
  // /blocks also lights up on /block/…, /txs on /tx/… (prefix match on the singular form).
  const status: NetworkStatus = config.demo ? 'live' : net.rpc === 'up' ? (net.api === 'down' ? 'degraded' : 'live') : net.rpc === 'down' ? 'down' : 'checking'
  return (
    <AppShell
      appName="Explorer"
      app="explorer"
      nav={NAV}
      currentPath={path}
      linkComponent={Link}
      network={{ status, label: config.demo ? 'Demo data' : 'Enclave testnet' }}
      headerActions={<SearchBox />}
      banner={
        config.demo ? (
          <span className="x-demo-banner">
            <b>Demo data.</b> Everything on this site is generated fixture data for previewing the explorer; it is not the Enclave testnet.
          </span>
        ) : undefined
      }
      footerNote={
        <p className="x-foot-note">
          {config.networkName} · chain id <span className="x-num">{config.chainId}</span>
          {!config.demo && net.head !== null ? (
            <>
              {' '}
              · head <span className="x-num">{net.head.toLocaleString('en-US')}</span>
            </>
          ) : null}
          {!config.demo && net.chainId !== null && net.chainId !== config.chainId ? (
            <span className="x-danger-text" role="alert">
              {' '}
              · the RPC reports chain {net.chainId}, expected {config.chainId}
            </span>
          ) : null}
          <span className="x-dim"> · Warnings and spam flags are advice, not facts.</span>
        </p>
      }
    >
      {children}
    </AppShell>
  )
}

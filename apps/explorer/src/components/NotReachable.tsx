'use client'

import { XDEVS_LINKS } from '@xdevs/ui'
import { useState } from 'react'
import { config } from '../lib/config'
import { reprobe, useNet } from '../lib/net'

/**
 * Full-page state when neither the node nor the indexer answers: the Enclave testnet is not public yet (or this
 * build points at a node that is down). Calm, informative, with what to do next. Never a blank page.
 */
export function NotReachable({ what }: { what?: string }) {
  const net = useNet()
  const [busy, setBusy] = useState(false)
  const host = (u: string) => {
    try {
      return new URL(u).host
    } catch {
      return u
    }
  }
  return (
    <section className="x-offline" aria-labelledby="offline-title">
      <div className="x-offline-art" aria-hidden="true">
        <svg viewBox="0 0 240 120" width="240" height="120">
          <defs>
            <linearGradient id="xo-line" x1="0" x2="1">
              <stop offset="0" stopColor="#4fe3cf" stopOpacity="0" />
              <stop offset=".5" stopColor="#4fe3cf" stopOpacity=".55" />
              <stop offset="1" stopColor="#4fe3cf" stopOpacity="0" />
            </linearGradient>
          </defs>
          <path d="M8 60h224" stroke="url(#xo-line)" strokeWidth="1" />
          {[0, 1, 2, 3, 4].map((i) => (
            <g key={i} className="x-offline-block" style={{ animationDelay: `${i * 0.35}s` }}>
              <rect x={24 + i * 42} y={44} width={32} height={32} rx={7} fill="#0b0d12" stroke="rgba(255,255,255,.16)" />
              <rect x={32 + i * 42} y={56} width={16} height={2} rx={1} fill="rgba(255,255,255,.22)" />
              <rect x={32 + i * 42} y={62} width={10} height={2} rx={1} fill="rgba(255,255,255,.12)" />
            </g>
          ))}
        </svg>
      </div>
      <p className="x-eyebrow">{config.networkName}</p>
      <h1 id="offline-title" className="x-offline-title">
        Network not reachable yet
      </h1>
      <p className="x-offline-lead">
        The Enclave testnet is launching soon. Once its RPC and explorer API are online, {what ?? 'this page'} will fill in by itself:
        live blocks every second, BFT finality certificates, transactions, validators and .xdevs names.
      </p>
      <ul className="x-offline-checks" aria-label="Connection checks">
        <li data-state={net.rpc}>
          <span className="x-offline-dot" aria-hidden="true" />
          <span>Node RPC</span>
          <code>{host(config.rpcUrl)}</code>
          <span className="x-offline-state">{net.rpc === 'up' ? 'reachable' : net.rpc === 'down' ? 'not reachable' : 'checking…'}</span>
        </li>
        <li data-state={net.api}>
          <span className="x-offline-dot" aria-hidden="true" />
          <span>Explorer API</span>
          <code>{host(config.apiUrl)}</code>
          <span className="x-offline-state">{net.api === 'up' ? 'reachable' : net.api === 'down' ? 'not reachable' : 'checking…'}</span>
        </li>
      </ul>
      <div className="x-offline-actions">
        <button
          type="button"
          className="xh-btn xh-btn-primary xu-btn is-md"
          disabled={busy}
          onClick={() => {
            setBusy(true)
            reprobe()
            setTimeout(() => setBusy(false), 1500)
          }}
        >
          {busy ? 'Checking…' : 'Check again'}
        </button>
        <a className="xh-btn xh-btn-ghost xu-btn is-md" href={XDEVS_LINKS.status}>
          Network status
        </a>
        <a className="xu-btn xu-btn-quiet xh-btn is-md" href={XDEVS_LINKS.site}>
          About xDEVS Chain
        </a>
      </div>
      <p className="x-offline-foot">The explorer re-checks on its own every minute or two. If the network is already live, check your own connection.</p>
    </section>
  )
}

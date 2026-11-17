'use client'

import { useEffect, useState } from 'react'
import { cx } from './format'
import { ENCLAVE, type NetworkStatus, XDEVS_LINKS, probeRpc } from './network'

export type NetworkBadgeProps = {
  /** Fixed status. When omitted and `probe` is on, the badge polls the RPC itself. */
  status?: NetworkStatus
  /** Poll `rpcUrl` with eth_chainId (default true when `status` is not given). */
  probe?: boolean
  rpcUrl?: string
  /** Poll interval in ms (default 60 s). */
  intervalMs?: number
  /** Text (default "Enclave testnet"). */
  label?: string
  /** Link target (default the status page); pass null for a plain badge. */
  href?: string | null
  className?: string
}

const DESCRIBE: Record<NetworkStatus, string> = {
  live: 'RPC reachable',
  degraded: 'RPC answering unexpectedly',
  down: 'RPC unreachable',
  checking: 'Checking the RPC…',
  unknown: 'Status unknown',
}

/** "Enclave testnet" pill with a live status dot (teal = live, amber = degraded, red = down). */
export function NetworkBadge({
  status,
  probe,
  rpcUrl = ENCLAVE.rpcUrl,
  intervalMs = 60_000,
  label = ENCLAVE.shortName,
  href = XDEVS_LINKS.status,
  className,
}: NetworkBadgeProps) {
  const polling = status === undefined && probe !== false
  // Deterministic first render (server and client agree), then probe in the browser.
  const [probed, setProbed] = useState<NetworkStatus>(polling ? 'checking' : 'unknown')

  useEffect(() => {
    if (!polling) return
    let alive = true
    const run = async () => {
      const s = await probeRpc(rpcUrl)
      if (alive) setProbed(s)
    }
    void run()
    const id = setInterval(() => {
      if (document.visibilityState === 'visible') void run()
    }, intervalMs)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [polling, rpcUrl, intervalMs])

  const s = status ?? probed
  const title = `${ENCLAVE.name} (chain ${ENCLAVE.chainId}): ${DESCRIBE[s]}`
  const inner = (
    <>
      <span className="xu-dot" aria-hidden="true" />
      {label}
      <span className="xh-sr">, {DESCRIBE[s]}</span>
    </>
  )
  const cls = cx('xu-badge xu-netbadge', className)
  return href ? (
    <a className={cls} data-status={s} title={title} href={href}>
      {inner}
    </a>
  ) : (
    <span className={cls} data-status={s} title={title}>
      {inner}
    </span>
  )
}

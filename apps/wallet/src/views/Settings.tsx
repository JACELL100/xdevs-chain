'use client'
// Settings: network endpoints (with live status), this device's accounts and passkey, export of PUBLIC account
// information (never any secret), and the demo switch.
import { AddressChip, Badge, Button, probeRpc } from '@xdevs/ui'
import { useEffect, useState } from 'react'
import { IconDownload } from '../components/icons.tsx'
import { DemoTag, KV, Notice, Page, SectionTitle } from '../components/ui.tsx'
import { config, explorerAddress, rpId } from '../config.ts'
import { shortAddr } from '../lib/format.ts'
import type { AccountRecord } from '../store.ts'
import { useWallet } from '../wallet/WalletProvider.tsx'

/** Public, shareable description of an account. Secrets (dev JWK / raw key) are never included. */
export function publicAccountInfo(rec: AccountRecord, extra: { primaryName: string | null; chainId: number; chainName: string }) {
  const replacer = (_k: string, v: unknown) => (typeof v === 'bigint' ? v.toString() : v)
  const info = {
    format: 'xdevs-wallet-public-account/1',
    exportedAt: new Date().toISOString(),
    chain: { id: extra.chainId, name: extra.chainName },
    address: rec.address,
    label: rec.label,
    primaryName: extra.primaryName,
    signer:
      rec.kind === 'passkey' && rec.passkey
        ? { kind: 'passkey', rpId: rec.passkey.rpId, credentialId: rec.passkey.credentialId, publicKey: rec.passkey.publicKey }
        : { kind: rec.kind === 'watch' ? 'none (watch-only)' : `${rec.kind} (dev only, secret not exported)` },
    counterfactual: rec.params && rec.salt ? { salt: rec.salt, initParams: rec.params } : null,
    note: 'Public information only. It cannot be used to sign anything. Keep it to restore the account view on another device or to help guardians.',
  }
  return JSON.parse(JSON.stringify(info, replacer)) as Record<string, unknown>
}

function useProbe(url: string, kind: 'rpc' | 'http') {
  const [s, setS] = useState<'checking' | 'live' | 'down' | 'degraded'>('checking')
  useEffect(() => {
    let alive = true
    const run = async () => {
      if (kind === 'rpc') {
        // probeRpc infers its chain id option as the Enclave literal; any configured chain id is valid here.
        const r = await probeRpc(url, { expectedChainId: config.chainId as 717171 })
        if (alive) setS(r === 'live' ? 'live' : r === 'degraded' ? 'degraded' : 'down')
        return
      }
      try {
        const ctrl = new AbortController()
        const t = setTimeout(() => ctrl.abort(), 6_000)
        const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_supportedEntryPoints', params: [] }), signal: ctrl.signal })
        clearTimeout(t)
        if (alive) setS(res.ok ? 'live' : 'degraded')
      } catch {
        if (alive) setS('down')
      }
    }
    void run()
    return () => {
      alive = false
    }
  }, [url, kind])
  return s
}

function StatusBadge({ s }: { s: string }) {
  return (
    <Badge tone={s === 'live' ? 'final' : s === 'checking' ? 'dim' : s === 'degraded' ? 'warn' : 'alert'} dot>
      {s === 'down' ? 'unreachable' : s}
    </Badge>
  )
}

export function Settings() {
  const w = useWallet()
  const rpc = useProbe(config.rpcUrl, 'rpc')
  const bundler = useProbe(config.bundlerUrl, 'http')
  const [host, setHost] = useState('')
  useEffect(() => setHost(location.hostname), [])
  const rec = w.rec

  function exportInfo() {
    if (!rec) return
    const blob = new Blob([JSON.stringify(publicAccountInfo(rec, { primaryName: w.primaryName, chainId: config.chainId, chainName: config.chainName }), null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `xdevs-account-${rec.address.slice(0, 10)}.json`
    a.click()
    setTimeout(() => URL.revokeObjectURL(url), 1_000)
  }

  return (
    <Page title={<>Settings {w.demo ? <DemoTag /> : null}</>} sub="Network, this device, and your account's public information." narrow>
      <section className="w-panel">
        <SectionTitle>Network</SectionTitle>
        <KV
          rows={[
            ['Network', `${config.chainName} (chain ${config.chainId})`],
            ['RPC', <span key="r" className="w-endpoint"><code>{config.rpcUrl}</code> <StatusBadge s={w.demo ? 'live' : rpc} /></span>],
            ['Bundler (ERC-4337)', <span key="b" className="w-endpoint"><code>{config.bundlerUrl}</code> <StatusBadge s={w.demo ? 'live' : bundler} /></span>],
            ['Explorer', <a key="e" className="xh-textlink" href={config.explorerUrl} target="_blank" rel="noopener noreferrer">{config.explorerUrl}</a>],
            ['Explorer API', config.apiUrl ? <code key="a">{config.apiUrl}</code> : 'off (reads from the chain)'],
            ['Faucet', <a key="f" className="xh-textlink" href={config.faucetUrl} target="_blank" rel="noopener noreferrer">{config.faucetUrl}</a>],
            ['Gas sponsor', config.paymasterSponsor ? <code key="p">{shortAddr(config.paymasterSponsor)}</code> : 'none configured'],
          ]}
        />
        <p className="w-fine">Endpoints are set at build time (NEXT_PUBLIC_* variables) and are public.</p>
      </section>

      <section className="w-panel">
        <SectionTitle>This device</SectionTitle>
        <KV
          rows={[
            ['Passkey domain (RP ID)', <code key="rp">{rpId()}</code>],
            ['Page host', <code key="h">{host}</code>],
            ['Accounts saved here', String(w.persisted.accounts.length)],
            ['Stored secrets', w.persisted.accounts.some((a) => a.kind === 'virtual' || a.kind === 'dev-key') ? <Badge key="s" tone="alert">dev keys present</Badge> : 'none (passkeys stay in your authenticator)'],
          ]}
        />
        {!w.demo && w.persisted.accounts.length ? (
          <ul className="w-plain-list">
            {w.persisted.accounts.map((a, i) => (
              <li key={a.address}>
                <span>
                  {a.label} {i === w.persisted.active ? <Badge tone="final">active</Badge> : null}
                </span>
                <AddressChip address={a.address} href={explorerAddress(a.address)} />
                {i !== w.persisted.active ? (
                  <Button size="sm" variant="quiet" onClick={() => w.selectAccount(i)}>
                    Switch
                  </Button>
                ) : null}
                <Button
                  size="sm"
                  variant="quiet"
                  onClick={() => {
                    if (window.confirm(`Forget ${a.label} on this device? The account itself is untouched; you can sign in again with its passkey once it is activated.`)) w.forget(a.address)
                  }}
                >
                  Forget
                </Button>
              </li>
            ))}
          </ul>
        ) : null}
      </section>

      {rec ? (
        <section className="w-panel">
          <SectionTitle>Export public account information</SectionTitle>
          <p className="w-fine">
            A JSON file with your address, passkey public key, credential id and (before activation) the account&apos;s creation parameters. It contains no
            secret and cannot sign anything. Useful for restoring the view on another device or for your guardians.
          </p>
          {rec.kind === 'virtual' || rec.kind === 'dev-key' ? <Notice tone="warn">Dev-only keys are never exported.</Notice> : null}
          <Button size="sm" variant="ghost" icon={<IconDownload />} onClick={exportInfo}>
            Download JSON
          </Button>
        </section>
      ) : null}

      <section className="w-panel">
        <SectionTitle>Demo data</SectionTitle>
        <p className="w-fine">Preview every screen with a sample account. Nothing is signed or sent in the demo.</p>
        {config.demo ? (
          <Badge tone="warn">this is a demo build</Badge>
        ) : (
          <Button size="sm" variant="ghost" onClick={() => w.setDemo(!w.demo)}>
            {w.demo ? 'Exit demo' : 'Show demo data'}
          </Button>
        )}
      </section>

      <p className="w-fine">
        Privacy: this wallet has no analytics and no tracking. It talks only to the endpoints above. Account records live in this browser&apos;s storage.
      </p>
    </Page>
  )
}

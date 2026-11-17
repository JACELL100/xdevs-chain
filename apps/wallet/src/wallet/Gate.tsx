'use client'
// Page gate: boot skeleton, the "Enclave launching soon" state when the network is unreachable, and the
// no-account state for pages that need one.
import { Button, Mark, Skeleton } from '@xdevs/ui'
import Link from 'next/link'
import type { ReactNode } from 'react'
import { PasskeyVisual } from '../components/PasskeyVisual.tsx'
import { config } from '../config.ts'
import { useWallet } from './WalletProvider.tsx'

export function Gate({ children, needsAccount = true, offlineOk = false }: { children: ReactNode; needsAccount?: boolean; offlineOk?: boolean }) {
  const w = useWallet()
  if (!w.ready || (!w.demo && w.chain.kind === 'checking' && !offlineOk)) return <BootSkeleton />
  if (!w.demo && !offlineOk && (w.chain.kind === 'unreachable' || w.chain.kind === 'wrong-chain')) return <LaunchingSoon />
  if (needsAccount && !w.rec) return <NoAccount />
  return <>{children}</>
}

export function BootSkeleton() {
  return (
    <div className="xu-page w-page" aria-busy="true" aria-label="Loading">
      <div className="w-boot">
        <Skeleton width={180} height={14} />
        <Skeleton width="min(420px, 80%)" height={44} />
        <Skeleton width="100%" height={180} radius={14} />
      </div>
    </div>
  )
}

function host(u: string): string {
  try {
    return new URL(u).host
  } catch {
    return u
  }
}

/** Polished pre-launch / outage state: the public network does not answer (yet). */
export function LaunchingSoon() {
  const { chain, recheckChain, setDemo } = useWallet()
  const wrong = chain.kind === 'wrong-chain'
  const enclave = config.chainId === 717171
  return (
    <div className="xu-page w-page">
      <section className="w-soon">
        <div className="w-soon-glow" aria-hidden="true" />
        <div className="w-soon-mark" aria-hidden="true">
          <span className="w-soon-orbit o1" />
          <span className="w-soon-orbit o2" />
          <Mark size={56} title="" />
        </div>
        <p className="w-eyebrow">{enclave ? 'xDEVS Chain Enclave · public testnet' : config.chainName}</p>
        <h1 className="w-soon-title">{wrong ? 'This network is not the one the wallet expects' : enclave ? 'Enclave testnet is launching soon' : 'The network is not reachable'}</h1>
        <p className="w-soon-lede">
          {wrong
            ? `The RPC at ${host(config.rpcUrl)} answers with chain id ${chain.chainId}, not ${config.chainId}. The wallet will not sign anything on the wrong chain.`
            : enclave
              ? 'The wallet is ready. The public network is not live yet: the moment it opens, you can create a passkey wallet here with no seed phrase.'
              : `The RPC at ${host(config.rpcUrl)} does not answer. Nothing can be signed or sent until it is back.`}
        </p>
        <div className="xh-ctas w-soon-ctas">
          <Button onClick={() => setDemo(true)}>Preview the wallet with demo data</Button>
          <Button variant="ghost" onClick={recheckChain}>
            Check again
          </Button>
        </div>
        <ul className="w-soon-status" aria-label="Network endpoints">
          <li>
            <span>RPC</span>
            <code>{host(config.rpcUrl)}</code>
            <em className={wrong ? 'is-warn' : 'is-off'}>{wrong ? 'wrong chain' : 'not live yet'}</em>
          </li>
          <li>
            <span>Bundler</span>
            <code>{host(config.bundlerUrl)}</code>
            <em className="is-off">waiting for the chain</em>
          </li>
          <li>
            <span>Faucet</span>
            <code>{host(config.faucetUrl)}</code>
            <em className="is-off">waiting for the chain</em>
          </li>
        </ul>
        <div className="w-soon-points">
          <div>
            <PasskeyVisual size={44} />
            <p>
              <b>No seed phrase.</b> Your Face ID, fingerprint or Windows Hello is the key.
            </p>
          </div>
          <div>
            <span className="w-soon-ico">24h</span>
            <p>
              <b>Guardian Vault.</b> Large transfers wait out a cancel window.
            </p>
          </div>
          <div>
            <span className="w-soon-ico">2/3</span>
            <p>
              <b>Guardians.</b> Friends or devices can freeze and recover your account.
            </p>
          </div>
        </div>
        <p className="w-soon-foot">
          Follow the launch on{' '}
          <a className="xh-textlink" href="https://status.xdevschain.tech">
            status.xdevschain.tech
          </a>{' '}
          or read how the wallet works in the{' '}
          <a className="xh-textlink" href={`${config.siteUrl}/concepts/smart-accounts`}>
            docs
          </a>
          .
        </p>
      </section>
    </div>
  )
}

function NoAccount() {
  return (
    <div className="xu-page w-page">
      <div className="w-empty-account">
        <PasskeyVisual size={88} />
        <h1 className="w-soon-title">No account on this device yet</h1>
        <p className="w-soon-lede">Create a wallet with a passkey, or sign in with a passkey you already have.</p>
        <div className="xh-ctas">
          <Button href="/welcome" linkComponent={Link}>
            Create or sign in
          </Button>
        </div>
      </div>
    </div>
  )
}

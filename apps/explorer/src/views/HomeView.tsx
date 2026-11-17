'use client'

import { Card, Skeleton } from '@xdevs/ui'
import { useMemo } from 'react'
import { Addr, Age, Native, Pill, useNow } from '../components/bits'
import { NotReachable } from '../components/NotReachable'
import { SearchBox } from '../components/SearchBox'
import { formatBlockTime, formatTps } from '../lib/chainstats'
import { config } from '../lib/config'
import { loadApiStats, loadLatestTxs, loadValidators } from '../lib/data'
import { formatNative, formatNumber, shortHash } from '../lib/format'
import { useLiveChain, type LiveMode } from '../lib/live'
import { networkDown, useNet } from '../lib/net'
import { paths } from '../lib/routes'
import type { BlockSummary, TxSummary } from '../lib/types'
import { useLoad } from '../lib/useLoad'
import { Page, useTitle } from './common'

const MODE_TEXT: Record<LiveMode, string> = {
  connecting: 'connecting',
  ws: 'live · websocket',
  poll: 'live · polling',
  demo: 'live · demo',
  offline: 'offline',
}

export function HomeView() {
  useTitle('Enclave testnet')
  const net = useNet()
  const live = useLiveChain(30)
  const now = useNow(1000)
  const online = config.demo || net.rpc === 'up'
  const validators = useLoad(online ? () => loadValidators() : null, `v:${online}`, 60_000)
  const apiUp = config.demo || net.api === 'up'
  const totals = useLoad(apiUp ? (s) => loadApiStats(s) : null, `s:${apiUp}`, 15_000)
  const indexed = !config.demo && net.api === 'up'
  const apiTxs = useLoad(indexed ? (s) => loadLatestTxs(10, s) : null, `t:${indexed}`, 4000)

  const names = useMemo(() => Object.fromEntries((validators.data?.validators ?? []).filter((v) => v.name).map((v) => [v.operator.toLowerCase(), v.name!])), [validators.data])
  const activeCount = validators.data?.validators.filter((v) => v.status === 'active').length ?? null

  if (live.mode === 'offline' && networkDown(net))
    return (
      <Page>
        <NotReachable what="the explorer" />
      </Page>
    )

  const head = live.blocks[0]
  const loading = !head
  const txs: TxSummary[] = (apiTxs.data ?? live.txs).slice(0, 10)
  const strip = [...live.blocks].reverse()
  const maxTx = Math.max(1, ...strip.map((b) => b.txCount))

  return (
    <Page wide>
      <section className="x-hero" aria-labelledby="hero-title">
        <div className="x-hero-main">
          <p className="x-eyebrow">
            <span className={`x-live-dot is-${live.mode}`} aria-hidden="true" />
            {config.networkName} · <span className="x-num">{MODE_TEXT[live.mode]}</span>
          </p>
          <h1 id="hero-title" className="x-hero-title">
            <span className="x-hero-label">Block</span>
            {head ? (
              <a href={paths.block(head.number)} className="x-hero-head x-num" key={head.number}>
                {head.number.toLocaleString('en-US')}
              </a>
            ) : (
              <Skeleton width={260} height={48} />
            )}
          </h1>
          <div className="x-hero-meta" aria-live="off">
            <Pill tone="ok" dot>
              final
            </Pill>
            <span className="x-muted">
              BFT finality: every block is final the moment it is committed, no confirmations to wait for.
            </span>
          </div>
        </div>
        <div className="x-hero-search">
          <SearchBox variant="hero" />
          <p className="x-hint">
            Try a block number, a transaction hash, an address or <span className="x-mono">name.xdevs</span>
          </p>
        </div>
      </section>

      <section className="x-stats" aria-label="Network statistics">
        <StatTile label="block height" value={live.stats.height !== null ? formatNumber(live.stats.height) : null} hint="latest committed block" />
        <StatTile label="avg block time" value={live.stats.avgBlockTime !== null ? formatBlockTime(live.stats.avgBlockTime) : null} hint={`over the last ${live.stats.sample} blocks`} />
        <StatTile label="throughput" value={live.stats.tps !== null ? `${formatTps(live.stats.tps)} tps` : null} hint={`measured, last ${live.stats.sample} blocks`} />
        <StatTile
          label="validators"
          value={activeCount !== null ? String(activeCount) : validators.error ? '—' : null}
          hint={validators.data ? `${validators.data.validators.length} registered` : validators.error ? 'Staking predeploy not readable' : 'active set'}
          href={paths.validators()}
        />
        <StatTile label="finality" value={<span className="x-final-value">instant <span aria-hidden="true">✓</span></span>} hint="2/3+ precommits per block" />
      </section>

      <section className="x-strip-card" aria-label="Transactions per block, last 30 blocks">
        <div className="x-strip-head">
          <span className="x-label">transactions per block</span>
          <span className="x-muted x-small">
            {totals.data ? (
              <>
                <span className="x-num">{formatNumber(totals.data.totalTransactions)}</span> txs ·{' '}
                <span className="x-num">{formatNumber(totals.data.totalAddresses)}</span> addresses ·{' '}
                <span className="x-num">{formatNative(totals.data.burnedFees, config.nativeSymbol, 2)}</span> burned
              </>
            ) : (
              'last 30 blocks'
            )}
          </span>
        </div>
        <div className="x-strip" role="list">
          {loading
            ? Array.from({ length: 30 }, (_, i) => <span key={i} className="x-strip-bar is-skel" style={{ height: `${20 + ((i * 41) % 60)}%` }} />)
            : strip.map((b) => (
                <a
                  role="listitem"
                  key={b.number}
                  href={paths.block(b.number)}
                  className="x-strip-bar"
                  style={{ height: `${Math.max(6, (b.txCount / maxTx) * 100)}%` }}
                  title={`Block ${b.number.toLocaleString('en-US')}: ${b.txCount} tx`}
                  aria-label={`Block ${b.number}: ${b.txCount} transactions`}
                />
              ))}
        </div>
      </section>

      <div className="x-grid-2">
        <Card title="Latest blocks" actions={<a className="x-card-link" href={paths.blocks()}>All blocks</a>} flush>
          <ul className="x-feed" aria-live="polite" aria-relevant="additions">
            {loading ? <FeedSkeleton /> : live.blocks.slice(0, 10).map((b) => <BlockRow key={b.number} b={b} now={now} name={names[b.miner.toLowerCase()]} />)}
          </ul>
        </Card>
        <Card title="Latest transactions" actions={<a className="x-card-link" href={paths.txs()}>All transactions</a>} flush>
          <ul className="x-feed">
            {loading && !apiTxs.data ? (
              <FeedSkeleton />
            ) : txs.length ? (
              txs.map((t) => <TxRow key={t.hash} t={t} now={now} />)
            ) : (
              <li className="x-feed-empty">No transactions in the last {live.blocks.length} blocks. The chain keeps producing empty blocks every second.</li>
            )}
          </ul>
        </Card>
      </div>
    </Page>
  )
}

function StatTile({ label, value, hint, href }: { label: string; value: React.ReactNode | null; hint?: string; href?: string }) {
  const body = (
    <>
      <span className="xu-stat-label">{label}</span>
      <span className="xu-stat-value">{value ?? <Skeleton width="60%" height={26} />}</span>
      {hint ? <span className="xu-stat-hint">{hint}</span> : null}
    </>
  )
  return href ? (
    <a className="xu-stat x-stat is-link" href={href}>
      {body}
    </a>
  ) : (
    <div className="xu-stat x-stat">{body}</div>
  )
}

function FeedSkeleton() {
  return (
    <>
      {Array.from({ length: 8 }, (_, i) => (
        <li key={i} className="x-feed-row">
          <Skeleton width={34} height={34} radius={9} />
          <span className="x-feed-main">
            <Skeleton width="45%" height={12} />
            <Skeleton width="70%" height={11} />
          </span>
          <Skeleton width={60} height={12} />
        </li>
      ))}
    </>
  )
}

function BlockRow({ b, now, name }: { b: BlockSummary; now: number; name?: string }) {
  return (
    <li className="x-feed-row x-enter">
      <span className="x-feed-icon" aria-hidden="true">
        Bk
      </span>
      <span className="x-feed-main">
        <span className="x-feed-line">
          <a className="x-link x-num x-strong" href={paths.block(b.number)}>
            {b.number.toLocaleString('en-US')}
          </a>
          <Age ts={b.timestamp} now={now} />
        </span>
        <span className="x-feed-line x-muted">
          <span className="x-small">proposed by</span> <Addr address={b.miner} name={name} copy={false} />
        </span>
      </span>
      <span className="x-feed-side">
        <span className="x-num">
          {b.txCount} <span className="x-muted">tx{b.txCount === 1 ? '' : 's'}</span>
        </span>
        <Pill tone="ok">final</Pill>
      </span>
    </li>
  )
}

function TxRow({ t, now }: { t: TxSummary; now: number }) {
  return (
    <li className="x-feed-row x-enter">
      <span className="x-feed-icon is-tx" aria-hidden="true">
        Tx
      </span>
      <span className="x-feed-main">
        <span className="x-feed-line">
          <a className="x-link x-mono" href={paths.tx(t.hash)} title={t.hash}>
            {shortHash(t.hash, 6)}
          </a>
          <Age ts={t.timestamp} now={now} />
        </span>
        <span className="x-feed-line x-muted x-small">
          <Addr address={t.from} copy={false} /> <span aria-hidden="true">→</span>
          <span className="xh-sr">to</span> {t.to ? <Addr address={t.to} copy={false} /> : <span>contract creation</span>}
        </span>
      </span>
      <span className="x-feed-side">
        {BigInt(t.value) > 0n ? <Native value={t.value} digits={3} /> : <span className="x-dim x-num">0 {config.nativeSymbol}</span>}
        {t.status === 'reverted' ? <Pill tone="danger">reverted</Pill> : null}
      </span>
    </li>
  )
}

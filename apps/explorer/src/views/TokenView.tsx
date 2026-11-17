'use client'

import { Card, CopyButton } from '@xdevs/ui'
import { KV, PageHead, Pill, SourceNote, useNow } from '../components/bits'
import { TransferTable } from '../components/tables'
import { loadToken } from '../lib/data'
import { formatDuration, formatUnits } from '../lib/format'
import { paths, useRouteParam } from '../lib/routes'
import { useLoad } from '../lib/useLoad'
import { DetailSkeleton, MissingParam, Page, Pending, Problem, useTitle } from './common'

export function TokenView() {
  const address = useRouteParam('token')
  const now = useNow(5000)
  const r = useLoad(address ? (s) => loadToken(address, s) : null, `tok:${address}`)
  useTitle(r.data?.token.symbol ? `${r.data.token.symbol} token` : address ? 'Token' : null)

  if (address === undefined) return <Page><DetailSkeleton /></Page>
  if (address === null) return <Page><MissingParam what="token" example="/token/0x…" /></Page>
  if (!r.data) return <Page>{r.error ? <Problem error={r.error} onRetry={r.reload} what="this token" /> : <Pending what="this token" />}</Page>

  const { token: t, transfers, scannedBlocks, source } = r.data
  const supply = t.totalSupply !== null ? formatUnits(t.totalSupply, t.decimals ?? 18, 2) : null
  return (
    <Page>
      <PageHead
        eyebrow={t.xdrc20 ? 'XDRC-20 token' : 'ERC-20 token'}
        title={
          <span>
            {t.name ?? 'Unnamed token'} {t.symbol ? <span className="x-muted x-title-sub">{t.symbol}</span> : null}
          </span>
        }
      >
        {t.known ? <Pill tone="ok" dot>verified</Pill> : <Pill tone="neutral">unverified</Pill>}
        {t.xdrc20 ? <Pill tone="ok">expiring approvals</Pill> : null}
        {t.label ? <Pill tone="info">{t.label}</Pill> : null}
      </PageHead>

      <section className="x-stats is-3" aria-label="Token facts">
        <div className="xu-stat x-stat">
          <span className="xu-stat-label">total supply</span>
          <span className="xu-stat-value">{supply ?? '—'}</span>
          <span className="xu-stat-hint">{t.symbol ?? 'units'}</span>
        </div>
        <div className="xu-stat x-stat">
          <span className="xu-stat-label">decimals</span>
          <span className="xu-stat-value">{t.decimals ?? '—'}</span>
          <span className="xu-stat-hint">{t.decimals === null ? 'decimals() failed: amounts shown raw' : 'from decimals()'}</span>
        </div>
        <div className="xu-stat x-stat">
          <span className="xu-stat-label">default approval</span>
          <span className="xu-stat-value">{t.xdrc20 ? formatDuration(t.xdrc20.defaultApprovalDuration) : 'never expires'}</span>
          <span className="xu-stat-hint">{t.xdrc20 ? (t.xdrc20.maxApprovalDuration ? `max ${formatDuration(t.xdrc20.maxApprovalDuration)}` : 'XDRC-20') : 'plain ERC-20 allowances'}</span>
        </div>
      </section>

      <Card title="Contract" className="x-section">
        <KV
          rows={[
            ['Address', <span className="x-hash-row"><a className="x-link x-mono x-break" href={paths.address(t.address)}>{t.address}</a><CopyButton value={t.address} title="Copy token address" /></span>],
            ['Standard', t.xdrc20 ? <span>XDRC-20 <span className="x-muted x-small">· ERC-20 with expiring approvals and revoke-all</span></span> : 'ERC-20'],
            !t.known ? ['Caution', <span className="x-muted">Not on the explorer's verified list. Anyone can deploy a token with any name and symbol: check the address, not the symbol.</span>] : null,
          ]}
        />
      </Card>

      <Card
        title="Recent transfers"
        flush
        className="x-section"
        actions={scannedBlocks !== null ? <span className="x-muted x-small">from the node, last {scannedBlocks.toLocaleString('en-US')} blocks</span> : null}
      >
        <TransferTable transfers={transfers} now={now} empty={<p className="x-empty-row">No transfers in the scanned range.</p>} />
      </Card>
      {r.data.holdersNote && source === 'rpc' ? <p className="x-small x-muted">{r.data.holdersNote}</p> : null}
      <SourceNote source={source} />
    </Page>
  )
}

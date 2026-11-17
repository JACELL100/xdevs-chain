'use client'

import { Card, CopyButton } from '@xdevs/ui'
import { useMemo } from 'react'
import { Addr, KV, Native, PageHead, Pill, SourceNote, useNow } from '../components/bits'
import { CommitCertificate } from '../components/CommitCert'
import { TxTable } from '../components/tables'
import { loadBlock, loadValidators } from '../lib/data'
import { formatDate, formatGwei, formatNumber, percent, timeAgo, toBig } from '../lib/format'
import { paths, useRouteParam } from '../lib/routes'
import { useLoad } from '../lib/useLoad'
import { DetailSkeleton, MissingParam, Page, Pending, Problem, useTitle } from './common'

export function BlockView() {
  const id = useRouteParam('block')
  const now = useNow(5000)
  const r = useLoad(id ? (s) => loadBlock(id, s) : null, `b:${id}`)
  const set = useLoad(r.data?.commit ? () => loadValidators() : null, `set:${!!r.data?.commit}`)
  useTitle(id ? `Block ${id}` : null)
  const names = useMemo(() => Object.fromEntries((set.data?.validators ?? []).filter((v) => v.name).map((v) => [v.operator.toLowerCase(), v.name!])), [set.data])

  if (id === undefined) return <Page><DetailSkeleton /></Page>
  if (id === null) return <Page><MissingParam what="block" example="/block/1024" /></Page>
  if (!r.data) return <Page>{r.error ? <Problem error={r.error} onRetry={r.reload} what="this block" /> : <Pending what="this block" />}</Page>
  const { block: b, commit, source } = r.data
  const used = toBig(b.gasUsed)
  const limit = toBig(b.gasLimit)
  return (
    <Page>
      <PageHead
        eyebrow="Block"
        title={<span className="x-num">#{formatNumber(b.number)}</span>}
        actions={
          <nav className="x-pager" aria-label="Neighbouring blocks">
            {b.number > 0 ? (
              <a className="xh-btn xh-btn-ghost xu-btn is-sm" href={paths.block(b.number - 1)} aria-label={`Previous block ${b.number - 1}`}>
                ← {formatNumber(b.number - 1)}
              </a>
            ) : null}
            <a className="xh-btn xh-btn-ghost xu-btn is-sm" href={paths.block(b.number + 1)} aria-label={`Next block ${b.number + 1}`}>
              {formatNumber(b.number + 1)} →
            </a>
          </nav>
        }
      >
        <Pill tone="ok" dot>
          final
        </Pill>
        <span className="x-muted">
          {formatDate(b.timestamp)}
          {now ? ` · ${timeAgo(b.timestamp, now)}` : ''}
        </span>
      </PageHead>

      <Card title="Commit certificate" actions={<span className="x-muted x-small">xdevs_getCommitCertificate</span>} className="x-section">
        <CommitCertificate commit={commit} set={set.data} signedCount={b.commit?.signatures} />
      </Card>

      <Card title="Overview" className="x-section">
        <KV
          rows={[
            ['Block hash', <span className="x-hash-row"><span className="x-mono x-break">{b.hash}</span><CopyButton value={b.hash} title="Copy block hash" /></span>],
            ['Parent hash', b.number > 0 ? <a className="x-link x-mono x-break" href={paths.block(b.number - 1)}>{b.parentHash}</a> : <span className="x-mono x-break">{b.parentHash}</span>],
            ['Proposer', <Addr address={b.miner} name={names[b.miner.toLowerCase()]} full />],
            ['Transactions', <span className="x-num">{b.txCount}</span>],
            [
              'Gas used',
              <span className="x-gas">
                <span className="x-num">{formatNumber(Number(used))}</span>
                <span className="x-muted"> of {formatNumber(Number(limit))} ({percent(used, limit)})</span>
                <span className="x-meter" aria-hidden="true"><span style={{ width: `${limit > 0n ? Number((used * 10000n) / limit) / 100 : 0}%` }} /></span>
              </span>,
            ],
            ['Base fee', <span className="x-num">{formatGwei(b.baseFeePerGas)}</span>],
            ['Fees burned', <Native value={b.fees.burned} digits={8} />],
            ['Fees to Treasury', <Native value={b.fees.treasury} digits={8} />],
            b.fees.priority !== null && ['Tips to proposer', <Native value={b.fees.priority} digits={8} />],
            ['Size', b.size ? <span className="x-num">{formatNumber(b.size)} bytes</span> : '—'],
          ]}
        />
      </Card>

      <Card title={`Transactions (${b.transactions.length})`} flush className="x-section">
        <TxTable txs={b.transactions} now={now} compact empty={<p className="x-empty-row">No transactions in this block.</p>} />
      </Card>
      <SourceNote source={source} />
    </Page>
  )
}

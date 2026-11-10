import { api } from '../lib/api.ts'
import { config } from '../lib/config.ts'
import { formatDate, formatGwei, formatNative, formatNumber, percent, timeAgo, toBig } from '../lib/format.ts'
import { useAsync, useNow } from '../lib/hooks.ts'
import { paths } from '../lib/router.ts'
import { TxTable } from '../components/tables.tsx'
import { AddressLink, Badge, Copy, Empty, ErrorBox, KV, Link, Loading, Section } from '../components/ui.tsx'

export function BlockPage({ id }: { id: string }) {
  const now = useNow(5000)
  const { data: b, error, reload } = useAsync((s) => api.block(id, s), [id])
  if (error) return <ErrorBox error={error} onRetry={reload} />
  if (!b) return <Loading what="Loading block" />
  const used = toBig(b.gasUsed)
  return (
    <>
      <div className="page-head">
        <h1>Block {formatNumber(b.number)}</h1>
        <nav className="pager" aria-label="Neighbouring blocks">
          {b.number > 0 ? <Link to={paths.block(b.number - 1)}>← {b.number - 1}</Link> : null}
          <Link to={paths.block(b.number + 1)}>{b.number + 1} →</Link>
        </nav>
      </div>
      <Section title="Overview" id="block-overview">
        <KV
          rows={[
            ['Finality', b.commit ? (
              <span>
                <Badge tone="ok">Finalized</Badge> by {b.commit.signatures} validator{b.commit.signatures === 1 ? '' : 's'} (round {b.commit.round})
              </span>
            ) : (
              <span>
                <Badge tone="neutral">No commit certificate</Badge> <span className="muted small">dev or single-node block, or the node does not serve xdevs_getCommitCertificate</span>
              </span>
            )],
            ['Timestamp', <span title={String(b.timestamp)}>{formatDate(b.timestamp)} ({timeAgo(b.timestamp, now)})</span>],
            ['Hash', <span className="mono break">{b.hash} <Copy text={b.hash} /></span>],
            ['Parent', b.number > 0 ? <Link to={paths.block(b.number - 1)} className="mono break">{b.parentHash}</Link> : <span className="mono break">{b.parentHash}</span>],
            ['Proposer (fee recipient)', <AddressLink address={b.miner} full />],
            ['Transactions', String(b.txCount)],
            ['Gas used', `${formatNumber(Number(used))} of ${formatNumber(Number(toBig(b.gasLimit)))} (${percent(used, toBig(b.gasLimit))})`],
            ['Base fee', formatGwei(b.baseFeePerGas)],
            ['Fees burned', formatNative(b.fees.burned, config.nativeSymbol, 8)],
            ['Fees to Treasury', formatNative(b.fees.treasury, config.nativeSymbol, 8)],
            ['Tips to proposer', formatNative(b.fees.priority, config.nativeSymbol, 8)],
            ['Size', b.size ? `${formatNumber(b.size)} bytes` : '—'],
          ]}
        />
        {b.commit ? (
          <details className="pad">
            <summary>Validator signatures ({b.commit.validators.length})</summary>
            <ul className="mono small">
              {b.commit.validators.map((v) => (
                <li key={v}>{v}</li>
              ))}
            </ul>
          </details>
        ) : null}
      </Section>
      <Section title={`Transactions (${b.transactions.length})`} id="block-txs">
        {b.transactions.length ? <TxTable txs={b.transactions} now={now} /> : <Empty>No transactions in this block.</Empty>}
      </Section>
    </>
  )
}

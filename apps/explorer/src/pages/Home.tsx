import { api } from '../lib/api.ts'
import { config } from '../lib/config.ts'
import { formatGwei, formatNative, formatNumber } from '../lib/format.ts'
import { useAsync, useNow, usePaged } from '../lib/hooks.ts'
import { paths } from '../lib/router.ts'
import { BlockTable, TxTable } from '../components/tables.tsx'
import { ErrorBox, Link, LoadMore, Loading, Section } from '../components/ui.tsx'

export function Home() {
  const now = useNow(5000)
  const stats = useAsync((s) => api.stats(s), [], 5000)
  const blocks = useAsync((s) => api.blocks(null, 10, s), [], 4000)
  const txs = useAsync((s) => api.txs(null, 10, s), [], 4000)
  const st = stats.data
  return (
    <>
      <h1 className="sr-only">xDEVS Chain explorer</h1>
      {stats.error && !st ? <ErrorBox error={stats.error} onRetry={stats.reload} /> : null}
      <section className="stats" aria-label="Network statistics">
        <Stat label="Latest block" value={st ? formatNumber(st.latestBlock ?? st.indexedBlock) : '…'} hint={st?.lagBlocks ? `indexer ${st.lagBlocks} behind` : undefined} />
        <Stat label="Avg block time" value={st?.avgBlockTimeMs ? `${(st.avgBlockTimeMs / 1000).toFixed(2)} s` : '…'} />
        <Stat label="Transactions" value={st ? formatNumber(st.totalTransactions) : '…'} />
        <Stat label="Addresses" value={st ? formatNumber(st.totalAddresses) : '…'} />
        <Stat label="Base fee" value={st ? formatGwei(st.latestBaseFee) : '…'} />
        <Stat label="Burned (all time)" value={st ? formatNative(st.burnedFees, config.nativeSymbol, 4) : '…'} />
        <Stat label="To Treasury" value={st ? formatNative(st.treasuryFees, config.nativeSymbol, 4) : '…'} />
      </section>
      <div className="grid-2">
        <Section title="Latest blocks" id="latest-blocks" actions={<Link to={paths.blocks()}>All blocks</Link>}>
          {blocks.data ? <BlockTable blocks={blocks.data.items} now={now} /> : blocks.error ? <ErrorBox error={blocks.error} /> : <Loading />}
        </Section>
        <Section title="Latest transactions" id="latest-txs" actions={<Link to={paths.txs()}>All transactions</Link>}>
          {txs.data ? <TxTable txs={txs.data.items} now={now} /> : txs.error ? <ErrorBox error={txs.error} /> : <Loading />}
        </Section>
      </div>
    </>
  )
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="stat">
      <div className="stat-label">{label}</div>
      <div className="stat-value">{value}</div>
      {hint ? <div className="stat-hint">{hint}</div> : null}
    </div>
  )
}

export function BlocksPage() {
  const now = useNow(5000)
  const p = usePaged((c, s) => api.blocks(c, 25, s), [])
  return (
    <Section title={<h1 className="h-inline">Blocks</h1>} id="blocks">
      {p.error ? <ErrorBox error={p.error} /> : null}
      {p.items.length ? <BlockTable blocks={p.items} now={now} /> : p.loading ? <Loading /> : null}
      <LoadMore hasMore={p.hasMore} loading={p.loading} onClick={p.loadMore} />
    </Section>
  )
}

export function TxsPage() {
  const now = useNow(5000)
  const p = usePaged((c, s) => api.txs(c, 25, s), [])
  return (
    <Section title={<h1 className="h-inline">Transactions</h1>} id="txs">
      {p.error ? <ErrorBox error={p.error} /> : null}
      {p.items.length ? <TxTable txs={p.items} now={now} /> : p.loading ? <Loading /> : null}
      <LoadMore hasMore={p.hasMore} loading={p.loading} onClick={p.loadMore} />
    </Section>
  )
}

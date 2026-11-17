'use client'
// /blocks, /txs and /search.

import { Card, EmptyState } from '@xdevs/ui'
import { useEffect, useState } from 'react'
import { PageHead, Pill, useNow } from '../components/bits'
import { BlockTable, TxTable } from '../components/tables'
import { NotReachable } from '../components/NotReachable'
import { searchTarget } from '../components/SearchBox'
import { api } from '../lib/api'
import { config } from '../lib/config'
import { loadValidators } from '../lib/data'
import * as demo from '../lib/fixtures'
import { classifySearch } from '../lib/format'
import { useLiveChain } from '../lib/live'
import { networkDown, useNet } from '../lib/net'
import { paths, useQueryParam } from '../lib/routes'
import type { BlockSummary, SearchResult, TxSummary } from '../lib/types'
import { useLoad } from '../lib/useLoad'
import { Page, useTitle } from './common'

const PAGE = 25

/** Paged list: indexer cursor pages when the API is up, otherwise the live RPC window. */
function usePagedList<T>(fetchPage: ((cursor: string | null) => Promise<{ items: T[]; nextCursor: string | null }>) | null, key: string) {
  const [items, setItems] = useState<T[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [next, setNext] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  useEffect(() => {
    setItems([])
    setCursor(null)
    setNext(null)
  }, [key])
  useEffect(() => {
    if (!fetchPage) return
    let live = true
    setLoading(true)
    fetchPage(cursor).then(
      (p) => {
        if (!live) return
        setItems((prev) => (cursor ? [...prev, ...p.items] : p.items))
        setNext(p.nextCursor)
        setLoading(false)
      },
      (e: Error) => {
        if (!live) return
        setError(e)
        setLoading(false)
      },
    )
    return () => {
      live = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, cursor, !!fetchPage])
  return { items, loading, error, hasMore: !!next, more: () => next && setCursor(next) }
}

function demoBlocksPage(cursor: string | null) {
  const from = cursor ? Number(cursor) : demo.demoHead()
  const items = Array.from({ length: PAGE }, (_, i) => demo.demoBlockSummary(from - i)).filter((b) => b.number >= 0)
  return Promise.resolve({ items, nextCursor: String(from - PAGE) })
}

export function BlocksView() {
  useTitle('Blocks')
  const net = useNet()
  const now = useNow(1000)
  const indexed = !config.demo && net.api === 'up'
  const live = useLiveChain(PAGE)
  const paged = usePagedList<BlockSummary>(config.demo ? demoBlocksPage : indexed ? (c) => api.blocks(c, PAGE) : null, `blocks:${indexed}`)
  const set = useLoad(config.demo || net.rpc === 'up' ? () => loadValidators() : null, `vs:${net.rpc}`)
  const names = Object.fromEntries((set.data?.validators ?? []).filter((v) => v.name).map((v) => [v.operator.toLowerCase(), v.name!]))
  if (networkDown(net)) return <Page><NotReachable what="the block list" /></Page>
  // Live window on top (new blocks every second), older pages from the indexer / demo below.
  const first = live.blocks
  const rest = paged.items.filter((b) => !first.length || b.number < first[first.length - 1]!.number)
  const rows = config.demo || indexed ? [...first, ...rest] : first
  return (
    <Page>
      <PageHead eyebrow={config.networkName} title="Blocks">
        <Pill tone="ok" dot>
          live
        </Pill>
        <span className="x-muted">Every block is final at commit. New blocks appear at the top as they are produced.</span>
      </PageHead>
      <Card flush className="x-section">
        <BlockTable blocks={rows} now={now} loading={!rows.length} names={names} />
      </Card>
      {config.demo || indexed ? (
        <div className="x-more">
          <button type="button" className="xh-btn xh-btn-ghost xu-btn is-sm" onClick={paged.more} disabled={paged.loading || !paged.hasMore}>
            {paged.loading ? 'Loading…' : 'Older blocks'}
          </button>
        </div>
      ) : (
        <p className="x-small x-muted">Showing the latest {first.length} blocks from the node. Older pages need the explorer API.</p>
      )}
    </Page>
  )
}

export function TxsView() {
  useTitle('Transactions')
  const net = useNet()
  const now = useNow(1000)
  const indexed = !config.demo && net.api === 'up'
  const live = useLiveChain(60)
  const paged = usePagedList<TxSummary>(indexed ? (c) => api.txs(c, PAGE) : null, `txs:${indexed}`)
  if (networkDown(net)) return <Page><NotReachable what="the transaction list" /></Page>
  const rows = indexed ? paged.items : live.txs
  return (
    <Page>
      <PageHead eyebrow={config.networkName} title="Transactions">
        <span className="x-muted">{indexed ? 'Newest first, from the explorer API.' : `Transactions in the latest ${live.blocks.length || 60} blocks, read live from the ${config.demo ? 'demo chain' : 'node'}.`}</span>
      </PageHead>
      <Card flush className="x-section">
        <TxTable txs={rows} now={now} loading={!rows.length && (live.mode === 'connecting' || paged.loading)} empty={<p className="x-empty-row">No transactions in the recent blocks.</p>} />
      </Card>
      {indexed ? (
        <div className="x-more">
          <button type="button" className="xh-btn xh-btn-ghost xu-btn is-sm" onClick={paged.more} disabled={paged.loading || !paged.hasMore}>
            {paged.loading ? 'Loading…' : 'Older transactions'}
          </button>
        </div>
      ) : null}
    </Page>
  )
}

const typeLabel: Record<SearchResult['type'], string> = { block: 'Block', tx: 'Transaction', address: 'Address', name: 'Name' }
const target = (r: SearchResult) => (r.type === 'block' ? paths.block(r.id) : r.type === 'tx' ? paths.tx(r.id) : r.type === 'address' ? paths.address(r.id) : paths.name(r.id))

export function SearchView() {
  const q = useQueryParam('q')
  const net = useNet()
  useTitle(q ? `Search “${q}”` : 'Search')
  const query = q ?? ''
  const direct = query ? searchTarget(query) : null
  const isText = !!query && classifySearch(query).type === 'text'
  const r = useLoad(isText && !config.demo && net.api !== 'down' ? (s) => api.search(query, s) : null, `q:${query}:${net.api}`)

  useEffect(() => {
    if (direct && !direct.startsWith('/search')) window.location.replace(direct)
  }, [direct])
  useEffect(() => {
    const only = r.data?.results.length === 1 ? r.data.results[0] : null
    if (only) window.location.replace(target(only))
  }, [r.data])

  const demoNames = config.demo && isText ? Object.values(demo.DEMO_NAMES).filter((n) => n.includes(query.toLowerCase())) : []
  const results: SearchResult[] = r.data?.results ?? demoNames.map((n) => ({ type: 'name' as const, id: n, label: null }))
  return (
    <Page>
      <PageHead eyebrow="Search" title={query ? <span>“{query}”</span> : 'Search'} />
      {!query ? (
        <EmptyState title="Search the chain" description="Type a block number, a transaction hash, an address or a .xdevs name in the search box." />
      ) : r.loading ? (
        <p className="x-muted">Searching…</p>
      ) : results.length ? (
        <Card flush>
          <ul className="x-results">
            {results.map((x) => (
              <li key={`${x.type}:${x.id}`}>
                <Pill tone="info">{typeLabel[x.type]}</Pill>
                <a className="x-link x-mono x-break" href={target(x)}>
                  {x.id}
                </a>
                {x.label ? <span className="x-muted x-small">{x.label}</span> : null}
              </li>
            ))}
          </ul>
        </Card>
      ) : (
        <EmptyState
          title="Nothing matches"
          description={net.api === 'down' && !config.demo ? 'Free-text and name-prefix search needs the explorer API, which is not reachable. Exact block numbers, hashes, addresses and full .xdevs names still work.' : 'Try a block number, a transaction hash, an address or a .xdevs name.'}
        />
      )}
    </Page>
  )
}

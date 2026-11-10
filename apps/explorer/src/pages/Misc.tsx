import { useEffect } from 'react'
import { api, ApiError } from '../lib/api.ts'
import { formatDate, formatDuration } from '../lib/format.ts'
import { useAsync, useNow } from '../lib/hooks.ts'
import { browserStore, recordAddress } from '../lib/lookalike.ts'
import { navigate, paths } from '../lib/router.ts'
import type { SearchResult } from '../lib/types.ts'
import { AddressLink, Badge, BlockLink, Empty, ErrorBox, KV, Link, Loading, Section, TxLink } from '../components/ui.tsx'

export function NamePage({ name }: { name: string }) {
  const now = useNow(30_000)
  const { data: n, error, reload } = useAsync((s) => api.name(name, s), [name])
  if (error) {
    if (error instanceof ApiError && error.status === 404)
      return (
        <Section title={<h1 className="h-inline">{name}</h1>} id="name-missing">
          <Empty>This name has never been registered on this chain.</Empty>
        </Section>
      )
    return <ErrorBox error={error} onRetry={reload} />
  }
  if (!n) return <Loading what="Loading name" />
  const tone = n.status === 'active' ? 'ok' : n.status === 'grace' ? 'warn' : 'neutral'
  return (
    <>
      <div className="page-head">
        <h1 className="xns-name">{n.name}</h1>
        <Badge tone={tone}>{n.status === 'active' ? 'Active' : n.status === 'grace' ? 'In grace period (not resolving)' : 'Released'}</Badge>
      </div>
      <Section title="Name" id="name-overview">
        <KV
          rows={[
            ['Resolves to', n.address ? <AddressLink address={n.address} full /> : <span className="muted">nothing (expired names do not resolve)</span>],
            ['Owner', <AddressLink address={n.owner} full />],
            ['Primary name of', n.isPrimaryFor ? <AddressLink address={n.isPrimaryFor} /> : '—'],
            ['Expires', `${formatDate(n.expires)} (${n.expires > now ? `in ${formatDuration(n.expires - now)}` : `${formatDuration(now - n.expires)} ago`})`],
            ['Grace period ends', formatDate(n.graceEndsAt)],
            ['Registered', <span><BlockLink n={n.registeredBlock} />, <TxLink hash={n.registeredTx} /></span>],
            ['Node (namehash)', <span className="mono break small">{n.node}</span>],
          ]}
        />
        <p className="small muted pad">
          .xdevs names reject look-alikes at registration (<span className="mono">paypa1</span> cannot be registered while{' '}
          <span className="mono">paypal</span> exists). Still check the address before sending.
        </p>
      </Section>
    </>
  )
}

const typeLabel: Record<SearchResult['type'], string> = { block: 'Block', tx: 'Transaction', address: 'Address', name: 'Name' }
const target = (r: SearchResult) =>
  r.type === 'block' ? paths.block(r.id) : r.type === 'tx' ? paths.tx(r.id) : r.type === 'address' ? paths.address(r.id) : paths.name(r.id)

export function SearchPage({ q }: { q: string }) {
  const { data, error, reload } = useAsync((s) => (q ? api.search(q, s) : Promise.resolve({ query: '', results: [] })), [q])
  useEffect(() => {
    if (data?.results.length === 1) {
      const r = data.results[0]!
      if (r.type === 'address') recordAddress(browserStore(), r.id, 'search', Date.now())
      navigate(target(r), { replace: true })
    }
  }, [data])
  if (error) return <ErrorBox error={error} onRetry={reload} />
  if (!data) return <Loading what="Searching" />
  return (
    <Section title={<h1 className="h-inline">Search{q ? `: “${q}”` : ''}</h1>} id="search">
      {data.results.length === 0 ? (
        <Empty>{q ? 'Nothing matches. Try a block number, a transaction hash, an address or a .xdevs name.' : 'Type a block number, transaction hash, address or .xdevs name.'}</Empty>
      ) : (
        <ul className="list">
          {data.results.map((r) => (
            <li key={`${r.type}:${r.id}`}>
              <Badge tone="info">{typeLabel[r.type]}</Badge> <Link to={target(r)} className="mono break">{r.id}</Link>
              {r.label ? <span className="muted small"> {r.label}</span> : null}
            </li>
          ))}
        </ul>
      )}
    </Section>
  )
}

export function NotFound() {
  return (
    <Section title={<h1 className="h-inline">Page not found</h1>} id="nf">
      <Empty>
        That page does not exist. <Link to={paths.home()}>Go to the home page</Link>.
      </Empty>
    </Section>
  )
}

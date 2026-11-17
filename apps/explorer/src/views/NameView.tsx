'use client'

import { Card, EmptyState } from '@xdevs/ui'
import { Addr, KV, PageHead, Pill, SourceNote, useNow } from '../components/bits'
import { loadName } from '../lib/data'
import { formatDate, formatDuration, formatNumber } from '../lib/format'
import { paths, useRouteParam } from '../lib/routes'
import { useLoad } from '../lib/useLoad'
import { DetailSkeleton, MissingParam, Page, Pending, Problem, useTitle } from './common'

export function NameView() {
  const name = useRouteParam('name')
  const now = useNow(30_000)
  const r = useLoad(name ? (s) => loadName(name, s) : null, `n:${name}`)
  useTitle(name ?? null)

  if (name === undefined) return <Page><DetailSkeleton /></Page>
  if (name === null) return <Page><MissingParam what="name" example="/name/alice.xdevs" /></Page>
  if (!r.data) return <Page>{r.error ? <Problem error={r.error} onRetry={r.reload} what="this name" /> : <Pending what="this name" />}</Page>

  const { info: n, resolved, source } = r.data
  if (!n && !resolved)
    return (
      <Page>
        <PageHead eyebrow=".xdevs name" title={<span className="x-xns-title">{name}</span>} />
        <EmptyState title="Not registered" description="This name has never been registered on this chain, or it expired and no longer resolves." />
      </Page>
    )
  if (!n)
    return (
      <Page>
        <PageHead eyebrow=".xdevs name" title={<span className="x-xns-title">{name}</span>}>
          <Pill tone="ok" dot>resolves</Pill>
        </PageHead>
        <Card title="Resolution" className="x-section">
          <KV rows={[['Resolves to', <Addr address={resolved} full />]]} />
          <p className="x-small x-muted">Owner, expiry and history need the explorer API.</p>
        </Card>
        <SourceNote source={source} />
      </Page>
    )
  const tone = n.status === 'active' ? 'ok' : n.status === 'grace' ? 'warn' : 'neutral'
  return (
    <Page>
      <PageHead eyebrow=".xdevs name" title={<span className="x-xns-title">{n.name}</span>}>
        <Pill tone={tone} dot>
          {n.status === 'active' ? 'active' : n.status === 'grace' ? 'grace period · not resolving' : 'released'}
        </Pill>
      </PageHead>
      <Card title="Name" className="x-section">
        <KV
          rows={[
            ['Resolves to', n.address ? <Addr address={n.address} full /> : <span className="x-muted">nothing (expired names do not resolve)</span>],
            ['Owner', <Addr address={n.owner} full />],
            ['Primary name of', n.isPrimaryFor ? <Addr address={n.isPrimaryFor} copy={false} /> : '—'],
            ['Expires', <span className="x-num">{formatDate(n.expires)}{now ? <span className="x-muted"> · {n.expires > now ? `in ${formatDuration(n.expires - now)}` : `${formatDuration(now - n.expires)} ago`}</span> : null}</span>],
            ['Grace period ends', <span className="x-num">{formatDate(n.graceEndsAt)}</span>],
            ['Registered', <span><a className="x-link x-num" href={paths.block(n.registeredBlock)}>block {formatNumber(n.registeredBlock)}</a> · <a className="x-link x-mono" href={paths.tx(n.registeredTx)}>{n.registeredTx.slice(0, 12)}…</a></span>],
            ['Namehash', <span className="x-mono x-break x-small">{n.node}</span>],
          ]}
        />
        <p className="x-small x-muted x-pad-top">
          .xdevs names reject look-alikes at registration (<span className="x-mono">paypa1</span> cannot be registered while <span className="x-mono">paypal</span> exists). Still check the
          address before sending.
        </p>
      </Card>
      <SourceNote source={source} />
    </Page>
  )
}

'use client'

import { Card, CopyButton, EmptyState, Tabs } from '@xdevs/ui'
import { useEffect, useState, type ReactNode } from 'react'
import { KV, Native, PageHead, Pill, SourceNote, useNow } from '../components/bits'
import { ApprovalTable, LookAlikeBanner, SecurityBadges, TransferTable, TxTable, VaultTable } from '../components/tables'
import { loadAddress, loadAddressTransfers, loadAddressTxs, loadApprovals, loadVaultQueue } from '../lib/data'
import { formatNumber } from '../lib/format'
import { knownLabel } from '../lib/labels'
import { addressParts, browserStore, findLookAlikes, loadHistory, recordAddress, type HistoryEntry } from '../lib/lookalike'
import { paths, useQueryParam, useRouteParam } from '../lib/routes'
import type { AddressSummary } from '../lib/types'
import { useLoad } from '../lib/useLoad'
import { DetailSkeleton, MissingParam, Page, Pending, Problem, useTitle } from './common'

const TABS = ['txs', 'transfers', 'approvals', 'vault'] as const
type Tab = (typeof TABS)[number]

export function AddressView() {
  const address = useRouteParam('address')
  const tabParam = useQueryParam('tab')
  const now = useNow(5000)
  const r = useLoad(address ? (s) => loadAddress(address, s) : null, `a:${address}`)
  const [lookAlikes, setLookAlikes] = useState<HistoryEntry[]>([])
  const [tab, setTab] = useState<Tab>('txs')
  useTitle(address ? `Address ${address.slice(0, 8)}…` : null)

  useEffect(() => {
    if (tabParam && (TABS as readonly string[]).includes(tabParam)) setTab(tabParam as Tab)
  }, [tabParam])

  // Compare with the history BEFORE recording this visit, then record it.
  useEffect(() => {
    if (!address || !/^0x[0-9a-fA-F]{40}$/.test(address)) return
    const store = browserStore()
    setLookAlikes(findLookAlikes(address, loadHistory(store)))
    recordAddress(store, address, 'view', Date.now())
  }, [address])

  if (address === undefined) return <Page><DetailSkeleton /></Page>
  if (address === null) return <Page><MissingParam what="address" example="/address/0x…" /></Page>
  if (!r.data) return <Page>{r.error ? <Problem error={r.error} onRetry={r.reload} what="this address" /> : <Pending what="this address" />}</Page>

  const { summary: a, source } = r.data
  const select = (id: string) => {
    setTab(id as Tab)
    window.history.replaceState(null, '', paths.address(address, id === 'txs' ? undefined : id))
  }
  const label = a.xns.primaryName ?? knownLabel(a.address) ?? a.labels[0] ?? null
  const kind = a.kind === 'delegated' ? 'Smart account' : a.kind === 'contract' ? (a.token ? 'Token contract' : 'Contract') : 'Account'
  const p = addressParts(a.address)

  return (
    <Page>
      <LookAlikeBanner address={a.address} matches={lookAlikes} />
      <PageHead eyebrow={kind} title={label ? <span className="x-xns-title">{label}</span> : <span className="x-mono x-title-hash">{a.address.slice(0, 10)}…{a.address.slice(-8)}</span>}>
        <span className="x-addr-line">
          <span className="x-mono x-break" aria-label={a.address}>
            <b>{p.head}</b>
            <span className="x-addr-mid">{p.middle}</span>
            <b>{p.tail}</b>
          </span>
          <CopyButton value={a.address} title="Copy address" />
        </span>
      </PageHead>
      <div className="x-badges x-section-tight">
        {a.labels.map((l) => (
          <Pill key={l} tone="info">
            {l}
          </Pill>
        ))}
        <SecurityBadges s={a.security} />
        {a.token ? (
          <a className="x-link x-small" href={paths.token(a.address)}>
            View token {a.token.symbol ?? ''} →
          </a>
        ) : null}
      </div>

      {a.security.frozen ? (
        <div className="x-notice is-danger" role="alert">
          <div className="x-notice-body">This account is frozen by its guardians. Nothing can execute from it until the freeze ends or a recovery completes.</div>
        </div>
      ) : null}

      <div className="x-overview-grid">
        <Card title="Balance" className="x-balance-card">
          {a.balance !== null ? (
            <p className="x-balance">
              <Native value={a.balance} digits={6} strong />
            </p>
          ) : (
            <p className="x-muted">The node could not be reached, so the balance is not shown.</p>
          )}
          <p className="x-muted x-small">
            Nonce <span className="x-num">{a.nonce ?? '—'}</span>
            {a.txCount ? (
              <>
                {' '}
                · <span className="x-num">{formatNumber(a.txCount)}</span> transactions
              </>
            ) : null}
          </p>
        </Card>
        <Card title="Account" className="x-section-flat">
          <KV
            rows={[
              ['Type', a.kind === 'delegated' ? <span>EOA delegated (EIP-7702) to <a className="x-link" href={paths.address(a.delegatedTo ?? '')}>{knownLabel(a.delegatedTo) ?? a.delegatedTo}</a></span> : a.kind === 'contract' ? 'Contract' : a.kind === 'eoa' ? 'Externally owned account' : '—'],
              ['EOA lock (XIP-0001)', <EoaLock s={a.security.eoaLock} />],
              ['.xdevs name', a.xns.primaryName ? <a className="x-link x-xns" href={paths.name(a.xns.primaryName)}>{a.xns.primaryName}</a> : <span className="x-muted">no primary name</span>],
              a.security.guardians ? ['Guardians', <span className="x-num">{a.security.guardians.count}{a.security.guardians.threshold ? <span className="x-muted"> · threshold {a.security.guardians.threshold}</span> : null}</span>] : null,
              a.firstSeenBlock !== null ? ['First seen', <a className="x-link x-num" href={paths.block(a.firstSeenBlock)}>block {formatNumber(a.firstSeenBlock)}</a>] : null,
            ]}
          />
        </Card>
      </div>

      <Tabs
        className="x-section"
        label="Address activity"
        value={tab}
        onChange={select}
        items={[
          { id: 'txs', label: 'Transactions', count: a.txCount ? formatNumber(a.txCount) : undefined, content: tab === 'txs' ? <TxsTab address={a.address} now={now} /> : null },
          { id: 'transfers', label: 'Transfers', content: tab === 'transfers' ? <TransfersTab address={a.address} now={now} /> : null },
          { id: 'approvals', label: 'Approvals', content: tab === 'approvals' ? <ApprovalsTab address={a.address} now={now} /> : null },
          { id: 'vault', label: 'Vault queue', count: a.security.openVaultActions || undefined, content: tab === 'vault' ? <VaultTab address={a.address} now={now} summary={a} /> : null },
        ]}
      />
      <SourceNote source={source} />
    </Page>
  )
}

function EoaLock({ s }: { s: AddressSummary['security']['eoaLock'] }) {
  const text: Record<typeof s.status, ReactNode> = {
    unlocked: <span>unlocked <span className="x-muted x-small">· the EOA key can send any transaction</span></span>,
    locked: <span><Pill tone="ok">locked</Pill> <span className="x-muted x-small">the EOA key can only call its own smart account</span></span>,
    sealed: <span><Pill tone="danger">sealed</Pill> <span className="x-muted x-small">no transaction from this key is valid</span></span>,
    unknown: <span className="x-muted">unknown (node does not serve xdevs_getEoaLock)</span>,
  }
  return (
    <span>
      {text[s.status]}
      {s.active === false ? <span className="x-muted x-small"> · protocol enforcement not active yet</span> : null}
    </span>
  )
}

function IndexerNeeded({ what }: { what: string }) {
  return <EmptyState title="Needs the explorer API" description={`${what} come from the indexer, which is not reachable right now. Balances, nonce and the EOA lock above are read live from the node.`} />
}

function SpamToggle({ show, setShow, hidden }: { show: boolean; setShow: (b: boolean) => void; hidden: number | null | undefined }) {
  return (
    <label className="x-toggle">
      <input type="checkbox" checked={show} onChange={(e) => setShow(e.target.checked)} />
      Show likely spam{hidden ? ` (${hidden >= 1000 ? '1000+' : hidden} hidden)` : ''}
    </label>
  )
}

function TxsTab({ address, now }: { address: string; now: number }) {
  const [show, setShow] = useState(false)
  const r = useLoad((s) => loadAddressTxs(address, null, show, s), `atx:${address}:${show}`)
  if (r.data?.unavailable) return <IndexerNeeded what="Transaction lists" />
  return (
    <Card flush title="Transactions" actions={<SpamToggle show={show} setShow={setShow} hidden={r.data?.hiddenSpam} />}>
      <TxTable txs={r.data?.items ?? []} now={now} perspective={address} loading={r.loading && !r.data} empty={<p className="x-empty-row">No transactions.</p>} />
    </Card>
  )
}

function TransfersTab({ address, now }: { address: string; now: number }) {
  const [show, setShow] = useState(false)
  const r = useLoad((s) => loadAddressTransfers(address, null, show, s), `atr:${address}:${show}`)
  if (r.data?.unavailable) return <IndexerNeeded what="Transfer lists" />
  return (
    <Card flush title="Native and token transfers" actions={<SpamToggle show={show} setShow={setShow} hidden={r.data?.hiddenSpam} />}>
      <TransferTable transfers={r.data?.items ?? []} now={now} perspective={address} loading={r.loading && !r.data} empty={<p className="x-empty-row">No transfers.</p>} />
      <p className="x-small x-muted x-card-foot">Zero-value transfers, dust from look-alike addresses and fake tokens are hidden by default. Never copy an address from your history without checking all of it.</p>
    </Card>
  )
}

function ApprovalsTab({ address, now }: { address: string; now: number }) {
  const [expired, setExpired] = useState(false)
  const r = useLoad((s) => loadApprovals(address, expired, s), `apr:${address}:${expired}`)
  if (r.data?.unavailable) return <IndexerNeeded what="Approvals" />
  const items = r.data?.items ?? []
  return (
    <Card
      flush
      title="Token approvals"
      actions={
        <label className="x-toggle">
          <input type="checkbox" checked={expired} onChange={(e) => setExpired(e.target.checked)} /> Include expired
        </label>
      }
    >
      {items.some((x) => x.unlimited || x.expiry === null) ? (
        <div className="x-notice is-warn x-card-notice" role="note">
          <div className="x-notice-body">Some approvals are unlimited or never expire. Revoke what you no longer use: stale approvals are the main way wallets get drained.</div>
        </div>
      ) : null}
      {r.loading && !r.data ? <DetailSkeleton rows={3} /> : items.length ? <ApprovalTable items={items} now={now} /> : <p className="x-empty-row">No live approvals.</p>}
    </Card>
  )
}

function VaultTab({ address, now, summary }: { address: string; now: number; summary: AddressSummary }) {
  const [all, setAll] = useState(false)
  const r = useLoad((s) => loadVaultQueue(address, null, all, s), `vq:${address}:${all}`)
  if (r.data?.unavailable) return <IndexerNeeded what="Guardian Vault queues" />
  const items = r.data?.items ?? []
  return (
    <Card
      flush
      title="Guardian Vault queue"
      actions={
        <label className="x-toggle">
          <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} /> Show history
        </label>
      }
    >
      {r.loading && !r.data ? (
        <DetailSkeleton rows={3} />
      ) : items.length ? (
        <VaultTable items={items} now={now} />
      ) : (
        <p className="x-empty-row">{summary.security.vaultEnabled ? (all ? 'No vault actions yet.' : 'Nothing queued.') : 'This address has no Guardian Vault.'}</p>
      )}
      <p className="x-small x-muted x-card-foot">
        Large transfers from a vault wait out the account's delay. The owner (or the guardian threshold) can cancel them until they execute. If you did not queue an action, cancel it from
        your wallet now.
      </p>
    </Card>
  )
}

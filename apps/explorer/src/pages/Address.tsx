import { useEffect, useState, type ReactNode } from 'react'
import { api } from '../lib/api.ts'
import { config } from '../lib/config.ts'
import { formatNative, formatNumber } from '../lib/format.ts'
import { useAsync, useNow, usePaged } from '../lib/hooks.ts'
import { browserStore, findLookAlikes, loadHistory, recordAddress, type HistoryEntry } from '../lib/lookalike.ts'
import { navigate, paths } from '../lib/router.ts'
import { ApprovalTable, LookAlikeBanner, SecurityBadges, TransferTable, TxTable, VaultTable } from '../components/tables.tsx'
import { AddressLink, Badge, BlockLink, Copy, Empty, ErrorBox, FullAddress, KV, Link, LoadMore, Loading, Section } from '../components/ui.tsx'

const TABS = [
  ['txs', 'Transactions'],
  ['transfers', 'Transfers'],
  ['approvals', 'Approvals'],
  ['vault', 'Vault queue'],
] as const
type Tab = (typeof TABS)[number][0]

export function AddressPage({ address, tab }: { address: string; tab: string | null }) {
  const valid = /^0x[0-9a-fA-F]{40}$/.test(address)
  const now = useNow(5000)
  const { data: a, error, reload } = useAsync((s) => (valid ? api.address(address, s) : Promise.reject(new Error(`"${address}" is not an address`))), [address])
  const [lookAlikes, setLookAlikes] = useState<HistoryEntry[]>([])

  // Compare with the history BEFORE recording this visit, then record it.
  useEffect(() => {
    if (!valid) return
    const store = browserStore()
    setLookAlikes(findLookAlikes(address, loadHistory(store)))
    recordAddress(store, address, 'view', Date.now())
  }, [address, valid])

  const active: Tab = (TABS.find(([k]) => k === tab)?.[0] ?? 'txs') as Tab
  if (error) return <ErrorBox error={error} onRetry={valid ? reload : undefined} />
  return (
    <>
      <LookAlikeBanner address={address} matches={lookAlikes} />
      <div className="page-head">
        <h1>
          {a?.xns.primaryName ? <span className="xns-name">{a.xns.primaryName}</span> : a?.kind === 'contract' ? 'Contract' : 'Address'}
        </h1>
        {a ? <span className="badges">{a.labels.map((l) => <Badge key={l} tone="info">{l}</Badge>)}</span> : null}
      </div>
      <p className="addr-line">
        <FullAddress address={a?.address ?? address} /> <Copy text={a?.address ?? address} label="Copy address" />
      </p>
      {!a ? (
        <Loading what="Loading address" />
      ) : (
        <>
          <SecurityBadges s={a.security} />
          {a.security.frozen ? (
            <div className="notice notice-danger" role="alert">
              This account is frozen by its guardians. Nothing can execute from it until the freeze ends or a recovery completes.
            </div>
          ) : null}
          {!a.live ? (
            <div className="notice notice-warn" role="note">
              The node could not be reached, so the balance and nonce are not shown.
            </div>
          ) : null}
          <Section title="Overview" id="addr-overview">
            <KV
              rows={[
                ['Balance', a.balance !== null ? <strong>{formatNative(a.balance, config.nativeSymbol, 8)}</strong> : '—'],
                ['Type', a.kind === 'delegated' ? <span>EOA with EIP-7702 delegation to <AddressLink address={a.delegatedTo} /></span> : a.kind === 'contract' ? 'Contract' : a.kind === 'eoa' ? 'Externally owned account' : '—'],
                ['Nonce', a.nonce !== null ? String(a.nonce) : '—'],
                ['Transactions (indexed)', formatNumber(a.txCount)],
                ['First seen', a.firstSeenBlock !== null ? <BlockLink n={a.firstSeenBlock} /> : 'never'],
                ['EOA lock (XIP-0001)', `${a.security.eoaLock.status}${a.security.eoaLock.source === 'indexer' ? ' (from indexed events)' : ''}`],
                ...(a.security.guardians ? [['Guardians', `${a.security.guardians.count}${a.security.guardians.threshold ? `, threshold ${a.security.guardians.threshold}` : ''}`] as [string, string]] : []),
                ...(a.xns.names.length
                  ? [['.xdevs names', <span className="badges">{a.xns.names.map((n) => <Link key={n.name} to={paths.name(n.name)}>{n.name}{n.status !== 'active' ? ` (${n.status})` : ''}</Link>)}</span>] as [string, ReactNode]]
                  : []),
                ...(a.token ? [['Token', `${a.token.name ?? '?'} (${a.token.symbol ?? '?'}), ${a.token.decimals ?? '?'} decimals${a.token.known ? '' : ' — not verified'}`] as [string, string]] : []),
              ]}
            />
          </Section>
        </>
      )}
      <nav className="tabs" aria-label="Address sections">
        {TABS.map(([k, label]) => (
          <a
            key={k}
            href={paths.address(address, k)}
            aria-current={active === k ? 'page' : undefined}
            onClick={(e) => {
              e.preventDefault()
              navigate(paths.address(address, k), { replace: true })
            }}
          >
            {label}
            {k === 'vault' && a?.security.openVaultActions ? <span className="tab-count">{a.security.openVaultActions}</span> : null}
          </a>
        ))}
      </nav>
      {valid ? (
        active === 'txs' ? <TxsTab address={address} now={now} /> : active === 'transfers' ? <TransfersTab address={address} now={now} /> : active === 'approvals' ? <ApprovalsTab address={address} now={now} /> : <VaultTab address={address} now={now} />
      ) : null}
    </>
  )
}

function SpamToggle({ show, setShow, hidden }: { show: boolean; setShow: (b: boolean) => void; hidden: number | null }) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={show} onChange={(e) => setShow(e.target.checked)} />
      Show likely spam{hidden ? ` (${hidden >= 1000 ? '1000+' : hidden} hidden)` : ''}
    </label>
  )
}

function TxsTab({ address, now }: { address: string; now: number }) {
  const [show, setShow] = useState(false)
  const p = usePaged((c, s) => api.addressTxs(address, { cursor: c, includeSpam: show }, s), [address, show])
  return (
    <Section title="Transactions" id="tab-txs" actions={<SpamToggle show={show} setShow={setShow} hidden={p.hiddenSpam} />}>
      {p.error ? <ErrorBox error={p.error} /> : null}
      {p.items.length ? <TxTable txs={p.items} now={now} perspective={address} /> : p.loading ? <Loading /> : <Empty>No transactions.</Empty>}
      <LoadMore hasMore={p.hasMore} loading={p.loading} onClick={p.loadMore} />
    </Section>
  )
}

function TransfersTab({ address, now }: { address: string; now: number }) {
  const [show, setShow] = useState(false)
  const p = usePaged((c, s) => api.addressTransfers(address, { cursor: c, includeSpam: show }, s), [address, show])
  return (
    <Section title="Transfers (native and tokens)" id="tab-transfers" actions={<SpamToggle show={show} setShow={setShow} hidden={p.hiddenSpam} />}>
      {p.error ? <ErrorBox error={p.error} /> : null}
      {p.items.length ? <TransferTable transfers={p.items} now={now} perspective={address} /> : p.loading ? <Loading /> : <Empty>No transfers.</Empty>}
      <LoadMore hasMore={p.hasMore} loading={p.loading} onClick={p.loadMore} />
      <p className="small muted pad">
        Zero-value transfers, dust from look-alike addresses and fake tokens are hidden by default. Never copy an address from your history without
        checking all of it.
      </p>
    </Section>
  )
}

function ApprovalsTab({ address, now }: { address: string; now: number }) {
  const [expired, setExpired] = useState(false)
  const { data, error } = useAsync((s) => api.approvals(address, expired, s), [address, expired])
  return (
    <Section
      title="Token approvals"
      id="tab-approvals"
      actions={
        <label className="toggle">
          <input type="checkbox" checked={expired} onChange={(e) => setExpired(e.target.checked)} /> Include expired
        </label>
      }
    >
      {error ? <ErrorBox error={error} /> : null}
      {!data ? (
        <Loading />
      ) : data.items.length ? (
        <>
          {data.items.some((x) => x.unlimited || x.expiry === null) ? (
            <div className="notice notice-warn" role="note">
              Some approvals are unlimited or never expire. Revoke what you no longer use: stale approvals are the main way wallets get drained.
            </div>
          ) : null}
          <ApprovalTable items={data.items} now={now} />
        </>
      ) : (
        <Empty>No live approvals.</Empty>
      )}
      {data && !data.live ? <p className="small muted pad">Remaining allowances could not be read from the node.</p> : null}
    </Section>
  )
}

function VaultTab({ address, now }: { address: string; now: number }) {
  const [all, setAll] = useState(false)
  const p = usePaged((c, s) => api.vaultQueue(address, { cursor: c, status: all ? 'all' : 'open' }, s), [address, all])
  return (
    <Section
      title="Guardian Vault queue"
      id="tab-vault"
      actions={
        <label className="toggle">
          <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} /> Show history
        </label>
      }
    >
      {p.error ? <ErrorBox error={p.error} /> : null}
      {p.items.length ? <VaultTable items={p.items} now={now} /> : p.loading ? <Loading /> : <Empty>{all ? 'No vault actions.' : 'Nothing queued.'}</Empty>}
      <LoadMore hasMore={p.hasMore} loading={p.loading} onClick={p.loadMore} />
      <p className="small muted pad">
        Queued actions wait out the account's delay. The owner (or the guardian threshold) can cancel them until they execute. If you did not queue
        an action, cancel it from your wallet now.
      </p>
    </Section>
  )
}

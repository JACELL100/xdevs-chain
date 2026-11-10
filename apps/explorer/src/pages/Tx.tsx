import { api } from '../lib/api.ts'
import { config } from '../lib/config.ts'
import { formatDate, formatGwei, formatNative, formatNumber, percent, timeAgo, toBig, TX_TYPES } from '../lib/format.ts'
import { useAsync, useNow } from '../lib/hooks.ts'
import type { DecodedLog } from '../lib/types.ts'
import { ApprovalChanges, TransferTable } from '../components/tables.tsx'
import { AddressLink, Badge, BlockLink, Copy, ErrorBox, KV, Loading, Section, SpamBadges } from '../components/ui.tsx'

export function TxPage({ hash }: { hash: string }) {
  const now = useNow(5000)
  const { data: t, error, reload } = useAsync((s) => api.tx(hash, s), [hash])
  if (error) return <ErrorBox error={error} onRetry={reload} />
  if (!t) return <Loading what="Loading transaction" />
  const fb = t.feeBreakdown
  const total = toBig(fb.total)
  return (
    <>
      <div className="page-head">
        <h1>Transaction</h1>
        <span className="badges">
          {t.status === 'success' ? <Badge tone="ok">Success</Badge> : <Badge tone="danger">Reverted</Badge>}
          <Badge tone="info">{TX_TYPES[t.type] ?? `Type ${t.type}`}</Badge>
          {t.spam ? <SpamBadges reasons={t.spamReasons} /> : null}
        </span>
      </div>
      {t.spam ? (
        <div className="notice notice-warn" role="note">
          This transaction is flagged as likely spam ({t.spamReasons.join(', ')}). Do not copy addresses from it.
        </div>
      ) : null}
      <Section title="Overview" id="tx-overview">
        <KV
          rows={[
            ['Hash', <span className="mono break">{t.hash} <Copy text={t.hash} /></span>],
            ['Block', <span><BlockLink n={t.blockNumber} /> <span className="muted small">(index {t.txIndex})</span></span>],
            ['Timestamp', `${formatDate(t.timestamp)} (${timeAgo(t.timestamp, now)})`],
            ['From', <AddressLink address={t.from} full />],
            [t.contractCreated ? 'Created contract' : 'To', t.contractCreated ? <AddressLink address={t.contractCreated} full /> : <AddressLink address={t.to} full />],
            ['Value', <strong>{formatNative(t.value, config.nativeSymbol, 18)}</strong>],
            ['Nonce', String(t.nonce)],
            ['Gas', `${formatNumber(Number(toBig(t.gasUsed)))} used of ${formatNumber(Number(toBig(t.gas)))} (${percent(toBig(t.gasUsed), toBig(t.gas))})`],
            ['Gas price', `${formatGwei(t.effectiveGasPrice)}${t.maxFeePerGas ? ` (max ${formatGwei(t.maxFeePerGas)}, tip cap ${formatGwei(t.maxPriorityFeePerGas)})` : ''}`],
          ]}
        />
      </Section>
      <Section title="Fees" id="tx-fees">
        <KV
          rows={[
            ['Total fee', <strong>{formatNative(fb.total, config.nativeSymbol, 10)}</strong>],
            ['Burned', `${formatNative(fb.burned, config.nativeSymbol, 10)} (${percent(toBig(fb.burned), total)})`],
            ['To Treasury', `${formatNative(fb.treasury, config.nativeSymbol, 10)} (${percent(toBig(fb.treasury), total)})`],
            ['Tip to proposer', `${formatNative(fb.priorityFee, config.nativeSymbol, 10)} (${percent(toBig(fb.priorityFee), total)})`],
          ]}
        />
        <p className="small muted pad">
          Base fee × gas used is split {fb.burnBps / 100}% burned / {100 - fb.burnBps / 100}% to the Treasury; the priority fee goes to the block
          proposer.
        </p>
      </Section>
      {t.approvals.length ? (
        <Section title={`Approvals (${t.approvals.length})`} id="tx-approvals">
          <ApprovalChanges items={t.approvals} now={now} />
        </Section>
      ) : null}
      {t.tokenTransfers.length ? (
        <Section title={`Transfers (${t.tokenTransfers.length})`} id="tx-transfers">
          <TransferTable transfers={t.tokenTransfers} now={now} />
        </Section>
      ) : null}
      {t.authorizationList?.length ? (
        <Section title="EIP-7702 authorizations" id="tx-auth">
          <ul className="list">
            {t.authorizationList.map((a, i) => (
              <li key={i}>
                Delegate to <AddressLink address={a.address} /> (chain {BigInt(a.chainId).toString()}, nonce {BigInt(a.nonce).toString()})
                {a.address.toLowerCase() === '0x0000000000000000000000000000000000001005' ? <Badge tone="info">xDEVS Smart Account</Badge> : null}
              </li>
            ))}
          </ul>
        </Section>
      ) : null}
      <Section title={`Logs (${t.logs.length})`} id="tx-logs">
        {t.logs.length ? t.logs.map((l) => <LogView key={l.logIndex} log={l} />) : <p className="pad muted">No logs.</p>}
      </Section>
      <Section title="Input data" id="tx-input">
        <details className="pad">
          <summary>{t.input === '0x' ? 'Empty' : `${(t.input.length - 2) / 2} bytes${t.methodId ? `, method ${t.methodId}` : ''}`}</summary>
          <pre className="mono break code">{t.input}</pre>
        </details>
      </Section>
    </>
  )
}

function LogView({ log }: { log: DecodedLog }) {
  return (
    <details className="log">
      <summary>
        <span className="muted">#{log.logIndex}</span> {log.event ? <strong>{log.event}</strong> : <span className="muted">Unknown event</span>}{' '}
        {log.standard ? <Badge tone="neutral">{log.standard}</Badge> : null} <span className="small">from</span> <AddressLink address={log.address} />
      </summary>
      {log.signature ? <p className="mono small">{log.signature}</p> : null}
      {log.args ? (
        <dl className="kv small">
          {Object.entries(log.args).map(([k, v]) => (
            <div className="kv-row" key={k}>
              <dt>{k}</dt>
              <dd className="mono break">{typeof v === 'string' && /^0x[0-9a-fA-F]{40}$/.test(v) ? <AddressLink address={v} /> : JSON.stringify(v)}</dd>
            </div>
          ))}
        </dl>
      ) : (
        <div className="mono small break">
          {log.topics.map((t, i) => (
            <div key={i}>
              topic{i}: {t}
            </div>
          ))}
          <div>data: {log.data}</div>
        </div>
      )}
    </details>
  )
}

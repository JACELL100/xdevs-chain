'use client'

import { Card, CopyButton } from '@xdevs/ui'
import { predeploys } from '@xdevs/sdk'
import { Addr, KV, Native, PageHead, Pill, SourceNote, useNow } from '../components/bits'
import { ApprovalChanges, SpamBadges, TransferTable, vaultStatusText } from '../components/tables'
import { config } from '../lib/config'
import { loadTx, loadVaultAction, type TxResult } from '../lib/data'
import { displayValue, type DecodedCall, type VaultActivity } from '../lib/decode'
import { formatDate, formatDuration, formatGwei, formatNumber, percent, shortHash, timeAgo, toBig, TX_TYPES, vaultStatusTone } from '../lib/format'
import { paths, useRouteParam } from '../lib/routes'
import type { DecodedLog } from '../lib/types'
import { useLoad } from '../lib/useLoad'
import { MissingParam, Page, Pending, Problem, useTitle, DetailSkeleton } from './common'

export function TxView() {
  const hash = useRouteParam('tx')
  const now = useNow(1000)
  const r = useLoad(hash ? (s) => loadTx(hash, s) : null, `tx:${hash}`)
  useTitle(hash ? `Tx ${hash.slice(0, 10)}…` : null)

  if (hash === undefined) return <Page><DetailSkeleton /></Page>
  if (hash === null) return <Page><MissingParam what="transaction" example="/tx/0x…" /></Page>
  if (!r.data) return <Page>{r.error ? <Problem error={r.error} onRetry={r.reload} what="this transaction" /> : <Pending what="this transaction" />}</Page>

  const { tx: t, call, source } = r.data
  const fb = t.feeBreakdown
  const total = toBig(fb.total)
  return (
    <Page>
      <PageHead eyebrow="Transaction" title={<span className="x-mono x-title-hash">{shortHash(t.hash, 10)}</span>}>
        {t.status === 'success' ? <Pill tone="ok" dot>success</Pill> : <Pill tone="danger" dot>reverted</Pill>}
        <Pill tone="ok">final</Pill>
        <Pill tone="neutral">{TX_TYPES[t.type] ?? `type ${t.type}`}</Pill>
        {t.spam ? <SpamBadges reasons={t.spamReasons} /> : null}
        <span className="x-muted">
          {formatDate(t.timestamp)}
          {now ? ` · ${timeAgo(t.timestamp, now)}` : ''}
        </span>
      </PageHead>

      {t.spam ? (
        <div className="x-notice is-warn" role="note">
          <div className="x-notice-body">This transaction is flagged as likely spam. Do not copy addresses from it.</div>
        </div>
      ) : null}

      <VaultPanel r={r.data} now={now} />

      <Card title="Overview" className="x-section">
        <KV
          rows={[
            ['Hash', <span className="x-hash-row"><span className="x-mono x-break">{t.hash}</span><CopyButton value={t.hash} title="Copy hash" /></span>],
            ['Block', <span><a className="x-link x-num" href={paths.block(t.blockNumber)}>{formatNumber(t.blockNumber)}</a> <span className="x-muted x-small">· position {t.txIndex}</span></span>],
            ['From', <Addr address={t.from} full />],
            [t.contractCreated ? 'Created contract' : 'To', t.contractCreated ? <Addr address={t.contractCreated} full /> : <Addr address={t.to} full />],
            ['Value', <Native value={t.value} digits={18} strong />],
            ['Nonce', <span className="x-num">{t.nonce}</span>],
            [
              'Gas',
              <span className="x-num">
                {formatNumber(Number(toBig(t.gasUsed)))} <span className="x-muted">used of {formatNumber(Number(toBig(t.gas)))} ({percent(toBig(t.gasUsed), toBig(t.gas))})</span>
              </span>,
            ],
            ['Gas price', <span className="x-num">{formatGwei(t.effectiveGasPrice)}{t.maxFeePerGas ? <span className="x-muted"> · max {formatGwei(t.maxFeePerGas)}, tip cap {formatGwei(t.maxPriorityFeePerGas)}</span> : null}</span>],
          ]}
        />
      </Card>

      <Card title="Input" className="x-section" actions={call ? <Pill tone="info">{call.contract ?? standardLabel(call.standard)}</Pill> : null}>
        {call ? <DecodedCallView call={call} /> : t.input === '0x' ? <p className="x-muted">No calldata (plain {config.nativeSymbol} transfer).</p> : <p className="x-muted">Not a known xDEVS predeploy or token call.</p>}
        {t.input !== '0x' ? (
          <details className="x-raw">
            <summary>
              Raw calldata · {formatNumber((t.input.length - 2) / 2)} bytes{t.methodId ? ` · selector ${t.methodId}` : ''}
            </summary>
            <div className="x-raw-body">
              <pre className="x-mono x-code">{t.input}</pre>
              <CopyButton value={t.input} label="Copy calldata" />
            </div>
          </details>
        ) : null}
      </Card>

      <Card title="Fees" className="x-section">
        <div className="x-fee-split" role="img" aria-label="Fee split between burn, Treasury and proposer tip">
          <span className="is-burn" style={{ flexGrow: Number(toBig(fb.burned)) || 0 }} />
          <span className="is-treasury" style={{ flexGrow: Number(toBig(fb.treasury)) || 0 }} />
          <span className="is-tip" style={{ flexGrow: Number(toBig(fb.priorityFee)) || 0 }} />
        </div>
        <KV
          rows={[
            ['Total fee', <Native value={fb.total} digits={10} strong />],
            ['Burned', <span><span className="x-legend is-burn" /> <Native value={fb.burned} digits={10} /> <span className="x-muted">({percent(toBig(fb.burned), total)})</span></span>],
            ['To Treasury', <span><span className="x-legend is-treasury" /> <Native value={fb.treasury} digits={10} /> <span className="x-muted">({percent(toBig(fb.treasury), total)})</span></span>],
            ['Tip to proposer', <span><span className="x-legend is-tip" /> <Native value={fb.priorityFee} digits={10} /> <span className="x-muted">({percent(toBig(fb.priorityFee), total)})</span></span>],
          ]}
        />
        <p className="x-small x-muted x-pad-top">
          Base fee × gas used is split {fb.burnBps / 100}% burned / {100 - fb.burnBps / 100}% to the Treasury predeploy; the priority fee goes to the block proposer.
        </p>
      </Card>

      {t.approvals.length ? (
        <Card title={`Approvals (${t.approvals.length})`} className="x-section">
          <ApprovalChanges items={t.approvals} now={now} />
        </Card>
      ) : null}
      {t.tokenTransfers.length ? (
        <Card title={`Transfers (${t.tokenTransfers.length})`} flush className="x-section">
          <TransferTable transfers={t.tokenTransfers} now={now} />
        </Card>
      ) : null}
      {t.authorizationList?.length ? (
        <Card title="EIP-7702 authorizations" className="x-section">
          <ul className="x-list">
            {t.authorizationList.map((a, i) => (
              <li key={i}>
                Delegate to <Addr address={a.address} copy={false} /> <span className="x-muted x-small">chain {BigInt(a.chainId).toString()}, nonce {BigInt(a.nonce).toString()}</span>{' '}
                {a.address.toLowerCase() === predeploys.SMART_ACCOUNT_IMPL.toLowerCase() ? <Pill tone="ok">xDEVS Smart Account</Pill> : <Pill tone="warn">not the xDEVS account</Pill>}
              </li>
            ))}
          </ul>
        </Card>
      ) : null}
      <Card title={`Logs (${t.logs.length})`} className="x-section">
        {t.logs.length ? <div className="x-logs">{t.logs.map((l) => <LogView key={l.logIndex} log={l} />)}</div> : <p className="x-muted">No logs.</p>}
      </Card>
      <SourceNote source={source} />
    </Page>
  )
}

const standardLabel = (s: string) => ({ erc20: 'ERC-20', xdrc20: 'XDRC-20', wxds: 'WXDS', staking: 'Staking', governance: 'Governance', xns: 'XNS', vault: 'Guardian Vault', account: 'Smart Account', permit2: 'Permit2Expiring' })[s] ?? s

function ArgValue({ type, value }: { type: string; value: unknown }) {
  if (type === 'address' && typeof value === 'string') return <Addr address={value} copy={false} />
  if (type === 'bytes' && typeof value === 'string' && value.length > 66)
    return (
      <span className="x-mono x-break" title={value}>
        {value.slice(0, 66)}… <span className="x-muted">({(value.length - 2) / 2} bytes)</span>
      </span>
    )
  if (type.startsWith('uint') && typeof value === 'bigint' && value >= 10n ** 15n) {
    return (
      <span className="x-num" title={value.toString()}>
        {value.toString()} <span className="x-muted x-small">({(Number(value) / 1e18).toLocaleString('en-US', { maximumFractionDigits: 6 })} × 10¹⁸)</span>
      </span>
    )
  }
  return <span className="x-mono x-break">{displayValue(value)}</span>
}

function DecodedCallView({ call }: { call: DecodedCall }) {
  return (
    <div className="x-call">
      <p className="x-call-sig">
        <span className="x-call-fn">{call.name}</span>
        <span className="x-muted">({call.args.map((a) => `${a.type} ${a.name}`).join(', ')})</span>
      </p>
      {call.args.length ? (
        <dl className="x-kv is-compact">
          {call.args.map((a) => (
            <div className="x-kv-row" key={a.name}>
              <dt>
                {a.name} <span className="x-dim x-small">{a.type}</span>
              </dt>
              <dd>
                <ArgValue type={a.type} value={a.value} />
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
    </div>
  )
}

/** Guardian Vault context: an action queued / cancelled / executed by this tx, or a cancel/execute call. */
function VaultPanel({ r, now }: { r: TxResult; now: number }) {
  const first: VaultActivity | undefined = r.vault[0]
  const account = first?.account ?? r.vaultCall?.account ?? null
  const id = first?.id ?? r.vaultCall?.id ?? null
  const current = useLoad(account && id !== null ? () => loadVaultAction(account, id) : null, `va:${account}:${id}`)
  if (!account || id === null) return null
  const queued = r.vault.find((v) => v.kind === 'queued')
  const action = current.data
  const executeAfter = queued?.executeAfter ?? action?.executeAfter ?? null
  const status = action?.status ?? (r.vault.some((v) => v.kind === 'executed') ? 'executed' : r.vault.some((v) => v.kind === 'cancelled') ? 'cancelled' : 'queued')
  const left = executeAfter && now ? executeAfter - now : null
  return (
    <section className="x-vault" aria-labelledby="vault-title">
      <div className="x-vault-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="22" height="22">
          <path d="M12 2.8 4.5 5.6v5.6c0 4.5 3.1 8.6 7.5 10 4.4-1.4 7.5-5.5 7.5-10V5.6Z" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path d="M12 8.2v4.6l2.8 1.7" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </div>
      <div className="x-vault-main">
        <p className="x-eyebrow" id="vault-title">
          Guardian Vault · action #{id.toString()}
        </p>
        <p className="x-vault-title">
          {queued
            ? 'This transaction was held in the account’s Guardian Vault instead of executing.'
            : r.vaultCall?.kind === 'cancel' || first?.kind === 'cancelled'
              ? 'This transaction cancelled a queued Guardian Vault action.'
              : 'This transaction executed a queued Guardian Vault action.'}
        </p>
        <KV
          rows={[
            ['Vault account', <Addr address={account} copy={false} />],
            queued?.target ? ['Target', <Addr address={queued.target} copy={false} />] : action ? ['Target', <Addr address={action.target} copy={false} />] : null,
            queued?.value !== undefined ? ['Amount', <Native value={queued.value} digits={6} strong />] : action ? ['Amount', <Native value={action.value} digits={6} strong />] : null,
            executeAfter
              ? ['Executable', <span className="x-num">{formatDate(executeAfter)}{left !== null ? <span className="x-muted"> · {left > 0 ? `in ${formatDuration(left)}` : `${formatDuration(-left)} ago`}</span> : null}</span>]
              : null,
            ['Status', <Pill tone={vaultStatusTone(status)} dot>{vaultStatusText(status)}</Pill>],
          ]}
        />
        {status === 'queued' || status === 'ready' ? (
          <p className="x-small x-muted">
            Until it executes, the owner can cancel it alone and the guardians can cancel it by threshold. If you did not queue this, cancel it from your wallet now.{' '}
            <a className="x-link" href={paths.address(account, 'vault')}>
              See the vault queue
            </a>
          </p>
        ) : null}
      </div>
    </section>
  )
}

function LogView({ log }: { log: DecodedLog }) {
  return (
    <details className="x-log">
      <summary>
        <span className="x-log-idx x-num">{log.logIndex}</span>
        {log.event ? <strong>{log.event}</strong> : <span className="x-muted">Unknown event</span>}
        {log.standard ? <Pill tone="neutral">{standardLabel(log.standard)}</Pill> : null}
        <span className="x-log-from">
          <Addr address={log.address} copy={false} />
        </span>
      </summary>
      <div className="x-log-body">
        {log.signature ? <p className="x-mono x-small x-muted">{log.signature}</p> : null}
        {log.args ? (
          <dl className="x-kv is-compact">
            {Object.entries(log.args).map(([k, v]) => (
              <div className="x-kv-row" key={k}>
                <dt>{k}</dt>
                <dd className="x-mono x-break">{typeof v === 'string' && /^0x[0-9a-fA-F]{40}$/.test(v) ? <Addr address={v} copy={false} /> : displayValue(v)}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <div className="x-mono x-small x-break">
            {log.topics.map((t, i) => (
              <div key={i}>
                <span className="x-muted">topic{i}</span> {t}
              </div>
            ))}
            <div>
              <span className="x-muted">data</span> {log.data}
            </div>
          </div>
        )}
      </div>
    </details>
  )
}

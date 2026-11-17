'use client'
// Lists and safety widgets (ported from the Vite explorer: same rules, new look).

import { CopyButton, Table, type TableColumn } from '@xdevs/ui'
import type { ReactNode } from 'react'
import { toFunctionSelector } from 'viem'
import { config } from '../lib/config'
import { decodeCall } from '../lib/decode'
import { expiryBadge, formatDate, formatDuration, formatToken, formatUnits, shortAddress, spamReasonLabel, vaultStatusTone } from '../lib/format'
import type { HistoryEntry } from '../lib/lookalike'
import { paths } from '../lib/routes'
import type { AddressSummary, Approval, ApprovalChange, BlockSummary, TokenRef, TokenTransfer, TxSummary, VaultAction } from '../lib/types'
import { Addr, Age, BlockNum, Native, Notice, Pill, TxHash } from './bits'

const proposerName = (a: string, names?: Record<string, string>) => names?.[a.toLowerCase()] ?? null

export function BlockTable({ blocks, now, loading, names, empty }: { blocks: BlockSummary[]; now: number; loading?: boolean; names?: Record<string, string>; empty?: ReactNode }) {
  const cols: TableColumn<BlockSummary>[] = [
    { key: 'n', header: 'Block', render: (b) => <BlockNum n={b.number} /> },
    { key: 'age', header: 'Age', render: (b) => <Age ts={b.timestamp} now={now} /> },
    { key: 'txs', header: 'Txs', align: 'right', render: (b) => <span className={`x-num ${b.txCount ? '' : 'x-dim'}`}>{b.txCount}</span> },
    { key: 'proposer', header: 'Proposer', render: (b) => <Addr address={b.miner} name={proposerName(b.miner, names)} copy={false} /> },
    { key: 'gas', header: 'Gas used', align: 'right', render: (b) => <span className="x-num x-muted">{formatUnits(b.gasUsed, 0)}</span> },
    { key: 'final', header: <span className="xh-sr">Finality</span>, align: 'right', render: () => <Pill tone="ok">final</Pill> },
  ]
  return <Table caption="Blocks" columns={cols} rows={blocks} rowKey={(b) => b.hash} loading={loading} skeletonRows={8} empty={empty} className="x-table" />
}

function Method({ t }: { t: Pick<TxSummary, 'to' | 'methodId' | 'contractCreated' | 'value'> & { input?: string } }) {
  if (t.contractCreated || (!t.to && !t.methodId)) return <span className="x-method is-create">Create</span>
  if (!t.methodId) return <span className="x-method">Transfer</span>
  const d = t.input ? decodeCall(t.to, t.input) : null
  return (
    <span className="x-method" title={t.methodId}>
      {d?.name ?? KNOWN_SELECTORS[t.methodId] ?? t.methodId}
    </span>
  )
}

/** Common 4-byte selectors (lists only carry the selector, not the calldata). */
const KNOWN_SELECTORS: Record<string, string> = Object.fromEntries(
  [
    'transfer(address,uint256)',
    'approve(address,uint256)',
    'approve(address,uint256,uint64)',
    'approve(address,address,uint160,uint48)',
    'transferFrom(address,address,uint256)',
    'revokeAll()',
    'deposit()',
    'withdraw(uint256)',
    'delegate(address)',
    'undelegate(address,uint256)',
    'claimRewards(address)',
    'withdrawUnbonded()',
    'register(bytes32,uint16)',
    'register(string,address,uint256)',
    'renew(string,uint256)',
    'setName(string)',
    'execute(address,uint256,bytes)',
    'executeBatch((address,uint256,bytes)[])',
    'cancel(uint256)',
    'execute(uint256)',
    'castVote(uint256,uint8)',
    'propose(address[],uint256[],bytes[],string)',
  ].map((sig) => [toFunctionSelector(sig), sig.slice(0, sig.indexOf('('))]),
)

export function TxTable({ txs, now, perspective, loading, empty, compact }: { txs: TxSummary[]; now: number; perspective?: string; loading?: boolean; empty?: ReactNode; compact?: boolean }) {
  const me = perspective?.toLowerCase()
  const showStatus = txs.some((t) => t.status !== 'unknown')
  const cols: TableColumn<TxSummary>[] = [
    {
      key: 'hash',
      header: 'Transaction',
      render: (t) => (
        <span className="x-tx-cell">
          <TxHash hash={t.hash} />
          {t.spam ? <SpamBadges reasons={t.spamReasons} /> : null}
        </span>
      ),
    },
    { key: 'method', header: 'Method', render: (t) => <Method t={t} /> },
    ...(compact ? [] : [{ key: 'block', header: 'Block', render: (t: TxSummary) => <BlockNum n={t.blockNumber} /> }]),
    { key: 'age', header: 'Age', render: (t) => <Age ts={t.timestamp} now={now} /> },
    { key: 'from', header: 'From', render: (t) => <Addr address={t.from} copy={false} /> },
    {
      key: 'dir',
      header: <span className="xh-sr">Direction</span>,
      align: 'center',
      render: (t) => {
        if (!me) return <span className="x-arrow" aria-hidden="true">→</span>
        const out = t.from.toLowerCase() === me
        return <Pill tone={out ? 'warn' : 'ok'}>{out ? 'out' : 'in'}</Pill>
      },
    },
    { key: 'to', header: 'To', render: (t) => (t.to ? <Addr address={t.to} copy={false} /> : t.contractCreated ? <Addr address={t.contractCreated} copy={false} /> : <span className="x-dim">—</span>) },
    { key: 'value', header: 'Value', align: 'right', render: (t) => (BigInt(t.value) === 0n ? <span className="x-dim x-num">0</span> : <Native value={t.value} digits={4} />) },
    ...(showStatus
      ? [{ key: 'status', header: <span className="xh-sr">Status</span>, align: 'right' as const, render: (t: TxSummary) => (t.status === 'reverted' ? <Pill tone="danger">reverted</Pill> : t.status === 'success' ? <Pill tone="ok">success</Pill> : null) }]
      : []),
  ]
  return <Table caption="Transactions" columns={cols} rows={txs} rowKey={(t) => t.hash} loading={loading} skeletonRows={8} empty={empty} className="x-table" />
}

function TokenName({ token }: { token: Pick<TokenRef, 'address' | 'symbol' | 'name' | 'known'> | null }) {
  if (!token) return <span>all tokens</span>
  if (!token.address) return <span>{config.nativeSymbol}</span>
  return (
    <span className="x-token">
      <a className="x-link" href={paths.token(token.address)}>
        {token.symbol ?? shortAddress(token.address)}
      </a>
      {!token.known ? <Pill tone="neutral">unverified</Pill> : null}
    </span>
  )
}

export function TransferTable({ transfers, now, perspective, loading, empty }: { transfers: TokenTransfer[]; now: number; perspective?: string; loading?: boolean; empty?: ReactNode }) {
  const me = perspective?.toLowerCase()
  const cols: TableColumn<TokenTransfer>[] = [
    {
      key: 'tx',
      header: 'Transaction',
      render: (t) => (
        <span className="x-tx-cell">
          <TxHash hash={t.txHash} />
          <SpamBadges reasons={t.spamReasons} lookAlikeOf={t.lookAlikeOf} />
        </span>
      ),
    },
    { key: 'age', header: 'Age', render: (t) => <Age ts={t.timestamp} now={now} /> },
    { key: 'from', header: 'From', render: (t) => <Addr address={t.from} copy={false} /> },
    {
      key: 'dir',
      header: <span className="xh-sr">Direction</span>,
      align: 'center',
      render: (t) => (me ? <Pill tone={t.from.toLowerCase() === me ? 'warn' : 'ok'}>{t.from.toLowerCase() === me ? 'out' : 'in'}</Pill> : <span className="x-arrow" aria-hidden="true">→</span>),
    },
    {
      key: 'to',
      header: 'To',
      render: (t) => (
        <span>
          <Addr address={t.to} copy={false} />
          {t.lookAlikeOf ? (
            <span className="x-small x-danger-text">
              {' '}
              imitates <Addr address={t.lookAlikeOf} copy={false} />
            </span>
          ) : null}
        </span>
      ),
    },
    { key: 'amount', header: 'Amount', align: 'right', render: (t) => (t.kind === 'native' ? <Native value={t.value} digits={6} /> : <span className="x-num">{formatUnits(t.value, t.token.decimals ?? 18)}</span>) },
    { key: 'token', header: 'Token', render: (t) => (t.kind === 'native' ? <span className="x-muted">{config.nativeSymbol}</span> : <TokenName token={t.token} />) },
  ]
  return (
    <Table
      caption="Transfers"
      columns={cols}
      rows={transfers}
      rowKey={(t) => `${t.txHash}-${t.logIndex ?? 'native'}`}
      loading={loading}
      skeletonRows={6}
      empty={empty}
      className="x-table"
    />
  )
}

const KIND_LABEL = { erc20: 'ERC-20', xdrc20: 'XDRC-20', permit2: 'Permit2Expiring' } as const

export function ApprovalChanges({ items, now }: { items: ApprovalChange[]; now: number }) {
  return (
    <ul className="x-list">
      {items.map((a, i) => {
        const exp = expiryBadge(a.expiry, now)
        return (
          <li key={i}>
            {a.revokesAll ? (
              <span>
                <Pill tone="ok">revoke all</Pill> {a.kind === 'permit2' ? 'Every Permit2Expiring allowance' : <>Every allowance on <TokenName token={a.token} /></>} of{' '}
                <Addr address={a.owner} copy={false} /> was revoked.
              </span>
            ) : (
              <span className="x-approval">
                <Pill tone="info">{KIND_LABEL[a.kind]}</Pill> <Addr address={a.owner} copy={false} /> allowed <Addr address={a.spender} copy={false} /> to spend{' '}
                <strong className="x-num">{a.amount && a.token ? formatToken(a.amount, a.token, a.unlimited) : '—'}</strong>
                {a.token?.address ? <TokenName token={a.token} /> : null}
                {a.unlimited ? <Pill tone="danger">unlimited</Pill> : null}
                {a.amount && BigInt(a.amount) === 0n ? <Pill tone="ok">revoked</Pill> : <Pill tone={exp.tone}>{exp.text}</Pill>}
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}

export function ApprovalTable({ items, now }: { items: Approval[]; now: number }) {
  const cols: TableColumn<Approval>[] = [
    { key: 'token', header: 'Token', render: (a) => (<span className="x-token"><TokenName token={a.token} /> <Pill tone="info">{KIND_LABEL[a.kind]}</Pill></span>) },
    { key: 'spender', header: 'Spender', render: (a) => <Addr address={a.spender} /> },
    {
      key: 'allowance',
      header: 'Allowance',
      align: 'right',
      render: (a) => (
        <span>
          {a.unlimited ? <Pill tone="danger">unlimited</Pill> : <span className="x-num">{formatToken(a.amount, a.token)}</span>}
          {a.remaining !== null && a.remaining !== a.amount ? (
            <span className="x-small x-muted x-block">remaining now: {a.unlimited && BigInt(a.remaining) > 10n ** 70n ? 'unlimited' : formatToken(a.remaining, a.token)}</span>
          ) : null}
        </span>
      ),
    },
    { key: 'expiry', header: 'Expiry', render: (a) => { const e = expiryBadge(a.expiry, now); return <Pill tone={e.tone}>{e.text}</Pill> } },
    {
      key: 'revoke',
      header: 'Revoke',
      render: (a) => (
        <details className="x-revoke">
          <summary>How to revoke</summary>
          <p className="x-small">
            Send a transaction from this address to <span className="x-mono">{shortAddress(a.revoke.to)}</span> calling {a.revoke.description}. Value 0.
          </p>
          <span className="x-calldata">
            <code className="x-mono">{a.revoke.data}</code> <CopyButton value={a.revoke.data} label="Copy calldata" />
          </span>
          <p className="x-small x-muted">The explorer never signs. Use your wallet's custom-transaction or revoke screen.</p>
        </details>
      ),
    },
  ]
  return <Table caption="Live approvals" columns={cols} rows={items} rowKey={(a) => `${a.kind}-${a.token.address}-${a.spender}`} className="x-table" />
}

export function vaultStatusText(s: VaultAction['status']): string {
  return s === 'ready' ? 'ready to execute' : s
}

export function VaultTable({ items, now }: { items: VaultAction[]; now: number }) {
  const cols: TableColumn<VaultAction>[] = [
    { key: 'id', header: '#', render: (v) => <span className="x-num">{BigInt(v.id).toString()}</span> },
    {
      key: 'target',
      header: 'Target',
      render: (v) => (
        <span>
          <Addr address={v.target} copy={false} />
          {v.data !== '0x' ? <span className="x-small x-muted x-mono x-block">calldata {v.data.slice(0, 10)}…</span> : null}
        </span>
      ),
    },
    { key: 'value', header: 'Value', align: 'right', render: (v) => <Native value={v.value} digits={4} /> },
    { key: 'when', header: 'Executable', render: (v) => <span title={formatDate(v.executeAfter)}>{now && v.executeAfter > now ? `in ${formatDuration(v.executeAfter - now)}` : formatDate(v.executeAfter)}</span> },
    {
      key: 'status',
      header: 'Status',
      render: (v) => (
        <span>
          <Pill tone={vaultStatusTone(v.status)}>{vaultStatusText(v.status)}</Pill>
          {v.cancelledBy ? (
            <span className="x-small x-block x-muted">
              by <Addr address={v.cancelledBy} copy={false} />
            </span>
          ) : null}
        </span>
      ),
    },
    { key: 'queued', header: 'Queued in', render: (v) => <TxHash hash={v.queuedTx} /> },
  ]
  return <Table caption="Guardian Vault queue" columns={cols} rows={items} rowKey={(v) => v.id} className="x-table" />
}

export function LookAlikeBanner({ address, matches }: { address: string; matches: HistoryEntry[] }) {
  if (!matches.length) return null
  return (
    <Notice tone="danger" title="Look-alike address warning" role="alert">
      <p>
        This address starts and ends like {matches.length === 1 ? 'an address' : `${matches.length} addresses`} you {matches[0]!.source === 'search' ? 'searched for' : 'viewed'} recently, but
        it is a <strong>different address</strong>. Scammers create look-alikes to poison your history ("address poisoning"). Compare the middle characters before
        sending anything.
      </p>
      <p className="x-mono x-small">This page: {address}</p>
      <ul className="x-list">
        {matches.map((m) => (
          <li key={m.address} className="x-mono x-small">
            Seen {formatDate(Math.floor(m.at / 1000))}: <a className="x-link" href={paths.address(m.address)}>{m.address}</a>
          </li>
        ))}
      </ul>
    </Notice>
  )
}

export function SecurityBadges({ s }: { s: AddressSummary['security'] }) {
  const out: ReactNode[] = []
  if (s.accountKind === 'xdevs-7702') out.push(<Pill key="k" tone="ok" dot>xDEVS smart account (EIP-7702)</Pill>)
  if (s.accountKind === 'xdevs-proxy') out.push(<Pill key="k" tone="ok" dot>xDEVS smart account</Pill>)
  if (s.accountKind === 'unknown') out.push(<Pill key="k" tone="warn" title="Emits account events but its code is not the xDEVS Smart Account">unverified account events</Pill>)
  if (s.vaultEnabled === true) out.push(<Pill key="v" tone="ok">Guardian Vault on</Pill>)
  if (s.vaultEnabled === false) out.push(<Pill key="v" tone="neutral">Guardian Vault off</Pill>)
  if (s.frozen) out.push(<Pill key="f" tone="danger">frozen until {formatDate(s.frozenUntil!)}</Pill>)
  if (s.eoaLock.status === 'locked') out.push(<Pill key="l" tone="ok" title="XIP-0001: the EOA key can only call the account itself">EOA lock: locked{s.eoaLock.active === false ? ' (not enforced yet)' : ''}</Pill>)
  if (s.eoaLock.status === 'sealed') out.push(<Pill key="l" tone="danger" title="XIP-0001: no transaction from this address is valid">EOA lock: sealed</Pill>)
  if (s.eoaKeyRevoked) out.push(<Pill key="r" tone="warn">EOA key revoked</Pill>)
  if (s.recovery) out.push(<Pill key="rc" tone="danger">recovery in progress{s.recovery.readyAt ? ` (ready ${formatDate(s.recovery.readyAt)})` : ''}</Pill>)
  if (s.openVaultActions) out.push(<Pill key="q" tone="warn">{s.openVaultActions} queued vault action{s.openVaultActions > 1 ? 's' : ''}</Pill>)
  if (!out.length) return null
  return <div className="x-badges">{out}</div>
}

export function SpamBadges({ reasons, lookAlikeOf }: { reasons: string[]; lookAlikeOf?: string | null }) {
  if (!reasons.length) return null
  return (
    <span className="x-badges">
      {reasons.map((r) => (
        <Pill key={r} tone={r === 'look-alike' || r === 'impersonating-token' ? 'danger' : 'warn'} title={r === 'look-alike' && lookAlikeOf ? `Imitates ${lookAlikeOf}` : undefined}>
          {spamReasonLabel(r)}
        </Pill>
      ))}
    </span>
  )
}

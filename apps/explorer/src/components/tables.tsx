import type { ReactNode } from 'react'
import type { AddressSummary, Approval, ApprovalChange, BlockSummary, TokenTransfer, TxSummary, VaultAction } from '../lib/types.ts'
import { expiryBadge, formatDate, formatDuration, formatNative, formatToken, formatUnits, shortAddress, timeAgo, vaultStatusTone } from '../lib/format.ts'
import type { HistoryEntry } from '../lib/lookalike.ts'
import { config } from '../lib/config.ts'
import { paths } from '../lib/router.ts'
import { AddressLink, Badge, BlockLink, Copy, FullAddress, Link, SpamBadges, TxLink } from './ui.tsx'

function Table({ caption, head, children }: { caption: string; head: string[]; children: ReactNode }) {
  return (
    <div className="table-wrap">
      <table>
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr>
            {head.map((h) => (
              <th scope="col" key={h}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  )
}

export function BlockTable({ blocks, now }: { blocks: BlockSummary[]; now: number }) {
  return (
    <Table caption="Blocks" head={['Block', 'Age', 'Txs', 'Proposer', 'Burned']}>
      {blocks.map((b) => (
        <tr key={b.hash}>
          <td data-label="Block">
            <BlockLink n={b.number} />
          </td>
          <td data-label="Age" title={formatDate(b.timestamp)}>
            {timeAgo(b.timestamp, now)}
          </td>
          <td data-label="Txs">{b.txCount}</td>
          <td data-label="Proposer">
            <AddressLink address={b.miner} />
          </td>
          <td data-label="Burned">{formatNative(b.fees.burned, config.nativeSymbol, 6)}</td>
        </tr>
      ))}
    </Table>
  )
}

export function TxTable({ txs, now, perspective }: { txs: TxSummary[]; now: number; perspective?: string }) {
  const me = perspective?.toLowerCase()
  return (
    <Table caption="Transactions" head={['Hash', 'Age', 'From', '', 'To', 'Value', 'Status']}>
      {txs.map((t) => {
        const dir = me ? (t.from.toLowerCase() === me ? 'OUT' : 'IN') : null
        return (
          <tr key={t.hash} className={t.spam ? 'row-spam' : undefined}>
            <td data-label="Hash">
              <TxLink hash={t.hash} />
              {t.methodId ? <span className="muted small mono"> {t.methodId}</span> : null}
              <SpamBadges reasons={t.spam ? t.spamReasons : []} />
            </td>
            <td data-label="Age" title={formatDate(t.timestamp)}>
              {timeAgo(t.timestamp, now)}
            </td>
            <td data-label="From">
              <AddressLink address={t.from} />
            </td>
            <td data-label="">{dir ? <Badge tone={dir === 'OUT' ? 'warn' : 'ok'}>{dir}</Badge> : <span aria-hidden="true">→</span>}</td>
            <td data-label="To">
              {t.to ? <AddressLink address={t.to} /> : t.contractCreated ? (
                <span>
                  <Badge tone="info">Create</Badge> <AddressLink address={t.contractCreated} />
                </span>
              ) : (
                '—'
              )}
            </td>
            <td data-label="Value">{formatNative(t.value, config.nativeSymbol, 4)}</td>
            <td data-label="Status">{t.status === 'success' ? <Badge tone="ok">Success</Badge> : <Badge tone="danger">Reverted</Badge>}</td>
          </tr>
        )
      })}
    </Table>
  )
}

export function TransferTable({ transfers, now, perspective }: { transfers: TokenTransfer[]; now: number; perspective?: string }) {
  const me = perspective?.toLowerCase()
  return (
    <Table caption="Token transfers" head={['Tx', 'Age', 'From', '', 'To', 'Amount', 'Token']}>
      {transfers.map((t) => {
        const dir = me ? (t.from.toLowerCase() === me ? 'OUT' : 'IN') : null
        return (
          <tr key={`${t.txHash}-${t.logIndex ?? 'native'}`} className={t.spam ? 'row-spam' : undefined}>
            <td data-label="Tx">
              <TxLink hash={t.txHash} />
              <SpamBadges reasons={t.spamReasons} lookAlikeOf={t.lookAlikeOf} />
              {t.lookAlikeOf ? (
                <span className="small danger-text">
                  {' '}
                  imitates <AddressLink address={t.lookAlikeOf} />
                </span>
              ) : null}
            </td>
            <td data-label="Age" title={formatDate(t.timestamp)}>
              {timeAgo(t.timestamp, now)}
            </td>
            <td data-label="From">
              <AddressLink address={t.from} />
            </td>
            <td data-label="">{dir ? <Badge tone={dir === 'OUT' ? 'warn' : 'ok'}>{dir}</Badge> : <span aria-hidden="true">→</span>}</td>
            <td data-label="To">
              <AddressLink address={t.to} />
            </td>
            <td data-label="Amount" className="num">
              {t.kind === 'native' ? formatNative(t.value, config.nativeSymbol) : formatUnits(t.value, t.token.decimals ?? 18)}
            </td>
            <td data-label="Token">
              {t.kind === 'native' ? (
                config.nativeSymbol
              ) : (
                <span>
                  <Link to={paths.address(t.token.address!)}>{t.token.symbol ?? shortAddress(t.token.address)}</Link>{' '}
                  {!t.token.known ? <Badge tone="neutral">Unverified</Badge> : null}
                </span>
              )}
            </td>
          </tr>
        )
      })}
    </Table>
  )
}

function TokenName({ token }: { token: { address: string | null; symbol: string | null; name: string | null; known: boolean } | null }) {
  if (!token) return <span>all tokens</span>
  if (!token.address) return <span>{config.nativeSymbol}</span>
  return (
    <span>
      <Link to={paths.address(token.address)}>{token.symbol ?? shortAddress(token.address)}</Link>
      {token.name ? <span className="muted small"> {token.name}</span> : null} {!token.known ? <Badge tone="neutral">Unverified</Badge> : null}
    </span>
  )
}

const KIND_LABEL = { erc20: 'ERC-20', xdrc20: 'XDRC-20', permit2: 'Permit2Expiring' } as const

export function ApprovalChanges({ items, now }: { items: ApprovalChange[]; now: number }) {
  return (
    <ul className="list">
      {items.map((a, i) => {
        const exp = expiryBadge(a.expiry, now)
        return (
          <li key={i}>
            {a.revokesAll ? (
              <span>
                <Badge tone="ok">Revoke all</Badge> {a.kind === 'permit2' ? 'Every Permit2Expiring allowance' : <>Every allowance on <TokenName token={a.token} /></>} of{' '}
                <AddressLink address={a.owner} /> was revoked.
              </span>
            ) : (
              <span>
                <Badge tone="info">{KIND_LABEL[a.kind]}</Badge> <AddressLink address={a.owner} /> allowed <AddressLink address={a.spender} /> to spend{' '}
                <strong>{a.amount && a.token ? formatToken(a.amount, a.token, a.unlimited) : '—'}</strong> {a.token ? <TokenName token={a.token} /> : null}{' '}
                {a.unlimited ? <Badge tone="danger">Unlimited</Badge> : null}{' '}
                {a.amount && BigInt(a.amount) === 0n ? <Badge tone="ok">Revoked</Badge> : <Badge tone={exp.tone}>{exp.text}</Badge>}
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}

export function ApprovalTable({ items, now }: { items: Approval[]; now: number }) {
  return (
    <Table caption="Live approvals" head={['Token', 'Spender', 'Allowance', 'Expiry', 'Revoke']}>
      {items.map((a) => {
        const exp = expiryBadge(a.expiry, now)
        return (
          <tr key={`${a.kind}-${a.token.address}-${a.spender}`}>
            <td data-label="Token">
              <TokenName token={a.token} /> <Badge tone="info">{KIND_LABEL[a.kind]}</Badge>
            </td>
            <td data-label="Spender">
              <AddressLink address={a.spender} />
            </td>
            <td data-label="Allowance">
              {a.unlimited ? <Badge tone="danger">Unlimited</Badge> : formatToken(a.amount, a.token)}
              {a.remaining !== null && a.remaining !== a.amount ? (
                <div className="small muted">remaining now: {a.unlimited && BigInt(a.remaining) > 10n ** 70n ? 'unlimited' : formatToken(a.remaining, a.token)}</div>
              ) : null}
            </td>
            <td data-label="Expiry">
              <Badge tone={exp.tone}>{exp.text}</Badge>
            </td>
            <td data-label="Revoke">
              <details className="revoke">
                <summary>How to revoke</summary>
                <p className="small">
                  Send a transaction from this address to <span className="mono">{a.revoke.to}</span> calling {a.revoke.description}. Value 0. Calldata:
                </p>
                <code className="mono break">{a.revoke.data}</code> <Copy text={a.revoke.data} label="Copy calldata" />
                <p className="small muted">The explorer never signs. Use your wallet's custom-transaction or revoke screen.</p>
              </details>
            </td>
          </tr>
        )
      })}
    </Table>
  )
}

export function VaultTable({ items, now }: { items: VaultAction[]; now: number }) {
  return (
    <Table caption="Guardian Vault queue" head={['#', 'Target', 'Value', 'Executable', 'Status', 'Queued in']}>
      {items.map((v) => (
        <tr key={v.id}>
          <td data-label="#">{BigInt(v.id).toString()}</td>
          <td data-label="Target">
            <AddressLink address={v.target} />
            {v.data !== '0x' ? <div className="small muted mono">calldata {v.data.slice(0, 10)}…</div> : null}
          </td>
          <td data-label="Value">{formatNative(v.value, config.nativeSymbol)}</td>
          <td data-label="Executable" title={formatDate(v.executeAfter)}>
            {v.executeAfter > now ? `in ${formatDuration(v.executeAfter - now)}` : formatDate(v.executeAfter)}
          </td>
          <td data-label="Status">
            <Badge tone={vaultStatusTone(v.status)}>{v.status === 'ready' ? 'Ready to execute' : v.status[0]!.toUpperCase() + v.status.slice(1)}</Badge>
            {v.cancelledBy ? (
              <div className="small">
                by <AddressLink address={v.cancelledBy} />
              </div>
            ) : null}
          </td>
          <td data-label="Queued in">
            <TxLink hash={v.queuedTx} />
          </td>
        </tr>
      ))}
    </Table>
  )
}

export function LookAlikeBanner({ address, matches }: { address: string; matches: HistoryEntry[] }) {
  if (!matches.length) return null
  return (
    <div className="notice notice-danger lookalike" role="alert">
      <h2>Look-alike address warning</h2>
      <p>
        This address starts and ends like {matches.length === 1 ? 'an address' : `${matches.length} addresses`} you{' '}
        {matches[0]!.source === 'search' ? 'searched for' : 'viewed'} recently, but it is a <strong>different address</strong>. Scammers create
        look-alikes to poison your history ("address poisoning"). Compare the middle characters before sending anything.
      </p>
      <p className="mono small">
        This page: <FullAddress address={address} />
      </p>
      <ul>
        {matches.map((m) => (
          <li key={m.address} className="mono small">
            Seen {formatDate(Math.floor(m.at / 1000))}: <Link to={paths.address(m.address)}>{m.address}</Link>
          </li>
        ))}
      </ul>
    </div>
  )
}

export function SecurityBadges({ s }: { s: AddressSummary['security'] }) {
  const out: ReactNode[] = []
  if (s.accountKind === 'xdevs-7702') out.push(<Badge key="k" tone="info">xDEVS smart account (EIP-7702)</Badge>)
  if (s.accountKind === 'xdevs-proxy') out.push(<Badge key="k" tone="info">xDEVS smart account</Badge>)
  if (s.accountKind === 'unknown') out.push(<Badge key="k" tone="warn" title="Emits account events but its code is not the xDEVS Smart Account">Unverified account events</Badge>)
  if (s.vaultEnabled === true) out.push(<Badge key="v" tone="ok">Guardian Vault on</Badge>)
  if (s.vaultEnabled === false) out.push(<Badge key="v" tone="neutral">Guardian Vault off</Badge>)
  if (s.frozen) out.push(<Badge key="f" tone="danger">Frozen until {formatDate(s.frozenUntil!)}</Badge>)
  if (s.eoaLock.status === 'locked') out.push(<Badge key="l" tone="ok" title="XIP-0001: the EOA key can only call the account itself">EOA lock: locked{s.eoaLock.active === false ? ' (not enforced yet)' : ''}</Badge>)
  if (s.eoaLock.status === 'sealed') out.push(<Badge key="l" tone="danger" title="XIP-0001: no transaction from this address is valid">EOA lock: sealed</Badge>)
  if (s.eoaKeyRevoked) out.push(<Badge key="r" tone="warn">EOA key revoked</Badge>)
  if (s.recovery) out.push(<Badge key="rc" tone="danger">Recovery in progress{s.recovery.readyAt ? ` (ready ${formatDate(s.recovery.readyAt)})` : ''}</Badge>)
  if (s.openVaultActions) out.push(<Badge key="q" tone="warn">{s.openVaultActions} queued vault action{s.openVaultActions > 1 ? 's' : ''}</Badge>)
  if (!out.length) return null
  return <div className="badges">{out}</div>
}

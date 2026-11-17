'use client'
// Overview: balance, quick actions, assets, activity, and the account's protection at a glance.
import { AddressChip, Badge, Button, Skeleton, cx } from '@xdevs/ui'
import Link from 'next/link'
import { useState } from 'react'
import { IconChevron, IconClock, IconDrop, IconKey, IconReceive, IconSend, IconShield, IconTag, IconUsers, IconX } from '../components/icons.tsx'
import { OpProgress } from '../components/review.tsx'
import { Avatar, Countdown, DemoTag, Meter, Notice, SectionTitle, TokenGlyph, useNow } from '../components/ui.tsx'
import { config, explorerAddress, explorerTx, faucetLink } from '../config.ts'
import type { ActivityItem } from '../data/types.ts'
import { fmtAmount, fmtDuration, fmtRelative, shortAddr } from '../lib/format.ts'
import { useLoad } from '../wallet/hooks.ts'
import { useOp, useWallet } from '../wallet/WalletProvider.tsx'

export function Dashboard() {
  const w = useWallet()
  const { rec, state, queue, primaryName } = w
  const assets = useLoad(rec && state ? () => w.backend.assets(rec.address, state.balance) : null, [rec?.address, state?.balance, w.backend])
  const activity = useLoad(rec ? () => w.backend.activity(rec.address) : null, [rec?.address, state?.balance, queue.length, w.backend])
  const approvals = useLoad(rec && state?.deployed ? () => w.backend.approvals(rec.address) : null, [rec?.address, state?.deployed, w.backend])
  const activate = useOp()
  const now = useNow(30_000)
  if (!rec) return null
  const open = queue.filter((a) => a.status === 'queued' || a.status === 'ready')
  const needsDeploy = !!state && !state.deployed && !!rec.params
  const p = state?.policy
  const unlimited = approvals.data?.filter((a) => a.unlimited).length ?? 0

  return (
    <div className="xu-page w-page">
      <section className="w-balance">
        <div className="w-balance-main">
          <div className="w-balance-who">
            <Avatar address={rec.address} size={44} />
            <div>
              <p className="w-balance-name">
                {primaryName ?? rec.label} {w.demo ? <DemoTag /> : null}
                {rec.kind === 'watch' ? <Badge tone="dim">watch-only</Badge> : null}
                {rec.kind === 'virtual' ? <Badge tone="warn">dev passkey</Badge> : null}
              </p>
              <AddressChip address={rec.address} href={explorerAddress(rec.address)} />
            </div>
          </div>
          <p className="w-label">Balance</p>
          <p className="w-balance-amount">
            {state ? (
              <>
                {fmtAmount(state.balance, 18, 4)} <span>{config.symbol}</span>
              </>
            ) : (
              <Skeleton width={260} height={52} />
            )}
          </p>
          <p className="w-balance-sub">{config.chainName} · test tokens have no value</p>
          <div className="w-actions">
            <Link href="/send" className="w-action">
              <span>
                <IconSend size={18} />
              </span>
              Send
            </Link>
            <Link href="/receive" className="w-action">
              <span>
                <IconReceive size={18} />
              </span>
              Receive
            </Link>
            <a href={faucetLink(rec.address)} target="_blank" rel="noopener noreferrer" className="w-action">
              <span>
                <IconDrop size={18} />
              </span>
              Get {config.symbol}
            </a>
            <Link href="/names" className="w-action">
              <span>
                <IconTag size={18} />
              </span>
              Name
            </Link>
          </div>
        </div>
        <div className="w-balance-side">
          <ProtectionCard />
        </div>
      </section>

      {open.length ? (
        <Notice
          tone="warn"
          icon={<IconClock />}
          title={`${open.length === 1 ? '1 action is' : `${open.length} actions are`} waiting in your Guardian Vault`}
          action={
            <Button size="sm" variant="ghost" href="/vault" linkComponent={Link}>
              Review
            </Button>
          }
        >
          The next one executes in <Countdown until={Math.min(...open.map((a) => Number(a.executeAfter)))} />. If you did not start it, cancel it now: nothing has left your account.
        </Notice>
      ) : null}

      {state?.frozen ? (
        <Notice tone="alert" title="Your guardians froze this account">
          Nothing can be signed or executed until they unfreeze it, a recovery completes, or the freeze ends in{' '}
          <Countdown until={Number(state.frozenUntil)} />.
        </Notice>
      ) : null}

      {needsDeploy ? (
        <section className="w-activate">
          <div>
            <h2>Activate your account</h2>
            <p>
              Your address already receives {config.symbol}. The account contract is created by your first transaction, signed with your passkey
              {config.paymasterSponsor ? ' — gas for it is sponsored.' : '. Fund it from the faucet first to pay the gas.'}
            </p>
          </div>
          {activate.phase === 'idle' || activate.phase === 'error' ? (
            <Button
              disabled={rec.kind === 'watch' || (!config.paymasterSponsor && (state?.balance ?? 0n) === 0n)}
              onClick={() => void activate.run({ callData: '0x', sponsored: !!config.paymasterSponsor, describe: { kind: 'account', title: 'Account activated' } })}
            >
              Activate with passkey
            </Button>
          ) : null}
          <OpProgress phase={activate.phase} status={activate.status} error={activate.error} doneText="Your smart account is live." />
        </section>
      ) : null}

      <div className="w-dash-grid">
        <section className="w-panel">
          <SectionTitle>Assets</SectionTitle>
          {assets.data ? (
            <ul className="w-assets">
              {assets.data.map((a) => (
                <li key={a.token ?? 'native'}>
                  <TokenGlyph symbol={a.symbol} />
                  <div className="w-asset-name">
                    <b>{a.symbol}</b>
                    <span>{a.name}</span>
                  </div>
                  <div className="w-asset-bal xh-num">{fmtAmount(a.balance, a.decimals, a.decimals > 6 ? 4 : 2)}</div>
                </li>
              ))}
            </ul>
          ) : (
            <Skeleton lines={3} height={36} />
          )}
          {!config.knownTokens.length && !w.demo ? <p className="w-fine">Tokens appear here when the deployment lists them (NEXT_PUBLIC_KNOWN_TOKENS).</p> : null}
        </section>

        <section className="w-panel">
          <SectionTitle>Activity</SectionTitle>
          {activity.data ? <ActivityList items={activity.data} now={now} /> : <Skeleton lines={4} height={40} />}
        </section>
      </div>

      {p && state?.deployed ? (
        <div className="w-dash-foot">
          <Link href="/approvals" className="w-tile">
            <IconKey />
            <div>
              <b>{approvals.data ? `${approvals.data.length} live approval${approvals.data.length === 1 ? '' : 's'}` : 'Approvals'}</b>
              <span>{unlimited ? `${unlimited} unlimited: review` : 'Every approval expires'}</span>
            </div>
            <IconChevron />
          </Link>
          <Link href="/guardians" className="w-tile">
            <IconUsers />
            <div>
              <b>{state.guardians.length ? `${state.guardianThreshold} of ${state.guardians.length} guardians` : 'No guardians'}</b>
              <span>{state.guardians.length ? 'Recovery and freeze are set up' : 'Add guardians for recovery'}</span>
            </div>
            <IconChevron />
          </Link>
          <Link href="/eoa-lock" className="w-tile">
            <IconShield />
            <div>
              <b>Using MetaMask?</b>
              <span>Protect an existing address with the EOA lock</span>
            </div>
            <IconChevron />
          </Link>
        </div>
      ) : null}
    </div>
  )
}

function ProtectionCard() {
  const { state, rec } = useWallet()
  if (!state || !rec) return <Skeleton height={220} radius={14} />
  const p = state.deployed ? state.policy : (rec.params?.policy ?? state.policy)
  const guardians = state.deployed ? state.guardians.length : (rec.params?.guardians.length ?? 0)
  const quorum = state.deployed ? state.guardianThreshold : (rec.params?.guardianThreshold ?? 0)
  const spent = Number(state.spentToday / 10n ** 14n) / 1e4
  const daily = Number(p.dailyLimit / 10n ** 14n) / 1e4
  return (
    <div className="w-protect">
      <div className="w-protect-head">
        <span className={cx('w-protect-badge', p.enabled ? 'is-on' : 'is-off')}>
          <IconShield size={16} />
        </span>
        <div>
          <p className="w-protect-title">{p.enabled ? 'Guardian Vault is on' : 'Guardian Vault is off'}</p>
          <p className="w-protect-sub">{p.enabled ? `Larger actions wait ${fmtDuration(Number(p.delay))}` : 'Everything executes instantly'}</p>
        </div>
      </div>
      {p.enabled ? (
        <>
          <div className="w-protect-row">
            <span>Instant per transfer</span>
            <b className="xh-num">
              {fmtAmount(p.nativeThreshold)} {config.symbol}
            </b>
          </div>
          <div className="w-protect-row">
            <span>Spent today</span>
            <b className="xh-num">
              {fmtAmount(state.spentToday)} / {fmtAmount(p.dailyLimit)}
            </b>
          </div>
          <Meter value={spent} max={daily} tone={spent / Math.max(daily, 1e-9) > 0.8 ? 'warn' : 'final'} label="Instant daily limit used" />
        </>
      ) : null}
      <div className="w-protect-row">
        <span>Guardians</span>
        <b>{guardians ? `${quorum} of ${guardians}` : 'none'}</b>
      </div>
      <div className="w-protect-row">
        <span>Signer</span>
        <b>{rec.kind === 'passkey' ? 'Passkey on this device' : rec.kind === 'virtual' ? 'Virtual passkey (dev)' : rec.kind === 'watch' ? 'None (watch-only)' : 'Raw key (dev)'}</b>
      </div>
      <Link href="/vault" className="w-protect-link">
        Vault settings <IconChevron size={14} />
      </Link>
    </div>
  )
}

export function ActivityList({ items, now }: { items: ActivityItem[]; now: number }) {
  const [showSpam, setShowSpam] = useState(false)
  const spam = items.filter((i) => i.spam)
  const shown = items.filter((i) => showSpam || !i.spam)
  if (!items.length)
    return (
      <div className="w-empty">
        <p>No activity yet.</p>
        <p className="w-fine">Transfers and vault actions appear here.</p>
      </div>
    )
  return (
    <>
      <ul className="w-activity">
        {shown.map((a) => {
          const out = a.kind === 'sent' || a.kind === 'queued' || a.kind === 'cancelled'
          const icon = a.kind === 'received' ? <IconReceive /> : a.kind === 'queued' ? <IconClock /> : a.kind === 'cancelled' ? <IconX /> : a.kind === 'sent' ? <IconSend /> : <IconShield />
          return (
            <li key={a.id} className={cx(a.spam && 'is-spam', `k-${a.kind}`)}>
              <span className="w-activity-icon">{icon}</span>
              <div className="w-activity-main">
                <b>{a.title}</b>
                <span>
                  {a.counterparty ? (a.counterpartyName ?? shortAddr(a.counterparty)) : null}
                  {a.counterparty ? ' · ' : null}
                  {fmtRelative(a.timestamp, now)}
                  {a.lookAlikeOf ? <em className="w-flag">look-alike address</em> : null}
                </span>
              </div>
              <div className="w-activity-amt">
                {a.amount !== undefined ? (
                  <b className={cx('xh-num', a.kind === 'received' && 'is-in', a.status === 'cancelled' && 'is-struck')}>
                    {a.kind === 'received' ? '+' : out ? '−' : ''}
                    {fmtAmount(a.amount, a.decimals ?? 18, 2)} {a.symbol}
                  </b>
                ) : null}
                {a.status === 'queued' ? (
                  <Badge tone="warn">in vault</Badge>
                ) : a.status === 'cancelled' ? (
                  <Badge tone="dim">cancelled</Badge>
                ) : a.status === 'failed' ? (
                  <Badge tone="alert">failed</Badge>
                ) : a.hash ? (
                  <a className="w-activity-link" href={explorerTx(a.hash)} target="_blank" rel="noopener noreferrer">
                    details
                  </a>
                ) : null}
              </div>
            </li>
          )
        })}
      </ul>
      {spam.length ? (
        <button type="button" className="w-disclose" aria-expanded={showSpam} onClick={() => setShowSpam((v) => !v)}>
          {showSpam ? 'Hide' : 'Show'} {spam.length} hidden spam transfer{spam.length === 1 ? '' : 's'} from look-alike addresses
        </button>
      ) : null}
    </>
  )
}

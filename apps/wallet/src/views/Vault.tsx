'use client'
// Guardian Vault: queued actions with countdown and cancel, history, policy and trusted addresses.
import { AddressChip, Badge, Button, cx } from '@xdevs/ui'
import { useEffect, useState } from 'react'
import { type Address, formatEther, getAddress, isAddress } from 'viem'
import { IconClock, IconShield } from '../components/icons.tsx'
import { OpProgress } from '../components/review.tsx'
import { ConfirmDialog, DemoTag, Field, KV, Meter, Notice, Page, SectionTitle, Toggle, useNow } from '../components/ui.tsx'
import { config, explorerAddress } from '../config.ts'
import { MAX_VAULT_DELAY, MIN_VAULT_DELAY } from '../lib/account.ts'
import { describeCall } from '../lib/describe.ts'
import { fmtAmount, fmtCountdown, fmtDate, fmtDuration, parseAmount } from '../lib/format.ts'
import { type QueuedAction, cancelCallData, executeQueuedCallData, setVaultPolicyCallData, setWhitelistedCallData } from '../lib/vault.ts'
import { useDescriber } from '../wallet/describer.ts'
import { useLoad } from '../wallet/hooks.ts'
import { useOp, useWallet } from '../wallet/WalletProvider.tsx'

export function Vault() {
  const w = useWallet()
  const { rec, state, queue } = w
  const now = useNow()
  const assets = useLoad(rec && state ? () => w.backend.assets(rec.address, state.balance) : null, [rec?.address, w.backend])
  const describer = useDescriber(assets.data)
  const op = useOp()
  const [confirm, setConfirm] = useState<QueuedAction | null>(null)
  const [editing, setEditing] = useState(false)
  const [form, setForm] = useState({ enabled: true, threshold: '', daily: '', delayHours: '', blockUnlimited: true, instantSigs: false })
  const [formErr, setFormErr] = useState<string | null>(null)
  const [trust, setTrust] = useState('')
  const p = state?.policy

  useEffect(() => {
    if (!p) return
    setForm({
      enabled: p.enabled,
      threshold: formatEther(p.nativeThreshold),
      daily: formatEther(p.dailyLimit),
      delayHours: String(Number(p.delay || 86_400n) / 3600),
      blockUnlimited: p.blockUnlimitedApprovals,
      instantSigs: p.allowInstantSignatures,
    })
  }, [p?.enabled, p?.nativeThreshold, p?.dailyLimit, p?.delay, p?.blockUnlimitedApprovals, p?.allowInstantSignatures])

  if (!rec) return null
  if (!state)
    return (
      <Page title="Guardian Vault">
        <div className="w-panel">Loading…</div>
      </Page>
    )
  if (!state.deployed)
    return (
      <Page title="Guardian Vault" sub="Large transfers and risky actions wait out a delay you can cancel.">
        <Notice tone="info" title="Activate your account first">
          The vault settings you chose at sign-up apply from your first transaction (Overview → Activate).
        </Notice>
      </Page>
    )

  const readOnly = rec.kind === 'watch'
  const open = queue.filter((a) => a.status === 'queued' || a.status === 'ready')
  const past = queue.filter((a) => a.status !== 'queued' && a.status !== 'ready')
  const delay = Number(state.policy.delay)
  const delayedNote = state.policy.enabled
    ? `Changes wait ${fmtDuration(delay)} in the vault, cancellable, so a thief with your device cannot switch protection off quickly.`
    : 'The vault is off, so changes apply immediately.'

  function submitPolicy() {
    setFormErr(null)
    try {
      const d = BigInt(Math.round(Number(form.delayHours) * 3600))
      if (form.enabled && (d < MIN_VAULT_DELAY || d > MAX_VAULT_DELAY)) throw new Error('The delay must be between 1 hour and 7 days.')
      void op.run({
        callData: setVaultPolicyCallData({
          enabled: form.enabled,
          blockUnlimitedApprovals: form.blockUnlimited,
          allowInstantSignatures: form.instantSigs,
          delay: d,
          nativeThreshold: parseAmount(form.threshold || '0'),
          dailyLimit: parseAmount(form.daily || '0'),
          dailyGasBudget: p?.dailyGasBudget || 10n ** 18n,
        }),
        describe: { kind: 'settings', title: form.enabled ? 'Vault policy change' : 'Turn the Guardian Vault off' },
      })
      setEditing(false)
    } catch (e) {
      setFormErr((e as Error).message)
    }
  }

  return (
    <Page
      title={<>Guardian Vault {w.demo ? <DemoTag /> : null}</>}
      sub={state.policy.enabled ? `Large transfers and risky actions wait ${fmtDuration(delay)}. You, or a quorum of your guardians, can cancel them meanwhile.` : 'The vault is off: everything executes instantly.'}
    >
      {state.frozen ? (
        <Notice tone="alert" title="Frozen by your guardians">
          While frozen, only the guardian quorum can cancel queued actions. The freeze ends {fmtDate(Number(state.frozenUntil))}.
        </Notice>
      ) : null}
      <OpProgress phase={op.phase} status={op.status} error={op.error} queued={!!op.result?.queuedIds.length} doneText={op.result?.queuedIds.length ? `Queued as action #${op.result.queuedIds[0]}.` : 'Done.'}>
        {op.phase !== 'running' ? (
          <Button size="sm" variant="ghost" onClick={op.reset}>
            Close
          </Button>
        ) : null}
      </OpProgress>

      <SectionTitle>
        Waiting <span className="w-count">{open.length}</span>
      </SectionTitle>
      {open.length === 0 ? (
        <div className="w-panel w-empty">
          <IconShield size={20} />
          <p>Nothing is waiting in your vault.</p>
          <p className="w-fine">Transfers above your instant limit, approvals and setting changes appear here before they execute.</p>
        </div>
      ) : (
        <ul className="w-queue">
          {open.map((a) => {
            const ready = a.status === 'ready' || Number(a.executeAfter) <= now
            const total = delay || 1
            const left = Math.max(0, Number(a.executeAfter) - now)
            const elapsed = Math.min(total, total - left)
            return (
              <li key={String(a.id)} className={cx('w-qitem', ready && 'is-ready')}>
                <div className="w-qitem-top">
                  <span className="w-qitem-id">#{String(a.id)}</span>
                  <div className="w-qitem-what">
                    {a.calls.map((c, i) => (
                      <p key={i}>{describeCall(rec.address, c, describer)}</p>
                    ))}
                  </div>
                  <div className="w-qitem-when">
                    {ready ? (
                      <Badge tone="alert" dot>
                        executable now
                      </Badge>
                    ) : (
                      <>
                        <span className="w-label">executes in</span>
                        <b className="xh-num">{fmtCountdown(left)}</b>
                      </>
                    )}
                  </div>
                </div>
                <Meter value={elapsed} max={total} tone={ready ? 'alert' : 'warn'} label={`Time elapsed for action ${a.id}`} />
                <div className="w-qitem-foot">
                  <span className="w-fine">
                    <IconClock size={13} /> {ready ? 'The delay is over: anyone can execute it now.' : `Executes ${fmtDate(Number(a.executeAfter))}`}
                    {state.guardians.length ? ` · guardian cancel votes ${a.cancelVotes} of ${state.guardianThreshold}` : ''}
                  </span>
                  {!readOnly ? (
                    <div className="w-qitem-actions">
                      {ready ? (
                        <Button size="sm" variant="ghost" disabled={state.frozen || op.phase === 'running'} onClick={() => void op.run({ callData: executeQueuedCallData(a.id), describe: { kind: 'settings', title: `Executed vault action #${a.id}` } })}>
                          Execute
                        </Button>
                      ) : null}
                      <Button size="sm" variant="danger" disabled={state.frozen || op.phase === 'running'} onClick={() => setConfirm(a)}>
                        Cancel
                      </Button>
                    </div>
                  ) : null}
                </div>
              </li>
            )
          })}
        </ul>
      )}

      <div className="w-dash-grid">
        <section className="w-panel">
          <SectionTitle
            action={
              !readOnly && !editing ? (
                <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
                  Change
                </Button>
              ) : null
            }
          >
            Policy
          </SectionTitle>
          {!editing ? (
            <KV
              rows={[
                ['Status', state.policy.enabled ? <Badge tone="final" dot>on</Badge> : <Badge tone="alert" dot>off</Badge>],
                ['Instant per transfer', `${fmtAmount(state.policy.nativeThreshold)} ${config.symbol}`],
                ['Instant per day', `${fmtAmount(state.policy.dailyLimit)} ${config.symbol} (${fmtAmount(state.spentToday)} used today)`],
                ['Delay for everything else', fmtDuration(delay)],
                ['Unlimited approvals', state.policy.blockUnlimitedApprovals ? 'blocked' : 'wait in the vault'],
                ['Off-chain signatures', state.policy.allowInstantSignatures ? 'instant (permits possible)' : 'only pre-approved'],
              ]}
            />
          ) : (
            <div className="w-form">
              <Notice tone="info">{delayedNote} Turning the vault on is always instant.</Notice>
              <Toggle checked={form.enabled} onChange={(v) => setForm({ ...form, enabled: v })} label="Guardian Vault" hint={form.enabled ? 'On' : 'Off: every transaction would execute instantly'} />
              <div className="w-grid2">
                <Field label={`Instant per transfer (${config.symbol})`} htmlFor="p-thr">
                  <input id="p-thr" className="w-input" inputMode="decimal" value={form.threshold} onChange={(e) => setForm({ ...form, threshold: e.target.value })} />
                </Field>
                <Field label={`Instant per day (${config.symbol})`} htmlFor="p-day">
                  <input id="p-day" className="w-input" inputMode="decimal" value={form.daily} onChange={(e) => setForm({ ...form, daily: e.target.value })} />
                </Field>
                <Field label="Delay (hours, 1–168)" htmlFor="p-delay">
                  <input id="p-delay" className="w-input" inputMode="numeric" value={form.delayHours} onChange={(e) => setForm({ ...form, delayHours: e.target.value })} />
                </Field>
              </div>
              <Toggle checked={form.blockUnlimited} onChange={(v) => setForm({ ...form, blockUnlimited: v })} label="Block unlimited approvals" />
              <Toggle checked={form.instantSigs} onChange={(v) => setForm({ ...form, instantSigs: v })} label="Allow instant off-chain signatures" hint="Not recommended: lets signed permits bypass the vault." />
              {formErr ? <Notice tone="alert">{formErr}</Notice> : null}
              <div className="w-flow-actions is-left">
                <Button onClick={submitPolicy} disabled={op.phase === 'running'}>
                  Submit with passkey
                </Button>
                <Button variant="quiet" onClick={() => setEditing(false)}>
                  Cancel
                </Button>
              </div>
            </div>
          )}
        </section>

        <section className="w-panel">
          <SectionTitle>Trusted addresses</SectionTitle>
          <p className="w-fine">Transfers, approvals and calls to these execute instantly, with any amount. Trust is powerful: keep this list short.</p>
          {state.whitelist.length === 0 ? <p className="w-muted">None.</p> : null}
          <ul className="w-plain-list">
            {state.whitelist.map((a) => (
              <li key={a}>
                <span>{describer.label?.(a) ?? null}</span>
                <AddressChip address={a} href={explorerAddress(a)} />
                {!readOnly ? (
                  <Button size="sm" variant="quiet" onClick={() => void op.run({ callData: setWhitelistedCallData(a, false), describe: { kind: 'settings', title: 'Stop trusting an address' } })}>
                    Remove
                  </Button>
                ) : null}
              </li>
            ))}
          </ul>
          {!readOnly ? (
            <div className="w-inline-form">
              <input className="w-input is-mono" value={trust} onChange={(e) => setTrust(e.target.value)} placeholder="0x… address to trust" aria-label="Address to trust" spellCheck={false} />
              <Button
                size="sm"
                variant="ghost"
                disabled={!isAddress(trust.trim(), { strict: false })}
                onClick={() => void op.run({ callData: setWhitelistedCallData(getAddress(trust.trim()) as Address, true), describe: { kind: 'settings', title: 'Trust an address' } })}
              >
                Add
              </Button>
            </div>
          ) : null}
          <p className="w-fine">{delayedNote}</p>
        </section>
      </div>

      {past.length ? (
        <>
          <SectionTitle>History</SectionTitle>
          <ul className="w-history">
            {past.map((a) => (
              <li key={String(a.id)}>
                <span className="w-qitem-id">#{String(a.id)}</span>
                <span>{a.calls.map((c) => describeCall(rec.address, c, describer)).join(' · ')}</span>
                <Badge tone={a.status === 'executed' ? 'final' : a.status === 'cancelled' ? 'dim' : 'warn'}>{a.status}</Badge>
              </li>
            ))}
          </ul>
        </>
      ) : null}

      <ConfirmDialog
        open={!!confirm}
        title={confirm ? `Cancel action #${confirm.id}?` : ''}
        confirmLabel="Cancel it with passkey"
        tone="danger"
        busy={op.phase === 'running'}
        onClose={() => setConfirm(null)}
        onConfirm={() => {
          const a = confirm
          setConfirm(null)
          if (a) void op.run({ callData: cancelCallData(a.id), describe: { kind: 'cancelled', title: `Cancelled vault action #${a.id}` } })
        }}
      >
        {confirm ? (
          <>
            <p>{confirm.calls.map((c) => describeCall(rec.address, c, describer)).join(' · ')}</p>
            <p className="w-fine">It will never execute. Cancelling is instant, even with the vault on, and costs only gas.</p>
          </>
        ) : null}
      </ConfirmDialog>
    </Page>
  )
}

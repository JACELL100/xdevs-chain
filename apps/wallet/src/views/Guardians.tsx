'use client'
// Guardians: who protects this account (quorum, add / remove), freeze and recovery state, and the tool to act as a
// guardian for someone else (cancel votes, freeze / unfreeze, recovery approvals) from a browser wallet.
import { AddressChip, Badge, Button, cx } from '@xdevs/ui'
import { useEffect, useState } from 'react'
import { type Address, type Hex, type WalletClient, encodeFunctionData, getAddress, isAddress, isHex, pad } from 'viem'
import { IconSnow, IconUsers, IconX } from '../components/icons.tsx'
import { OpProgress } from '../components/review.tsx'
import { Avatar, DemoTag, Field, KV, Notice, Page, SectionTitle, useNow } from '../components/ui.tsx'
import { config, explorerAddress } from '../config.ts'
import type { RecoveryStatus } from '../data/types.ts'
import { accountAbi } from '../lib/abi.ts'
import type { AccountState } from '../lib/account.ts'
import { describeCall } from '../lib/describe.ts'
import { fmtCountdown, fmtDate, fmtDuration } from '../lib/format.ts'
import { connectInjected, sendAndWait } from '../lib/injected.ts'
import { type SignerStruct, ecdsaSignerStruct } from '../lib/signers.ts'
import type { QueuedAction } from '../lib/vault.ts'
import { setGuardiansCallData } from '../lib/vault.ts'
import { useDescriber } from '../wallet/describer.ts'
import { useLoad } from '../wallet/hooks.ts'
import { humanError, useOp, useWallet } from '../wallet/WalletProvider.tsx'

export function Guardians() {
  const w = useWallet()
  const { rec, state } = w
  const describer = useDescriber()
  const op = useOp()
  const now = useNow()
  const recovery = useLoad(rec && state?.deployed ? () => w.backend.recoveryStatus(rec.address) : null, [rec?.address, state?.deployed, state?.lastActionId, w.backend])
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState<Address[]>([])
  const [quorum, setQuorum] = useState(0)
  const [add, setAdd] = useState('')
  const [addErr, setAddErr] = useState<string | null>(null)

  useEffect(() => {
    if (!state) return
    setDraft(state.guardians)
    setQuorum(state.guardianThreshold)
  }, [state?.guardians.length, state?.guardianThreshold])

  if (!rec) return null
  if (!state) return <Page title="Guardians">{null}</Page>
  const readOnly = rec.kind === 'watch' || !state.deployed
  const label = (a: Address) => describer.label?.(a) ?? null

  async function addGuardian() {
    setAddErr(null)
    try {
      const r = await w.backend.resolveRecipient(add)
      if (r.address.toLowerCase() === rec!.address.toLowerCase()) throw new Error('Your own account cannot be its guardian.')
      if (draft.some((d) => d.toLowerCase() === r.address.toLowerCase())) throw new Error('Already a guardian.')
      if (draft.length >= 16) throw new Error('At most 16 guardians.')
      const next = [...draft, r.address]
      setDraft(next)
      if (quorum === 0) setQuorum(1)
      setAdd('')
    } catch (e) {
      setAddErr(humanError(e))
    }
  }

  const changed = draft.join() !== state.guardians.join() || quorum !== state.guardianThreshold
  const delayed = state.policy.enabled ? `This change waits ${fmtDuration(Number(state.policy.delay))} in your Guardian Vault and can be cancelled meanwhile.` : 'The vault is off: this applies immediately.'

  return (
    <Page
      title={<>Guardians {w.demo ? <DemoTag /> : null}</>}
      sub="People or devices you trust. Together, a quorum of them can stop a thief: cancel queued transfers, freeze the account and recover it to a new passkey."
    >
      <OpProgress phase={op.phase} status={op.status} error={op.error} queued={!!op.result?.queuedIds.length} doneText={op.result?.queuedIds.length ? `Queued as vault action #${op.result.queuedIds[0]}.` : 'Done.'}>
        {op.phase !== 'running' ? (
          <Button size="sm" variant="ghost" onClick={op.reset}>
            Close
          </Button>
        ) : null}
      </OpProgress>

      <div className="w-dash-grid is-wide-left">
        <section className="w-panel">
          <SectionTitle
            action={
              !readOnly && !editing ? (
                <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
                  Edit
                </Button>
              ) : null
            }
          >
            Your guardians
          </SectionTitle>
          {state.guardians.length || editing ? (
            <>
              <div className="w-quorum" aria-label={`${editing ? quorum : state.guardianThreshold} of ${(editing ? draft : state.guardians).length} must agree`}>
                {(editing ? draft : state.guardians).map((g, i) => (
                  <span key={g} className={cx('w-quorum-dot', i < (editing ? quorum : state.guardianThreshold) && 'is-on')} />
                ))}
                <p>
                  <b>
                    {editing ? quorum : state.guardianThreshold} of {(editing ? draft : state.guardians).length}
                  </b>{' '}
                  must agree
                </p>
              </div>
              <ul className="w-guardians">
                {(editing ? draft : state.guardians).map((g) => (
                  <li key={g}>
                    <Avatar address={g} size={32} />
                    <div>
                      <b>{label(g) ?? 'Guardian'}</b>
                      <AddressChip address={g} href={explorerAddress(g)} />
                    </div>
                    {editing ? (
                      <button
                        type="button"
                        className="w-icon-btn"
                        aria-label={`Remove guardian ${label(g) ?? g}`}
                        onClick={() => {
                          const next = draft.filter((d) => d !== g)
                          setDraft(next)
                          setQuorum((q) => Math.min(q, next.length))
                        }}
                      >
                        <IconX size={14} />
                      </button>
                    ) : null}
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <div className="w-empty">
              <IconUsers size={20} />
              <p>No guardians yet.</p>
              <p className="w-fine">Without guardians there is no recovery and no freeze: losing your passkey means losing the account.</p>
            </div>
          )}
          {editing ? (
            <div className="w-form">
              <div className="w-inline-form">
                <input className="w-input is-mono" value={add} onChange={(e) => setAdd(e.target.value)} placeholder="0x… or name.xdevs" aria-label="New guardian" spellCheck={false} />
                <Button size="sm" variant="ghost" disabled={!add.trim()} onClick={() => void addGuardian()}>
                  Add
                </Button>
              </div>
              {addErr ? <p className="w-field-error">{addErr}</p> : null}
              {draft.length ? (
                <Field label="Quorum" hint="Majority is a good default: one rogue guardian can never act alone." htmlFor="g-q">
                  <select id="g-q" className="w-input is-narrow" value={quorum} onChange={(e) => setQuorum(Number(e.target.value))}>
                    {draft.map((_, i) => (
                      <option key={i} value={i + 1}>
                        {i + 1} of {draft.length}
                      </option>
                    ))}
                  </select>
                </Field>
              ) : null}
              <Notice tone="info">{delayed}</Notice>
              <div className="w-flow-actions is-left">
                <Button disabled={!changed || op.phase === 'running'} onClick={() => void op.run({ callData: setGuardiansCallData(draft, draft.length ? quorum : 0), describe: { kind: 'settings', title: 'Change guardians' } }).then(() => setEditing(false))}>
                  Submit with passkey
                </Button>
                <Button
                  variant="quiet"
                  onClick={() => {
                    setEditing(false)
                    setDraft(state.guardians)
                    setQuorum(state.guardianThreshold)
                  }}
                >
                  Cancel
                </Button>
              </div>
            </div>
          ) : null}
        </section>

        <section className="w-panel">
          <SectionTitle>Freeze and recovery</SectionTitle>
          <div className={cx('w-freeze', state.frozen && 'is-frozen')}>
            <IconSnow size={18} />
            <div>
              <b>{state.frozen ? 'Frozen' : 'Not frozen'}</b>
              <p className="w-fine">
                {state.frozen
                  ? `Until ${fmtDate(Number(state.frozenUntil))} (${fmtCountdown(Number(state.frozenUntil) - now)}). Nothing can be signed meanwhile.`
                  : `If your passkey is stolen, ${state.guardianThreshold || 'a quorum'} of your guardians can freeze everything for up to ${fmtDuration(Number(state.freezeDuration || 2_592_000n))}.`}
              </p>
            </div>
          </div>
          <KV
            rows={[
              ['Recovery', recovery.data ? <RecoveryText r={recovery.data} /> : '…'],
              ['Recovery delay', fmtDuration(Number(state.recoveryDelay || 172_800n))],
              ['Next freeze allowed', Number(state.nextFreezeAllowedAt) > now ? `in ${fmtCountdown(Number(state.nextFreezeAllowedAt) - now)}` : 'any time'],
            ]}
          />
          <p className="w-fine">
            Guardians are trusted for safety, not just recovery: a quorum can recover the account to keys of its choice. A single guardian below the quorum
            can do nothing alone.
          </p>
        </section>
      </div>

      <GuardianDuty />
    </Page>
  )
}

function RecoveryText({ r }: { r: RecoveryStatus }) {
  return <span className={r.kind === 'ready' || r.kind === 'proposed' ? 'w-text-alert' : undefined}>{r.text}</span>
}

/** Parses "0x<address>" (ECDSA signer) or "x,y" (passkey public key) per line. */
function parseSigners(text: string): SignerStruct[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => {
      if (isAddress(l, { strict: false })) return ecdsaSignerStruct(getAddress(l))
      const [x, y] = l.split(/[\s,]+/)
      if (isHex(x) && isHex(y)) return { kind: 2, x: pad(x as Hex), y: pad(y as Hex) }
      throw new Error(`Not a signer: ${l}`)
    })
}

/** Acting as a guardian of someone else's account, from a browser wallet. */
function GuardianDuty() {
  const w = useWallet()
  const describer = useDescriber()
  const now = useNow()
  const [open, setOpen] = useState(false)
  const [me, setMe] = useState<{ address: Address; wallet: WalletClient } | null>(null)
  const [target, setTarget] = useState('')
  const [st, setSt] = useState<AccountState | null>(null)
  const [queue, setQueue] = useState<QueuedAction[]>([])
  const [signers, setSigners] = useState('')
  const [msg, setMsg] = useState<{ tone: 'ok' | 'alert' | 'info'; text: string } | null>(null)

  async function load() {
    setMsg(null)
    try {
      const r = await w.backend.resolveRecipient(target)
      const s = await w.backend.readAccount(r.address)
      setSt(s)
      setQueue(s.deployed ? await w.backend.vaultQueue(r.address) : [])
    } catch (e) {
      setMsg({ tone: 'alert', text: humanError(e) })
    }
  }

  async function act(label: string, data: Hex) {
    if (!me || !st) return
    setMsg({ tone: 'info', text: `${label}: confirm in your wallet…` })
    try {
      await sendAndWait(me.wallet, st.address, data)
      setMsg({ tone: 'ok', text: `${label}: done. It takes effect when the quorum is reached.` })
      await load()
    } catch (e) {
      setMsg({ tone: 'alert', text: `${label}: ${humanError(e)}` })
    }
  }

  const isGuardian = !!me && !!st?.guardians.some((g) => g.toLowerCase() === me.address.toLowerCase())
  const openActions = queue.filter((q) => q.status === 'queued' || q.status === 'ready')
  return (
    <section className="w-panel w-duty">
      <button type="button" className="w-duty-head" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <div>
          <h2>Are you someone&apos;s guardian?</h2>
          <p className="w-fine">Vote to cancel a suspicious transfer, freeze or recover an account you protect, from MetaMask or any browser wallet.</p>
        </div>
        <span className="w-disclose">{open ? 'Hide' : 'Open'}</span>
      </button>
      {open ? (
        <div className="w-form">
          {w.demo ? <Notice tone="info">Guardian voting needs a real browser wallet, so it is not available in the demo.</Notice> : null}
          <div className="w-inline-form">
            {me ? (
              <p className="w-me">
                Connected: <AddressChip address={me.address} /> {st ? isGuardian ? <Badge tone="final">guardian of this account</Badge> : <Badge tone="warn">not a guardian of this account</Badge> : null}
              </p>
            ) : (
              <Button size="sm" disabled={w.demo} onClick={() => void connectInjected().then(setMe, (e) => setMsg({ tone: 'alert', text: humanError(e) }))}>
                Connect browser wallet
              </Button>
            )}
          </div>
          <div className="w-inline-form">
            <input className="w-input is-mono" value={target} onChange={(e) => setTarget(e.target.value)} placeholder="Protected account: 0x… or name.xdevs" aria-label="Protected account" spellCheck={false} />
            <Button size="sm" variant="ghost" disabled={!target.trim()} onClick={() => void load()}>
              Load
            </Button>
          </div>
          {msg ? <Notice tone={msg.tone}>{msg.text}</Notice> : null}
          {st ? (
            <>
              <p className="w-fine">
                Quorum {st.guardianThreshold} of {st.guardians.length}. {st.frozen ? `Frozen until ${fmtDate(Number(st.frozenUntil))}.` : 'Not frozen.'}
              </p>
              {openActions.length ? (
                <ul className="w-plain-list">
                  {openActions.map((a) => (
                    <li key={String(a.id)}>
                      <span>
                        #{String(a.id)} {a.calls.map((c) => describeCall(st.address, c, describer)).join(' · ')} · {Number(a.executeAfter) > now ? `executes in ${fmtCountdown(Number(a.executeAfter) - now)}` : 'executable now'} · votes {a.cancelVotes}/{st.guardianThreshold}
                      </span>
                      <Button size="sm" variant="danger" disabled={!isGuardian} onClick={() => void act(`Cancel vote #${a.id}`, encodeFunctionData({ abi: accountAbi, functionName: 'cancel', args: [a.id] }))}>
                        Vote to cancel
                      </Button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="w-muted">No queued actions.</p>
              )}
              <div className="w-flow-actions is-left">
                <Button size="sm" variant="danger" disabled={!isGuardian || st.frozen} onClick={() => void act('Freeze vote', encodeFunctionData({ abi: accountAbi, functionName: 'freeze' }))}>
                  Vote to freeze
                </Button>
                <Button size="sm" variant="ghost" disabled={!isGuardian || !st.frozen} onClick={() => void act('Unfreeze vote', encodeFunctionData({ abi: accountAbi, functionName: 'unfreeze' }))}>
                  Vote to unfreeze
                </Button>
              </div>
              <Field label="Recovery: the owner's new signers" hint={'One per line: an address (key) or a passkey public key "x,y". Approvals count per exact set.'} htmlFor="rec-signers">
                <textarea id="rec-signers" className="w-input is-mono" rows={3} value={signers} onChange={(e) => setSigners(e.target.value)} spellCheck={false} />
              </Field>
              <div className="w-flow-actions is-left">
                <Button
                  size="sm"
                  disabled={!isGuardian || !signers.trim()}
                  onClick={() => {
                    try {
                      void act('Approve recovery', encodeFunctionData({ abi: accountAbi, functionName: 'approveRecovery', args: [parseSigners(signers)] }))
                    } catch (e) {
                      setMsg({ tone: 'alert', text: humanError(e) })
                    }
                  }}
                >
                  Approve recovery
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={!me || !signers.trim()}
                  onClick={() => {
                    try {
                      void act('Execute recovery', encodeFunctionData({ abi: accountAbi, functionName: 'executeRecovery', args: [parseSigners(signers)] }))
                    } catch (e) {
                      setMsg({ tone: 'alert', text: humanError(e) })
                    }
                  }}
                >
                  Execute when ready
                </Button>
              </div>
            </>
          ) : null}
          <p className="w-fine">Each action is a vote paid by your own wallet; it takes effect when {config.chainName} sees the quorum.</p>
        </div>
      ) : null}
    </section>
  )
}

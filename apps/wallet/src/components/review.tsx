'use client'
// Pre-sign review blocks shared by Send, Approvals and Names: Guardian Vault outcome, recipient checks, preview
// warnings ("advice, not fact"), and the passkey signing progress.
import { cx } from '@xdevs/ui'
import type { ReactNode } from 'react'
import { config } from '../config.ts'
import type { AccountState } from '../lib/account.ts'
import { fmtAmount, fmtDate, fmtDuration } from '../lib/format.ts'
import type { Preview } from '../lib/preview.ts'
import type { RecipientCheckResult } from '../lib/recipient.ts'
import type { VaultOutcome } from '../lib/vault.ts'
import type { OpPhase } from '../wallet/WalletProvider.tsx'
import { IconAlert, IconCheck, IconClock, IconInfo, IconShield, IconX } from './icons.tsx'
import { PasskeyVisual } from './PasskeyVisual.tsx'
import { Notice } from './ui.tsx'

/** Human explanation of what the Guardian Vault will do with this transaction. */
export function VaultOutcomeCard({ outcome, state }: { outcome: VaultOutcome; state: AccountState }) {
  const g = state.guardians.length
  if (outcome.kind === 'queued') {
    const delay = Number(outcome.delay)
    const at = Math.floor(Date.now() / 1000) + delay
    return (
      <div className="w-vault-out is-queued">
        <span className="w-vault-out-icon">
          <IconClock size={20} />
        </span>
        <div>
          <p className="w-vault-out-title">Will wait in your Guardian Vault for {fmtDuration(delay)}</p>
          <p className="w-vault-out-text">
            {capital(outcome.reason)}. Nothing leaves your account until {fmtDate(at)}. Until then you
            {g ? `, or ${state.guardianThreshold} of your ${g} guardians,` : ''} can cancel it with one tap. After the
            delay it executes.
          </p>
        </div>
      </div>
    )
  }
  if (outcome.kind === 'instant')
    return (
      <div className="w-vault-out is-instant">
        <span className="w-vault-out-icon">
          <IconShield size={20} />
        </span>
        <div>
          <p className="w-vault-out-title">Sends instantly</p>
          <p className="w-vault-out-text">
            {capital(outcome.reason)}
            {state.policy.enabled ? ` (instant up to ${fmtAmount(state.policy.nativeThreshold)} ${config.symbol} per transfer).` : '.'}
          </p>
        </div>
      </div>
    )
  if (outcome.kind === 'rejected')
    return (
      <div className="w-vault-out is-rejected" role="alert">
        <span className="w-vault-out-icon">
          <IconX size={20} />
        </span>
        <div>
          <p className="w-vault-out-title">Blocked by your Guardian Vault</p>
          <p className="w-vault-out-text">{capital(outcome.reason)}. This can't be signed.</p>
        </div>
      </div>
    )
  return (
    <Notice tone="info" title="Guardian Vault">
      {outcome.reason}
    </Notice>
  )
}

const capital = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s)

type Check = { tone: 'ok' | 'warn' | 'alert' | 'info'; text: ReactNode }

export function RecipientChecks({ check, name, label }: { check: RecipientCheckResult | null; name?: string | null; label?: string | null }) {
  if (!check) return null
  const items: Check[] = []
  if (check.lookAlikeOf.length)
    items.push({ tone: 'alert', text: <>Looks like an address you used before, but it is different. This is how address-poisoning scams work: check every character.</> })
  if (name) items.push({ tone: 'ok', text: <>Name verified: {name} resolves to this address</> })
  else if (check.name) items.push({ tone: 'ok', text: <>Has the verified name {check.name}</> })
  if (check.firstTime) items.push({ tone: 'warn', text: 'First time you send to this address' })
  else if (!check.lookAlikeOf.length) items.push({ tone: 'ok', text: 'You have sent to this address before' })
  if (check.isContract) items.push({ tone: 'info', text: `This is a contract${label ? ` (${label})` : ''}` })
  return (
    <ul className="w-checks">
      {items.map((c, i) => (
        <li key={i} className={`is-${c.tone}`}>
          <span>{c.tone === 'ok' ? <IconCheck size={14} /> : c.tone === 'info' ? <IconInfo size={14} /> : <IconAlert size={14} />}</span>
          {c.text}
        </li>
      ))}
    </ul>
  )
}

export function PreviewWarnings({ preview }: { preview: Preview }) {
  const shown = preview.warnings.filter((w) => w.code !== 'FIRST_TIME_RECIPIENT' && w.code !== 'LOOKALIKE_RECIPIENT')
  return (
    <div className="w-preview">
      <div className="w-preview-head">
        <span>Simulation</span>
        <span className="w-advice" title={preview.disclaimer}>
          advice, not fact
        </span>
      </div>
      <ul className="w-preview-summary">
        {preview.summary.map((s) => (
          <li key={s}>{s}</li>
        ))}
      </ul>
      {!preview.success ? (
        <Notice tone="alert" title="This would fail">
          {preview.revertReason ?? 'The simulation reverted.'}
        </Notice>
      ) : null}
      {shown.map((w) => (
        <div key={w.code + w.message} className={cx('w-warning', `sev-${w.severity}`)}>
          <span className="w-warning-sev">{w.severity}</span>
          <p>{w.message}</p>
        </div>
      ))}
      <p className="w-preview-foot">
        {preview.source === 'node' ? 'Simulated by the node.' : 'Simulated locally against the current state.'} Results can change before inclusion.
      </p>
    </div>
  )
}

const STEPS = ['Confirm with your passkey', 'Submitting', 'Waiting for inclusion']

function stepIndex(status: string | null): number {
  if (!status) return 0
  if (/passkey|Signing/i.test(status)) return 0
  if (/Submitting/i.test(status)) return 1
  if (/Waiting|inclusion/i.test(status)) return 2
  return 0
}

/** Signing / submission progress with the biometric glyph. */
export function OpProgress({ phase, status, error, queued, doneText, children }: { phase: OpPhase; status: string | null; error: string | null; queued?: boolean; doneText?: ReactNode; children?: ReactNode }) {
  if (phase === 'idle') return null
  const i = stepIndex(status)
  return (
    <div className={cx('w-progress', `is-${phase}`)} aria-live="polite">
      <PasskeyVisual state={phase === 'done' ? 'done' : phase === 'error' ? 'error' : 'scanning'} size={96} />
      {phase === 'running' ? (
        <>
          <p className="w-progress-title">{status ?? 'Preparing…'}</p>
          <ol className="w-steps">
            {STEPS.map((s, k) => (
              <li key={s} className={k < i ? 'is-done' : k === i ? 'is-now' : undefined}>
                {s}
              </li>
            ))}
          </ol>
        </>
      ) : phase === 'done' ? (
        <>
          <p className="w-progress-title">{queued ? 'Queued in your Guardian Vault' : 'Done'}</p>
          {doneText ? <p className="w-progress-text">{doneText}</p> : null}
        </>
      ) : (
        <>
          <p className="w-progress-title">Not completed</p>
          <p className="w-progress-text">{error}</p>
        </>
      )}
      {children}
    </div>
  )
}

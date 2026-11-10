import type { Preview } from '../lib/preview.ts'
import type { VaultOutcome } from '../lib/vault.ts'
import { formatCountdown } from '../lib/vault.ts'
import { Notice } from './common.tsx'

export function VaultBadge({ outcome }: { outcome: VaultOutcome | null }) {
  if (!outcome) return null
  if (outcome.kind === 'instant') return <Notice kind="ok">Guardian Vault: executes instantly — {outcome.reason}.</Notice>
  if (outcome.kind === 'queued')
    return (
      <Notice kind="warn">
        Guardian Vault: this will be <b>queued for {formatCountdown(outcome.delay)}</b> ({outcome.reason}). You or a guardian
        quorum can cancel it during that window; after it, anyone can execute it.
      </Notice>
    )
  if (outcome.kind === 'rejected') return <Notice kind="danger">Guardian Vault: this will be rejected — {outcome.reason}.</Notice>
  return <Notice kind="info">Guardian Vault: {outcome.reason}</Notice>
}

export function PreviewPanel({ preview }: { preview: Preview | null }) {
  if (!preview) return null
  return (
    <div className="preview">
      <div className="preview-head">
        <b>Preview</b>
        <span className="badge">{preview.source === 'node' ? 'node simulation (xdevs_simulateTransaction)' : 'local preview (eth_call + decoding)'}</span>
        <span className="badge warn">advice, not fact</span>
      </div>
      <ul className="summary">
        {preview.summary.map((s) => (
          <li key={s}>{s}</li>
        ))}
      </ul>
      {!preview.success && <Notice kind="danger">Simulation: this call would fail{preview.revertReason ? ` — ${preview.revertReason}` : ''}.</Notice>}
      {preview.warnings.map((w) => (
        <div key={w.code + w.message} className={`warning sev-${w.severity}`}>
          <b>{w.severity.toUpperCase()}</b> <code>{w.code}</code> {w.message}
        </div>
      ))}
      {preview.warnings.length === 0 && <p className="muted">No warnings.</p>}
      <p className="disclaimer">{preview.disclaimer}</p>
    </div>
  )
}

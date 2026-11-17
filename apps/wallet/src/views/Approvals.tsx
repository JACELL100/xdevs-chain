'use client'
// Approvals: every live token approval with its expiry, one-tap revoke (always instant, even with the vault on),
// and a checker that shows what the wallet says about an approval before a dApp asks for one.
import { AddressChip, Badge, Button } from '@xdevs/ui'
import { useState } from 'react'
import { type Address, encodeFunctionData, getAddress, isAddress, maxUint256 } from 'viem'
import { IconKey } from '../components/icons.tsx'
import { OpProgress, PreviewWarnings, VaultOutcomeCard } from '../components/review.tsx'
import { ConfirmDialog, DemoTag, Field, Notice, Page, SectionTitle, TokenGlyph, Toggle, useNow } from '../components/ui.tsx'
import { config, explorerAddress } from '../config.ts'
import { demoTokens } from '../data/demo.ts'
import type { Review } from '../data/types.ts'
import { erc20Abi } from '../lib/abi.ts'
import { type ApprovalItem, revokeAllCalls, revokeCall } from '../lib/approvals.ts'
import { fmtAmount, fmtDate, parseAmount } from '../lib/format.ts'
import { hasHighSeverity } from '../lib/preview.ts'
import { useDescriber } from '../wallet/describer.ts'
import { useLoad } from '../wallet/hooks.ts'
import { humanError, useOp, useWallet } from '../wallet/WalletProvider.tsx'

function expiryText(expiry: number | null, now: number): { text: string; tone: 'alert' | 'warn' | 'dim' } {
  if (expiry === null) return { text: 'Never expires', tone: 'alert' }
  const left = expiry - now
  if (left <= 0) return { text: 'Expired', tone: 'dim' }
  const days = Math.floor(left / 86_400)
  if (days >= 1) return { text: `Expires in ${days} day${days === 1 ? '' : 's'}`, tone: days <= 3 ? 'warn' : 'dim' }
  return { text: `Expires in ${Math.max(1, Math.floor(left / 3600))} h`, tone: 'warn' }
}

export function Approvals() {
  const w = useWallet()
  const { rec, state } = w
  const now = useNow(60_000)
  const describer = useDescriber()
  const list = useLoad(rec ? () => w.backend.approvals(rec.address) : null, [rec?.address, w.backend])
  const op = useOp(() => void list.reload())
  const [confirm, setConfirm] = useState<ApprovalItem[] | null>(null)

  if (!rec) return null
  const items = list.data
  const readOnly = rec.kind === 'watch' || !state?.deployed
  const vaultOn = !!state?.policy.enabled
  const name = (a: Address) => describer.label?.(a) ?? null

  return (
    <Page
      title={<>Approvals {w.demo ? <DemoTag /> : null}</>}
      sub="Apps you allowed to spend your tokens. On xDEVS Chain approvals expire, and revoking one is always instant."
      actions={
        items && items.length > 1 && !readOnly ? (
          <Button variant="danger" size="sm" onClick={() => setConfirm(items)}>
            Revoke all
          </Button>
        ) : null
      }
    >
      <OpProgress phase={op.phase} status={op.status} error={op.error} doneText="Revoked. The app can no longer spend these tokens.">
        {op.phase !== 'running' ? (
          <Button size="sm" variant="ghost" onClick={op.reset}>
            Close
          </Button>
        ) : null}
      </OpProgress>
      {list.error ? <Notice tone="alert" title="Approvals could not be loaded">{list.error}</Notice> : null}
      <section className="w-panel is-flush">
        {!items ? (
          <p className="w-pad w-muted">Loading approvals…</p>
        ) : items.length === 0 ? (
          <div className="w-empty">
            <IconKey size={20} />
            <p>No live approvals.</p>
            <p className="w-fine">No app can spend your tokens right now.</p>
          </div>
        ) : (
          <ul className="w-approvals">
            {items.map((a) => {
              const exp = expiryText(a.expiry, now)
              const meta = describer.token?.(a.token)
              return (
                <li key={`${a.kind}-${a.token}-${a.spender}`}>
                  <TokenGlyph symbol={a.symbol ?? '?'} size={36} />
                  <div className="w-appr-main">
                    <p>
                      <b>{name(a.spender) ?? 'Unknown app'}</b> can spend{' '}
                      {a.unlimited ? <b className="w-text-alert">ALL your {a.symbol ?? 'tokens'}</b> : <b className="xh-num">{fmtAmount(a.amount, meta?.decimals ?? 18)} {a.symbol}</b>}
                    </p>
                    <span className="w-appr-meta">
                      <AddressChip address={a.spender} href={explorerAddress(a.spender)} />
                      <Badge tone="dim">{a.kind === 'permit2' ? 'Permit2' : a.kind === 'xdrc20' ? 'XDRC-20' : 'ERC-20'}</Badge>
                    </span>
                  </div>
                  <div className="w-appr-exp">
                    <Badge tone={exp.tone} dot={exp.tone === 'alert'}>
                      {exp.text}
                    </Badge>
                    {a.expiry ? <span className="w-fine">{fmtDate(a.expiry)}</span> : <span className="w-fine">plain ERC-20</span>}
                  </div>
                  {!readOnly ? (
                    <Button size="sm" variant="ghost" disabled={op.phase === 'running'} onClick={() => setConfirm([a])}>
                      Revoke
                    </Button>
                  ) : null}
                </li>
              )
            })}
          </ul>
        )}
      </section>
      <p className="w-fine">
        Source: {config.apiUrl && !w.demo ? 'explorer API, with a fallback to the node and a scan of known tokens' : w.demo ? 'demo fixtures' : 'the node, or a scan of known tokens and Permit2Expiring over the last ~9,000 blocks'}.{' '}
        {vaultOn ? 'With the vault on, "revoke all" sends one revocation per approval, because those are on the instant path.' : null}
      </p>

      <ApprovalChecker />

      <ConfirmDialog
        open={!!confirm}
        title={confirm && confirm.length > 1 ? `Revoke all ${confirm.length} approvals?` : 'Revoke this approval?'}
        confirmLabel="Revoke with passkey"
        tone="danger"
        busy={op.phase === 'running'}
        onClose={() => setConfirm(null)}
        onConfirm={() => {
          const c = confirm
          setConfirm(null)
          if (!c) return
          const calls = c.length > 1 ? revokeAllCalls(c, vaultOn) : [revokeCall(c[0])]
          void op.run({ calls, describe: { kind: 'revoke', title: c.length > 1 ? `Revoked ${c.length} approvals` : `Revoked ${c[0].symbol ?? 'token'} approval` } })
        }}
      >
        <p>{confirm && confirm.length === 1 ? `${name(confirm[0].spender) ?? confirm[0].spender} will no longer be able to spend your ${confirm[0].symbol ?? 'tokens'}.` : 'No app will be able to spend your tokens until you approve it again.'}</p>
        <p className="w-fine">Revocations execute instantly, even while the Guardian Vault is on.</p>
      </ConfirmDialog>
    </Page>
  )
}

function ApprovalChecker() {
  const w = useWallet()
  const { rec, state } = w
  const [open, setOpen] = useState(false)
  const [token, setToken] = useState<string>(w.demo ? demoTokens()[0] : (config.knownTokens[0] ?? ''))
  const [spender, setSpender] = useState('')
  const [amount, setAmount] = useState('')
  const [unlimited, setUnlimited] = useState(true)
  const [review, setReview] = useState<Review | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [ack, setAck] = useState(false)
  const op = useOp()

  async function check() {
    if (!state) return
    setErr(null)
    setReview(null)
    setAck(false)
    try {
      if (!isAddress(token, { strict: false })) throw new Error('Enter the token contract address.')
      const s = await w.backend.resolveRecipient(spender)
      const decimals = await w.backend.tokenDecimals(getAddress(token))
      const value = unlimited ? maxUint256 : parseAmount(amount, decimals)
      const call = { target: getAddress(token), value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [s.address, value] }) }
      setReview(await w.backend.review(state, call, s, null))
    } catch (e) {
      setErr(humanError(e))
    }
  }

  return (
    <section className="w-panel w-duty">
      <button type="button" className="w-duty-head" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <div>
          <h2>Check an approval before you grant it</h2>
          <p className="w-fine">See what the simulation and your Guardian Vault say about the &quot;unlimited&quot; approval a dApp asks for.</p>
        </div>
        <span className="w-disclose">{open ? 'Hide' : 'Open'}</span>
      </button>
      {open ? (
        <div className="w-form">
          <div className="w-grid2">
            <Field label="Token contract" htmlFor="ap-token">
              <input id="ap-token" className="w-input is-mono" value={token} onChange={(e) => setToken(e.target.value)} placeholder="0x…" spellCheck={false} />
            </Field>
            <Field label="Spender (the app)" htmlFor="ap-spender">
              <input id="ap-spender" className="w-input is-mono" value={spender} onChange={(e) => setSpender(e.target.value)} placeholder="0x… or name.xdevs" spellCheck={false} />
            </Field>
          </div>
          <Toggle checked={unlimited} onChange={setUnlimited} label="Unlimited amount" hint="What many dApps ask for by default." />
          {!unlimited ? (
            <Field label="Amount" htmlFor="ap-amount">
              <input id="ap-amount" className="w-input" inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} />
            </Field>
          ) : null}
          <div className="w-flow-actions is-left">
            <Button size="sm" variant="ghost" disabled={!state || !spender.trim()} onClick={() => void check()}>
              Check
            </Button>
          </div>
          {err ? <Notice tone="alert">{err}</Notice> : null}
          {review && state ? (
            <>
              <VaultOutcomeCard outcome={review.vault} state={state} />
              <PreviewWarnings preview={review.preview} />
              {hasHighSeverity(review.preview) ? (
                <label className="w-ack">
                  <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />
                  <span>I understand the warnings above.</span>
                </label>
              ) : null}
              <OpProgress phase={op.phase} status={op.status} error={op.error} queued={!!op.result?.queuedIds.length} />
              <Button
                size="sm"
                disabled={!rec || rec.kind === 'watch' || review.vault.kind === 'rejected' || (hasHighSeverity(review.preview) && !ack) || op.phase === 'running'}
                onClick={() => void op.run({ calls: [review.call], describe: { kind: 'approval', title: 'Approval granted' } })}
              >
                Grant with passkey
              </Button>
            </>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}

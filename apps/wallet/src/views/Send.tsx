'use client'
// Send: recipient (address or .xdevs name) with poisoning checks, strict amounts, simulation preview, Guardian Vault
// outcome, one passkey confirmation.
import { Button, Skeleton } from '@xdevs/ui'
import Link from 'next/link'
import { useEffect, useMemo, useState } from 'react'
import { type Address, type Hex, encodeFunctionData, isHex } from 'viem'
import { IconCheck } from '../components/icons.tsx'
import { OpProgress, PreviewWarnings, RecipientChecks, VaultOutcomeCard } from '../components/review.tsx'
import { AddressFull, DemoTag, Field, Notice, Page, Segmented, TokenGlyph } from '../components/ui.tsx'
import { config } from '../config.ts'
import { demoLabel } from '../data/demo.ts'
import type { Asset, Review } from '../data/types.ts'
import { erc20Abi } from '../lib/abi.ts'
import type { Call } from '../lib/account.ts'
import { fmtAmount, fmtDate, fmtExact, parseAmount } from '../lib/format.ts'
import { hasHighSeverity } from '../lib/preview.ts'
import type { ResolvedRecipient } from '../lib/recipient.ts'
import { recipientHistory, rememberRecipient } from '../store.ts'
import { useLoad } from '../wallet/hooks.ts'
import { humanError, useOp, useWallet } from '../wallet/WalletProvider.tsx'

type Mode = 'native' | 'token' | 'call'

export function Send() {
  const w = useWallet()
  const { rec, state } = w
  const [mode, setMode] = useState<Mode>('native')
  const [to, setTo] = useState('')
  const [amount, setAmount] = useState('')
  const [tokenIdx, setTokenIdx] = useState(0)
  const [data, setData] = useState('0x')
  const [resolved, setResolved] = useState<ResolvedRecipient | null>(null)
  const [resolveErr, setResolveErr] = useState<string | null>(null)
  const [review, setReview] = useState<Review | null>(null)
  const [reviewErr, setReviewErr] = useState<string | null>(null)
  const [reviewing, setReviewing] = useState(false)
  const [ack, setAck] = useState(false)
  const op = useOp((r) => {
    if (review?.recipient) rememberRecipient(rec!.address, review.recipient.address)
    void r
  })

  const assets = useLoad(rec && state ? () => w.backend.assets(rec.address, state.balance) : null, [rec?.address, state?.balance, w.backend])
  const tokens = useMemo(() => (assets.data ?? []).filter((a) => a.token), [assets.data])
  const asset: Asset | undefined = mode === 'token' ? tokens[tokenIdx] : assets.data?.[0]

  useEffect(() => {
    const q = new URLSearchParams(window.location.search).get('to')
    if (q) setTo(q)
  }, [])

  // Resolve the recipient as the user types (debounced).
  useEffect(() => {
    setResolved(null)
    setResolveErr(null)
    setReview(null)
    const s = to.trim()
    if (!s) return
    const t = setTimeout(() => {
      w.backend.resolveRecipient(s).then(setResolved, (e) => setResolveErr(humanError(e)))
    }, 350)
    return () => clearTimeout(t)
  }, [to, w.backend])

  let amountErr: string | null = null
  let value = 0n
  if (amount.trim()) {
    try {
      value = parseAmount(amount, asset?.decimals ?? 18)
      if (asset && value > asset.balance) amountErr = `More than your balance (${fmtAmount(asset.balance, asset.decimals)} ${asset.symbol}).`
      if (value === 0n && mode !== 'call') amountErr = 'Enter an amount above zero.'
    } catch (e) {
      amountErr = (e as Error).message
    }
  }

  function buildCall(r: ResolvedRecipient): Call {
    if (mode === 'native') return { target: r.address, value, data: '0x' }
    if (mode === 'token') {
      if (!asset?.token) throw new Error('Pick a token.')
      return { target: asset.token, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'transfer', args: [r.address, value] }) }
    }
    if (!isHex(data)) throw new Error('Calldata must be 0x-prefixed hex.')
    return { target: r.address, value, data: data as Hex }
  }

  async function doReview() {
    if (!rec || !state || !resolved) return
    setReviewing(true)
    setReviewErr(null)
    setReview(null)
    setAck(false)
    try {
      const call = buildCall(resolved)
      const check = await w.backend.checkRecipient(rec.address, resolved.address, recipientHistory(rec.address))
      setReview(await w.backend.review(state, call, resolved, check))
    } catch (e) {
      setReviewErr(humanError(e))
    } finally {
      setReviewing(false)
    }
  }

  if (!rec) return null
  const lookAlike = !!review?.check?.lookAlikeOf.length
  const needsAck = !!review && (hasHighSeverity(review.preview) || lookAlike)
  const blocked =
    !review || review.vault.kind === 'rejected' || (needsAck && !ack) || rec.kind === 'watch' || !state?.deployed || !!state?.frozen || op.phase === 'running'
  const symbol = mode === 'token' ? (asset?.symbol ?? 'token') : config.symbol
  const label = resolved ? (resolved.name ?? (w.demo ? demoLabel(resolved.address) : null)) : null
  const queued = review?.vault.kind === 'queued'

  return (
    <Page title={<>Send {w.demo ? <DemoTag /> : null}</>} sub="Every transfer is checked and simulated before you confirm it with your passkey." narrow>
      {op.phase === 'done' ? (
        <section className="w-panel w-sent">
          <OpProgress
            phase="done"
            status={op.status}
            error={null}
            queued={!!op.result?.queuedIds.length}
            doneText={
              op.result?.queuedIds.length ? (
                <>
                  Action #{String(op.result.queuedIds[0])} waits in your Guardian Vault until {fmtDate(Math.floor(Date.now() / 1000) + Number(state?.policy.delay ?? 0n))}. You can cancel it until then.
                </>
              ) : (
                <>
                  {fmtExact(value, asset?.decimals ?? 18)} {symbol} sent to {label ?? resolved?.address}.
                </>
              )
            }
          >
            <div className="w-flow-actions">
              {op.result?.queuedIds.length ? (
                <Button href="/vault" linkComponent={Link}>
                  Open the vault
                </Button>
              ) : (
                <Button href="/" linkComponent={Link}>
                  Back to overview
                </Button>
              )}
              <Button
                variant="ghost"
                onClick={() => {
                  op.reset()
                  setReview(null)
                  setAmount('')
                  setTo('')
                }}
              >
                New transfer
              </Button>
            </div>
          </OpProgress>
        </section>
      ) : (
        <div className="w-send">
          <section className="w-panel w-send-form">
            <Segmented
              label="What to send"
              value={mode}
              onChange={(m) => {
                setMode(m)
                setReview(null)
              }}
              options={[
                { id: 'native', label: config.symbol },
                { id: 'token', label: 'Token' },
                { id: 'call', label: 'Contract call' },
              ]}
            />
            {mode === 'token' ? (
              tokens.length ? (
                <div className="w-token-pick" role="radiogroup" aria-label="Token">
                  {tokens.map((t, i) => (
                    <button key={t.token} type="button" role="radio" aria-checked={i === tokenIdx} className={i === tokenIdx ? 'is-on' : undefined} onClick={() => setTokenIdx(i)}>
                      <TokenGlyph symbol={t.symbol} size={28} />
                      <span>
                        <b>{t.symbol}</b>
                        <em className="xh-num">{fmtAmount(t.balance, t.decimals, 2)}</em>
                      </span>
                    </button>
                  ))}
                </div>
              ) : assets.data ? (
                <Notice tone="info">No tokens in this account yet.</Notice>
              ) : (
                <Skeleton height={48} />
              )
            ) : null}

            <Field
              label={mode === 'call' ? 'Contract' : 'To'}
              htmlFor="to"
              error={resolveErr}
              hint={resolved ? null : 'An address (0x…) or a name like maya.xdevs'}
            >
              <input
                id="to"
                className="w-input is-mono"
                value={to}
                onChange={(e) => setTo(e.target.value)}
                placeholder="0x… or name.xdevs"
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            </Field>
            {resolved ? (
              <div className="w-resolved">
                {resolved.name ? (
                  <p className="w-resolved-name">
                    <IconCheck size={14} /> {resolved.name}
                  </p>
                ) : label ? (
                  <p className="w-resolved-name is-plain">{label}</p>
                ) : null}
                <AddressFull address={resolved.address} />
              </div>
            ) : null}

            <Field
              label={`Amount${mode === 'call' ? ` (${config.symbol}, optional)` : ''}`}
              htmlFor="amount"
              error={amountErr}
              aside={
                asset ? (
                  <button type="button" className="w-max" onClick={() => setAmount(fmtExact(asset.balance, asset.decimals).replace(/,/g, ''))}>
                    Balance {fmtAmount(asset.balance, asset.decimals)} {asset.symbol}
                  </button>
                ) : null
              }
            >
              <div className="w-amount">
                <input id="amount" className="w-input is-amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} placeholder="0.0" autoComplete="off" />
                <span>{symbol}</span>
              </div>
            </Field>
            {mode === 'call' ? (
              <Field label="Calldata" hint="What a dApp asks you to sign. The wallet decodes what it can." htmlFor="data">
                <textarea id="data" className="w-input is-mono" rows={3} value={data} onChange={(e) => setData(e.target.value)} spellCheck={false} />
              </Field>
            ) : null}
            <Button size="lg" className="w-wide" disabled={!resolved || !!amountErr || (!amount.trim() && mode !== 'call') || reviewing || !state} onClick={() => void doReview()}>
              {reviewing ? 'Checking…' : 'Review'}
            </Button>
            {reviewErr ? <Notice tone="alert">{reviewErr}</Notice> : null}
            {state && state.policy.enabled ? (
              <p className="w-fine">
                Instant up to {fmtAmount(state.policy.nativeThreshold)} {config.symbol} per transfer and {fmtAmount(state.policy.dailyLimit)} per day (
                {fmtAmount(state.spentToday)} used today). Larger amounts wait in your Guardian Vault.
              </p>
            ) : null}
          </section>

          <section className={review ? 'w-panel w-review' : 'w-panel w-review is-empty'} aria-live="polite">
            {!review ? (
              <div className="w-review-empty">
                <p>The review appears here.</p>
                <p className="w-fine">You will see who receives what, what your Guardian Vault does with it, and anything that looks wrong.</p>
              </div>
            ) : (
              <>
                <p className="w-label">You send</p>
                <p className="w-review-amount xh-num">
                  {review.call.data !== '0x' && mode === 'call' ? 'Contract call' : `${fmtExact(value, asset?.decimals ?? 18)} ${symbol}`}
                </p>
                <p className="w-label">To</p>
                <div className="w-review-to">
                  {label ? <b>{label}</b> : null}
                  <AddressFull address={resolved?.address ?? (review.call.target as Address)} />
                </div>
                <RecipientChecks check={review.check} name={review.recipient?.name} label={label} />
                {state ? <VaultOutcomeCard outcome={review.vault} state={state} /> : null}
                <PreviewWarnings preview={review.preview} />
                {needsAck ? (
                  <label className="w-ack">
                    <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />
                    <span>{lookAlike ? 'I compared every character of the address with the recipient, and it is correct.' : 'I understand the warnings above.'}</span>
                  </label>
                ) : null}
                {!state?.deployed ? <Notice tone="warn">Activate your account first (Overview).</Notice> : null}
                {rec.kind === 'watch' ? <Notice tone="info">This account is watch-only on this device: there is no passkey to sign with.</Notice> : null}
                {op.phase === 'running' || op.phase === 'error' ? (
                  <OpProgress phase={op.phase} status={op.status} error={op.error} />
                ) : null}
                <Button
                  size="lg"
                  className="w-wide"
                  variant={queued ? 'ghost' : 'primary'}
                  disabled={blocked}
                  onClick={() => void op.run({ calls: [review.call], describe: { kind: 'sent', title: `Sent ${symbol}`, counterparty: resolved?.address, counterpartyName: label, amount: value, symbol, decimals: asset?.decimals ?? 18 } })}
                >
                  {op.phase === 'running' ? 'Waiting…' : queued ? 'Confirm and queue with passkey' : 'Confirm with passkey'}
                </Button>
                {!w.demo ? null : <p className="w-fine">Demo: the passkey prompt is simulated.</p>}
              </>
            )}
          </section>
        </div>
      )}
    </Page>
  )
}

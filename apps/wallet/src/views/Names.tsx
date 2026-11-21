'use client'
// XNS: claim a .xdevs name (look-alike-proof) and set it as the account's primary name.
import { Button } from '@xdevs/ui'
import { useEffect, useState } from 'react'
import { encodeFunctionData } from 'viem'
import { IconCheck, IconTag, IconX } from '../components/icons.tsx'
import { OpProgress, VaultOutcomeCard } from '../components/review.tsx'
import { DemoTag, Notice, Page } from '../components/ui.tsx'
import { config } from '../config.ts'
import { XNS_YEAR } from '../data/live.ts'
import type { Review, XnsQuote } from '../data/types.ts'
import { xnsAbi } from '../lib/abi.ts'
import { XNS_REGISTRY } from '../lib/account.ts'
import { fmtAmount } from '../lib/format.ts'
import { humanError, useOp, useWallet } from '../wallet/WalletProvider.tsx'

const REASONS: Record<string, string> = {
  'too-short': 'At least 3 characters.',
  'too-long': 'At most 63 characters.',
  'forbidden-character': 'Only a–z, 0–9 and "-". Accented, Cyrillic and other look-alike characters are refused on purpose.',
  'bad-hyphen': 'A hyphen cannot start or end the name, or follow another hyphen.',
}

export function Names() {
  const w = useWallet()
  const { rec, state, primaryName } = w
  const [input, setInput] = useState('')
  const [quote, setQuote] = useState<XnsQuote | null>(null)
  const [checking, setChecking] = useState(false)
  const [review, setReview] = useState<Review | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const op = useOp()
  const label = input.trim().toLowerCase().replace(/\.xdevs$/, '')

  useEffect(() => {
    setQuote(null)
    setReview(null)
    if (!label) return
    setChecking(true)
    const t = setTimeout(() => {
      w.backend
        .xnsQuote(label)
        .then(setQuote, (e) => setErr(humanError(e)))
        .finally(() => setChecking(false))
    }, 350)
    return () => clearTimeout(t)
  }, [label, w.backend])

  if (!rec) return null
  const calls = quote?.available
    ? [
        { target: XNS_REGISTRY, value: quote.price, data: encodeFunctionData({ abi: xnsAbi, functionName: 'register', args: [label, rec.address, XNS_YEAR] }) },
        { target: XNS_REGISTRY, value: 0n, data: encodeFunctionData({ abi: xnsAbi, functionName: 'setName', args: [quote.name] }) },
      ]
    : null

  async function doReview() {
    if (!calls || !state) return
    setErr(null)
    try {
      setReview(await w.backend.review(state, calls[0], null, null))
    } catch (e) {
      setErr(humanError(e))
    }
  }

  return (
    <Page title={<>Your .xdevs name {w.demo ? <DemoTag /> : null}</>} sub="A name people can send to instead of a long address. XNS refuses names that look like an existing one." narrow>
      <section className="w-panel w-name-current">
        <span className="w-name-icon">
          <IconTag size={20} />
        </span>
        <div>
          <p className="w-label">Primary name</p>
          <p className="w-name-big">{primaryName ?? <span className="w-muted">none yet</span>}</p>
        </div>
      </section>

      <section className="w-panel w-form">
        <label className="w-label" htmlFor="xns">
          Claim a name
        </label>
        <div className="w-name-input">
          <input id="xns" className="w-input is-mono" value={input} onChange={(e) => setInput(e.target.value)} placeholder="yourname" autoComplete="off" autoCapitalize="off" spellCheck={false} maxLength={70} />
          <span>.xdevs</span>
        </div>
        {label && (checking || !quote) && !err ? <p className="w-field-hint">Checking…</p> : null}
        {quote ? (
          quote.available ? (
            <p className="w-avail is-ok">
              <IconCheck size={14} /> {quote.name} is available · {quote.price === 0n ? 'free on the testnet' : `${fmtAmount(quote.price)} ${config.symbol} per year`}
            </p>
          ) : (
            <p className="w-avail is-no">
              <IconX size={14} /> {quote.name} is not available{quote.reason ? `: ${REASONS[quote.reason] ?? quote.reason}` : ''}
            </p>
          )
        ) : null}
        {err ? <Notice tone="alert">{err}</Notice> : null}
        {quote?.available && !review ? (
          <Button onClick={() => void doReview()} disabled={!state?.deployed || rec.kind === 'watch'}>
            Review claim
          </Button>
        ) : null}
        {review && state && calls ? (
          <>
            <VaultOutcomeCard outcome={review.vault} state={state} />
            <OpProgress phase={op.phase} status={op.status} error={op.error} queued={!!op.result?.queuedIds.length} doneText={op.result?.queuedIds.length ? 'The registration waits in your Guardian Vault and executes when the delay ends, if the name is still free then.' : `${quote?.name} is yours.`} />
            {op.phase === 'idle' || op.phase === 'error' ? (
              <Button disabled={review.vault.kind === 'rejected'} onClick={() => void op.run({ calls, describe: { kind: 'name', title: `Claimed ${quote?.name}` } })}>
                Claim {quote?.name} with passkey
              </Button>
            ) : null}
          </>
        ) : null}
        {!state?.deployed ? <p className="w-fine">Activate your account first to claim a name.</p> : null}
        <ul className="w-name-rules">
          <li>3–63 characters: a–z, 0–9 and single hyphens.</li>
          <li>
            Look-alikes of registered names are refused (0→o, 1→l, rn→m, vv→w, cl→d): if <code>paypal.xdevs</code> is registered, <code>paypa1.xdevs</code> cannot be. A look-alike of a name nobody holds is still available.
          </li>
          <li>Names last a year and anyone can renew them. An expired name stops resolving immediately.</li>
        </ul>
      </section>
    </Page>
  )
}

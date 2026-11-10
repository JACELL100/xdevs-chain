import { useState } from 'react'
import { type Address, type Hex, encodeFunctionData, isHex, parseEther, parseUnits } from 'viem'
import { config, publicClient } from '../config.ts'
import { erc20Abi } from '../lib/abi.ts'
import type { AccountState, Call } from '../lib/account.ts'
import { type Preview, hasHighSeverity, previewTransaction } from '../lib/preview.ts'
import { type RecipientCheckResult, type ResolvedRecipient, checkRecipient, resolveRecipient } from '../lib/recipient.ts'
import { type VaultOutcome, checkVaultOutcome } from '../lib/vault.ts'
import { type AccountRecord, recipientHistory, rememberRecipient } from '../store.ts'
import { Addr, Card, Notice, OpStatus, useUserOp } from './common.tsx'
import { PreviewPanel, VaultBadge } from './PreviewPanel.tsx'

type Mode = 'native' | 'token' | 'call'

type Review = {
  call: Call
  recipient: ResolvedRecipient | null
  check: RecipientCheckResult | null
  preview: Preview
  vault: VaultOutcome
}

/** Builds the review of a set of calls (shared with the approvals screen). */
export async function reviewCall(state: AccountState, call: Call, recipient: ResolvedRecipient | null, check: RecipientCheckResult | null): Promise<Review> {
  const [preview, vault] = await Promise.all([
    previewTransaction(publicClient, { from: state.address, to: call.target, value: call.value, data: call.data }, { recipient: check, balance: state.balance }),
    checkVaultOutcome(publicClient, state, [call]),
  ])
  return { call, recipient, check, preview, vault }
}

export function Send({ rec, state, refresh }: { rec: AccountRecord; state: AccountState | null; refresh: () => void }) {
  const [mode, setMode] = useState<Mode>('native')
  const [to, setTo] = useState('')
  const [amount, setAmount] = useState('')
  const [token, setToken] = useState<string>(config.knownTokens[0] ?? '')
  const [data, setData] = useState('0x')
  const [review, setReview] = useState<Review | null>(null)
  const [ack, setAck] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const op = useUserOp(rec, refresh)

  async function doReview() {
    if (!state) return
    setBusy(true)
    setError(null)
    setReview(null)
    setAck(false)
    try {
      const r = await resolveRecipient(publicClient, to)
      let call: Call
      let payee: Address = r.address
      if (mode === 'native') {
        call = { target: r.address, value: parseEther(amount || '0'), data: '0x' }
      } else if (mode === 'token') {
        if (!token) throw new Error('pick a token')
        const decimals = await publicClient.readContract({ address: token as Address, abi: erc20Abi, functionName: 'decimals' })
        call = { target: token as Address, value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'transfer', args: [r.address, parseUnits(amount || '0', decimals)] }) }
      } else {
        if (!isHex(data)) throw new Error('calldata must be 0x-prefixed hex')
        call = { target: r.address, value: parseEther(amount || '0'), data: data as Hex }
        payee = r.address
      }
      const check = await checkRecipient(publicClient, rec.address, payee, recipientHistory(rec.address))
      setReview(await reviewCall(state, call, r, check))
    } catch (e) {
      setError((e as Error).message.split('\n')[0])
    } finally {
      setBusy(false)
    }
  }

  async function confirm() {
    if (!review) return
    const r = await op.run({ calls: [review.call] })
    if (r && review.recipient) rememberRecipient(rec.address, review.recipient.address)
  }

  const blocked = !review || review.vault.kind === 'rejected' || (hasHighSeverity(review.preview) && !ack) || rec.kind === 'watch' || !state?.deployed
  return (
    <Card title="Send">
      <div className="tabs small">
        {(['native', 'token', 'call'] as Mode[]).map((m) => (
          <button key={m} type="button" className={mode === m ? 'active' : ''} onClick={() => { setMode(m); setReview(null) }}>
            {m === 'native' ? 'tXDS' : m === 'token' ? 'Token' : 'Contract call'}
          </button>
        ))}
      </div>
      <label>
        {mode === 'call' ? 'Contract' : 'Recipient'} — address or name (alice.xdevs)
        <input value={to} onChange={(e) => { setTo(e.target.value); setReview(null) }} placeholder="0x… or alice.xdevs" />
      </label>
      {mode === 'token' && (
        <label>
          Token
          {config.knownTokens.length ? (
            <select value={token} onChange={(e) => setToken(e.target.value)}>
              {config.knownTokens.map((t) => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
          ) : (
            <input value={token} onChange={(e) => setToken(e.target.value)} placeholder="token address" />
          )}
        </label>
      )}
      <label>
        Amount {mode === 'token' ? '(token units)' : '(tXDS)'}
        <input value={amount} onChange={(e) => { setAmount(e.target.value); setReview(null) }} placeholder="0.0" />
      </label>
      {mode === 'call' && (
        <label>
          Calldata (what a dApp asks you to sign)
          <textarea rows={3} value={data} onChange={(e) => { setData(e.target.value); setReview(null) }} />
        </label>
      )}
      <button type="button" className="secondary" disabled={busy || !to || !state} onClick={() => void doReview()}>
        {busy ? 'Checking…' : 'Review'}
      </button>
      {error && <Notice kind="danger">{error}</Notice>}
      {review && (
        <div className="stack">
          {review.recipient?.name && (
            <Notice kind="ok">
              {review.recipient.name} resolves to <Addr a={review.recipient.address} /> ({review.recipient.via === 'node' ? 'node' : 'XNS registry'}).
            </Notice>
          )}
          {review.check && (
            <div className="checks">
              {review.check.lookAlikeOf.length > 0 && (
                <Notice kind="danger">
                  Look-alike address! It shares its first and last characters with {review.check.lookAlikeOf.map((a) => <Addr key={a} a={a} full />)} which you
                  used before, but it is a different address. Address-poisoning scams plant such addresses in your history.
                </Notice>
              )}
              {review.check.firstTime && <Notice kind="warn">First time you send to this address. Double-check it with the recipient.</Notice>}
              {review.check.isContract && <Notice kind="info">The recipient is a contract.</Notice>}
              {review.check.name && !review.recipient?.name && <Notice kind="info">Primary name: {review.check.name}</Notice>}
            </div>
          )}
          <VaultBadge outcome={review.vault} />
          <PreviewPanel preview={review.preview} />
          {hasHighSeverity(review.preview) && (
            <label className="inline">
              <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /> I understand the high-severity warnings above
            </label>
          )}
          <button type="button" disabled={blocked} onClick={() => void confirm()}>
            {review.vault.kind === 'queued' ? 'Sign and queue' : 'Sign and send'}
          </button>
          {!state?.deployed && <Notice kind="warn">Activate the account first (Overview).</Notice>}
          <OpStatus status={op.status} error={op.error} />
        </div>
      )}
    </Card>
  )
}

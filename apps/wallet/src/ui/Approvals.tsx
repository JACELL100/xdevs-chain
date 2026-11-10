import { useCallback, useEffect, useState } from 'react'
import { type Address, encodeFunctionData, formatUnits, getAddress, isAddress, maxUint256, parseUnits } from 'viem'
import { config, publicClient } from '../config.ts'
import { erc20Abi } from '../lib/abi.ts'
import type { AccountState } from '../lib/account.ts'
import { type ApprovalItem, listApprovals, revokeAllCalls, revokeCall } from '../lib/approvals.ts'
import { hasHighSeverity } from '../lib/preview.ts'
import type { AccountRecord } from '../store.ts'
import { Addr, Card, Notice, OpStatus, useUserOp } from './common.tsx'
import { PreviewPanel, VaultBadge } from './PreviewPanel.tsx'
import { reviewCall } from './Send.tsx'

type Review = Awaited<ReturnType<typeof reviewCall>>

export function Approvals({ rec, state, refresh }: { rec: AccountRecord; state: AccountState | null; refresh: () => void }) {
  const [items, setItems] = useState<ApprovalItem[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const op = useUserOp(rec, () => {
    refresh()
    void load()
  })
  const load = useCallback(async () => {
    try {
      setItems(await listApprovals(publicClient, rec.address, { apiUrl: config.apiUrl, knownTokens: config.knownTokens }))
      setError(null)
    } catch (e) {
      setError((e as Error).message.split('\n')[0])
    }
  }, [rec.address])
  useEffect(() => {
    void load()
  }, [load])

  // "Grant approval" builder, to see what the preview and the vault say about it.
  const [token, setToken] = useState<string>(config.knownTokens[0] ?? '')
  const [spender, setSpender] = useState('')
  const [amount, setAmount] = useState('')
  const [unlimited, setUnlimited] = useState(false)
  const [review, setReview] = useState<Review | null>(null)
  const [rErr, setRErr] = useState<string | null>(null)
  const [ack, setAck] = useState(false)

  async function doReview() {
    if (!state) return
    setRErr(null)
    setReview(null)
    setAck(false)
    try {
      if (!isAddress(token, { strict: false }) || !isAddress(spender, { strict: false })) throw new Error('token and spender must be addresses')
      const decimals = await publicClient.readContract({ address: token as Address, abi: erc20Abi, functionName: 'decimals' })
      const value = unlimited ? maxUint256 : parseUnits(amount || '0', decimals)
      const call = { target: getAddress(token), value: 0n, data: encodeFunctionData({ abi: erc20Abi, functionName: 'approve', args: [getAddress(spender), value] }) }
      setReview(await reviewCall(state, call, null, null))
    } catch (e) {
      setRErr((e as Error).message.split('\n')[0])
    }
  }

  const vaultOn = !!state?.policy.enabled
  const readOnly = rec.kind === 'watch' || !state?.deployed
  const source = items?.[0]?.source ?? (config.apiUrl ? 'api' : 'local')
  return (
    <div className="stack">
      <Card
        title="Live approvals"
        actions={<button type="button" className="link" onClick={() => void load()}>refresh</button>}
      >
        <p className="muted">
          Source: {source === 'api' ? 'explorer API' : source === 'node' ? 'node (xdevs_getApprovals)' : `on-chain scan of known tokens (${config.knownTokens.length}) and Permit2Expiring, last ~9,000 blocks`}.
        </p>
        {error && <Notice kind="warn">{error}</Notice>}
        {items && items.length === 0 && <p className="muted">No live approvals found.</p>}
        {items && items.length > 0 && (
          <table>
            <thead>
              <tr><th>Token</th><th>Spender</th><th>Amount</th><th>Expires</th><th /></tr>
            </thead>
            <tbody>
              {items.map((a) => (
                <tr key={`${a.kind}-${a.token}-${a.spender}`}>
                  <td>{a.symbol ?? <Addr a={a.token} />} <span className="badge">{a.kind}</span></td>
                  <td><Addr a={a.spender} /></td>
                  <td className={a.unlimited ? 'sev-high' : ''}>{a.unlimited ? 'UNLIMITED' : formatUnits(a.amount, 18)}</td>
                  <td>{a.expiry ? new Date(a.expiry * 1000).toLocaleDateString() : <span className="sev-high">never</span>}</td>
                  <td>{!readOnly && <button type="button" className="link" onClick={() => void op.run({ calls: [revokeCall(a)] })}>revoke</button>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {items && items.length > 0 && !readOnly && (
          <button type="button" className="danger" onClick={() => void op.run({ calls: revokeAllCalls(items, vaultOn) })}>
            Revoke all ({items.length})
          </button>
        )}
        <p className="muted">
          Revocations are always instant, even with the Guardian Vault on.
          {vaultOn
            ? ' With the vault on, "revoke all" sends one approve(spender, 0) per approval: the one-shot XDRC-20 / Permit2 revokeAll() is not recognised as a revocation by the vault and would be queued.'
            : ' "Revoke all" uses XDRC-20 revokeAll() and Permit2Expiring revokeAll() (one call kills every approval, including signed but unsubmitted permits).'}
        </p>
        <OpStatus status={op.status} error={op.error} />
      </Card>

      <Card title="Grant an approval (see what the preview says)">
        <div className="grid2">
          <label>Token<input value={token} onChange={(e) => setToken(e.target.value)} placeholder="token address" /></label>
          <label>Spender<input value={spender} onChange={(e) => setSpender(e.target.value)} placeholder="0x…" /></label>
          <label>Amount<input value={amount} disabled={unlimited} onChange={(e) => setAmount(e.target.value)} /></label>
          <label className="inline"><input type="checkbox" checked={unlimited} onChange={(e) => setUnlimited(e.target.checked)} /> Unlimited (what many dApps ask for)</label>
        </div>
        <button type="button" className="secondary" disabled={!state} onClick={() => void doReview()}>Review</button>
        {rErr && <Notice kind="danger">{rErr}</Notice>}
        {review && (
          <div className="stack">
            <VaultBadge outcome={review.vault} />
            <PreviewPanel preview={review.preview} />
            {hasHighSeverity(review.preview) && (
              <label className="inline"><input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /> I understand the high-severity warnings</label>
            )}
            <button type="button" disabled={readOnly || review.vault.kind === 'rejected' || (hasHighSeverity(review.preview) && !ack)} onClick={() => void op.run({ calls: [review.call] })}>
              Sign
            </button>
          </div>
        )}
      </Card>
    </div>
  )
}

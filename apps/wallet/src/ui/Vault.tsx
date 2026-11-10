import { useCallback, useEffect, useState } from 'react'
import { type Address, type Hex, decodeFunctionData, formatEther, getAddress, isAddress, isAddressEqual, parseEther } from 'viem'
import { config, publicClient } from '../config.ts'
import { accountAbi, erc20Abi, nftAbi, permit2Abi, xdrc20Abi } from '../lib/abi.ts'
import { type AccountState, type Call, MAX_VAULT_DELAY, MIN_VAULT_DELAY } from '../lib/account.ts'
import {
  type QueuedAction,
  cancelCallData,
  executeQueuedCallData,
  formatCountdown,
  listVaultQueue,
  listVaultQueueFromApi,
  setGuardiansCallData,
  setVaultPolicyCallData,
  setWhitelistedCallData,
} from '../lib/vault.ts'
import type { AccountRecord } from '../store.ts'
import { Addr, Card, Notice, OpStatus, fmtXds, short, useNow, useUserOp } from './common.tsx'
import { parseGuardians } from './Onboarding.tsx'

const anyAbi = [...accountAbi, ...erc20Abi, ...xdrc20Abi, ...nftAbi, ...permit2Abi]

export function describeCall(self: Address, c: Call): string {
  if (c.data === '0x' || c.data.length < 10) return `send ${formatEther(c.value)} tXDS to ${short(c.target)}`
  try {
    const d = decodeFunctionData({ abi: anyAbi, data: c.data as Hex })
    const args = ((d.args ?? []) as readonly unknown[]).map((a) => (typeof a === 'string' && a.startsWith('0x') && a.length === 42 ? short(a) : typeof a === 'object' ? '{…}' : String(a)))
    const where = isAddressEqual(c.target, self) ? 'account setting' : short(c.target)
    return `${d.functionName}(${args.join(', ')}) on ${where}${c.value ? ` + ${formatEther(c.value)} tXDS` : ''}`
  } catch {
    return `call ${c.data.slice(0, 10)} on ${short(c.target)}${c.value ? ` + ${formatEther(c.value)} tXDS` : ''}`
  }
}

/** Open vault actions, polled; used by the alert banner and the vault screen. */
export function useVaultQueue(address: Address | null, deployed: boolean, intervalMs = 5_000) {
  const [queue, setQueue] = useState<QueuedAction[]>([])
  const [error, setError] = useState<string | null>(null)
  const now = useNow(intervalMs)
  const load = useCallback(async () => {
    if (!address || !deployed) return
    try {
      const q = config.apiUrl
        ? await listVaultQueueFromApi(config.apiUrl, address).catch(() => listVaultQueue(publicClient, address, BigInt(Math.floor(Date.now() / 1000))))
        : await listVaultQueue(publicClient, address, BigInt(Math.floor(Date.now() / 1000)))
      setQueue(q)
      setError(null)
    } catch (e) {
      setError((e as Error).message.split('\n')[0])
    }
  }, [address, deployed])
  useEffect(() => {
    void load()
  }, [load, now])
  return { queue, error, reload: load }
}

export function Vault({ rec, state, refresh }: { rec: AccountRecord; state: AccountState | null; refresh: () => void }) {
  const now = useNow()
  const { queue, error: qErr, reload } = useVaultQueue(rec.address, !!state?.deployed)
  const op = useUserOp(rec, () => {
    refresh()
    void reload()
  })
  const p = state?.policy
  const [form, setForm] = useState({ enabled: true, threshold: '', daily: '', delayHours: '', blockUnlimited: true, instantSigs: false })
  const [wl, setWl] = useState('')
  const [gText, setGText] = useState('')
  const [gThr, setGThr] = useState('')
  const [formErr, setFormErr] = useState<string | null>(null)
  const [recovery, setRecovery] = useState<string | null>(null)

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
    setGText(state?.guardians.join('\n') ?? '')
    setGThr(String(state?.guardianThreshold ?? ''))
  }, [p?.enabled, p?.nativeThreshold, p?.dailyLimit, p?.delay, p?.blockUnlimitedApprovals, state?.guardians.length])

  // Social recovery status from the account's events (bounded window: the node caps eth_getLogs at 10k blocks).
  useEffect(() => {
    if (!state?.deployed) return
    let stop = false
    void (async () => {
      try {
        const latest = await publicClient.getBlockNumber()
        const fromBlock = latest > 9_000n ? latest - 9_000n : 0n
        const logs = await publicClient.getContractEvents({ address: rec.address, abi: accountAbi, fromBlock, toBlock: latest })
        const rel = logs.filter((l) => ['RecoveryApproved', 'RecoveryReady', 'RecoveryExecuted', 'RecoveryCancelled'].includes(l.eventName))
        const last = rel[rel.length - 1]
        if (stop) return
        if (!last) setRecovery('No recovery in progress (last ~9,000 blocks).')
        else if (last.eventName === 'RecoveryApproved') setRecovery(`Recovery proposed: ${(last.args as { approvals: number }).approvals} guardian approval(s) so far.`)
        else if (last.eventName === 'RecoveryReady') setRecovery(`Recovery approved by the quorum; executable from ${new Date(Number((last.args as { readyAt: bigint }).readyAt) * 1000).toLocaleString()}. If this is not you, cancel it (owner) — unless the account is frozen.`)
        else if (last.eventName === 'RecoveryExecuted') setRecovery('A recovery was executed: the signer set was replaced.')
        else setRecovery('The last recovery was cancelled by the owner.')
      } catch (e) {
        if (!stop) setRecovery(`Recovery status unavailable: ${(e as Error).message.split('\n')[0]}`)
      }
    })()
    return () => {
      stop = true
    }
  }, [rec.address, state?.deployed, state?.lastActionId])

  if (!state) return <Card title="Guardian Vault">Loading…</Card>
  if (!state.deployed) return <Card title="Guardian Vault"><Notice kind="info">Activate the account first (Overview).</Notice></Card>
  const readOnly = rec.kind === 'watch'

  function submitPolicy() {
    setFormErr(null)
    try {
      const delay = BigInt(Math.round(Number(form.delayHours) * 3600))
      if (form.enabled && (delay < MIN_VAULT_DELAY || delay > MAX_VAULT_DELAY)) throw new Error('delay must be between 1 hour and 7 days')
      void op.run({
        callData: setVaultPolicyCallData({
          enabled: form.enabled,
          blockUnlimitedApprovals: form.blockUnlimited,
          allowInstantSignatures: form.instantSigs,
          delay,
          nativeThreshold: parseEther(form.threshold || '0'),
          dailyLimit: parseEther(form.daily || '0'),
          dailyGasBudget: p?.dailyGasBudget || parseEther('1'),
        }),
      })
    } catch (e) {
      setFormErr((e as Error).message)
    }
  }

  function submitGuardians() {
    setFormErr(null)
    try {
      const g = parseGuardians(gText)
      const t = g.length ? Number(gThr) : 0
      if (g.length && (!Number.isInteger(t) || t < 1 || t > g.length)) throw new Error(`threshold must be 1..${g.length}`)
      void op.run({ callData: setGuardiansCallData(g, t) })
    } catch (e) {
      setFormErr((e as Error).message)
    }
  }

  const open = queue.filter((a) => a.status === 'queued' || a.status === 'ready')
  const delayed = p?.enabled ? `Delayed by ${formatCountdown(p.delay)} (queued, cancellable).` : 'Applies immediately (vault is off).'
  return (
    <div className="stack">
      <Card title={`Queued actions (${open.length} open)`} actions={<button type="button" className="link" onClick={() => void reload()}>refresh</button>}>
        {qErr && <Notice kind="warn">{qErr}</Notice>}
        {queue.length === 0 && <p className="muted">Nothing queued.</p>}
        <ul className="queue">
          {queue.map((a) => (
            <li key={String(a.id)} className={`q-${a.status}`}>
              <div>
                <b>#{String(a.id)}</b> <span className="badge">{a.status}</span>{' '}
                {(a.status === 'queued' || a.status === 'ready') && <span>{a.status === 'queued' ? `executable in ${formatCountdown(a.executeAfter - now)}` : 'executable now'}</span>}
                {a.cancelVotes > 0 && <span className="muted"> · guardian cancel votes {a.cancelVotes}/{state.guardianThreshold}</span>}
              </div>
              <ul className="calls">
                {a.calls.map((c, i) => (
                  <li key={i}>{describeCall(rec.address, c)}</li>
                ))}
              </ul>
              {!readOnly && (a.status === 'queued' || a.status === 'ready') && (
                <div className="row">
                  <button type="button" className="danger" disabled={state.frozen} onClick={() => void op.run({ callData: cancelCallData(a.id) })}>
                    Cancel (not me!)
                  </button>
                  {a.status === 'ready' && (
                    <button type="button" className="secondary" disabled={state.frozen} onClick={() => void op.run({ calls: [{ target: rec.address, value: 0n, data: executeQueuedCallData(a.id) }] })}>
                      Execute
                    </button>
                  )}
                </div>
              )}
            </li>
          ))}
        </ul>
        {state.frozen && <Notice kind="warn">While frozen only the guardian quorum can cancel (Guardian tools tab).</Notice>}
        <OpStatus status={op.status} error={op.error} />
      </Card>

      <Card title="Policy">
        <div className="kv">
          <span>Status</span><b>{p?.enabled ? 'ON' : 'OFF'}</b>
          <span>Instant limit per transfer</span><span>{fmtXds(p?.nativeThreshold ?? 0n)} tXDS</span>
          <span>Instant limit per day</span><span>{fmtXds(p?.dailyLimit ?? 0n)} tXDS (spent today {fmtXds(state.spentToday)})</span>
          <span>Delay</span><span>{formatCountdown(p?.delay ?? 0n)}</span>
          <span>Unlimited approvals</span><span>{p?.blockUnlimitedApprovals ? 'blocked' : 'queued'}</span>
          <span>Off-chain signatures (ERC-1271)</span><span>{p?.allowInstantSignatures ? 'instant (permits possible!)' : 'only pre-approved hashes'}</span>
          <span>Daily gas budget</span><span>{fmtXds(p?.dailyGasBudget ?? 0n)} tXDS</span>
        </div>
        {!readOnly && (
          <details>
            <summary>Change policy</summary>
            <Notice kind="info">{delayed} A thief who steals your passkey therefore cannot switch protection off quickly.</Notice>
            <div className="grid2">
              <label className="inline"><input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /> Vault enabled</label>
              <label className="inline"><input type="checkbox" checked={form.blockUnlimited} onChange={(e) => setForm({ ...form, blockUnlimited: e.target.checked })} /> Block unlimited approvals</label>
              <label>Instant limit per transfer (tXDS)<input value={form.threshold} onChange={(e) => setForm({ ...form, threshold: e.target.value })} /></label>
              <label>Instant limit per day (tXDS)<input value={form.daily} onChange={(e) => setForm({ ...form, daily: e.target.value })} /></label>
              <label>Delay (hours, 1–168)<input value={form.delayHours} onChange={(e) => setForm({ ...form, delayHours: e.target.value })} /></label>
              <label className="inline"><input type="checkbox" checked={form.instantSigs} onChange={(e) => setForm({ ...form, instantSigs: e.target.checked })} /> Allow instant off-chain signatures (not recommended)</label>
            </div>
            <button type="button" onClick={submitPolicy}>Submit policy change</button>
          </details>
        )}
      </Card>

      <Card title="Whitelist (trusted recipients / contracts / spenders)">
        {state.whitelist.length === 0 && <p className="muted">Empty.</p>}
        <ul>
          {state.whitelist.map((w) => (
            <li key={w}>
              <Addr a={w} />{' '}
              {!readOnly && <button type="button" className="link" onClick={() => void op.run({ callData: setWhitelistedCallData(w, false) })}>remove (delayed)</button>}
            </li>
          ))}
        </ul>
        {!readOnly && (
          <div className="row">
            <input value={wl} onChange={(e) => setWl(e.target.value)} placeholder="0x… address to trust" />
            <button type="button" className="secondary" disabled={!isAddress(wl.trim(), { strict: false })} onClick={() => void op.run({ callData: setWhitelistedCallData(getAddress(wl.trim()), true) })}>
              Add
            </button>
          </div>
        )}
        <p className="muted">Whitelisted addresses get instant transfers, approvals and calls with any data — whitelists are trust. {delayed}</p>
      </Card>

      <Card title="Guardians, freeze and recovery">
        <div className="kv">
          <span>Guardians</span>
          <span>{state.guardians.length ? state.guardians.map((g) => <div key={g}><Addr a={g} /></div>) : 'none — no recovery, no freeze, no guardian cancels'}</span>
          <span>Quorum</span><span>{state.guardianThreshold} of {state.guardians.length}</span>
          <span>Recovery delay</span><span>{formatCountdown(state.recoveryDelay)}</span>
          <span>Freeze</span>
          <span>
            {state.frozen ? `FROZEN until ${new Date(Number(state.frozenUntil) * 1000).toLocaleString()}` : 'not frozen'}
            {state.nextFreezeAllowedAt > now ? ` · next freeze allowed in ${formatCountdown(state.nextFreezeAllowedAt - now)}` : ''}
          </span>
          <span>Social recovery</span><span>{recovery ?? '…'}</span>
        </div>
        <p className="muted">
          A guardian quorum can cancel queued actions, freeze the account (up to {formatCountdown(state.freezeDuration || 30n * 86_400n)}, ≤ 60 days) and
          recover it to new keys after the recovery delay. A single guardian below the quorum can do nothing alone.
        </p>
        {!readOnly && (
          <details>
            <summary>Change guardians</summary>
            <textarea rows={3} value={gText} onChange={(e) => setGText(e.target.value)} />
            <label className="inline">Quorum <input className="narrow" value={gThr} onChange={(e) => setGThr(e.target.value)} /></label>
            <Notice kind="info">{delayed}</Notice>
            <button type="button" onClick={submitGuardians}>Submit guardian change</button>
          </details>
        )}
        {formErr && <Notice kind="danger">{formErr}</Notice>}
      </Card>
    </div>
  )
}

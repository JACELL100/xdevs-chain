import { useState } from 'react'
import { type Address, getAddress, isAddress, parseEther, toHex } from 'viem'
import { config, publicClient } from '../config.ts'
import { type InitParams, buildInitParams, counterfactualAddress, recommendedPolicy, saltFrom, VAULT_OFF } from '../lib/account.ts'
import { SoftPasskey, createPasskey, passkeySignerStruct } from '../lib/signers.ts'
import type { P256PublicKey } from '../lib/webauthn.ts'
import { type AccountRecord, addAccount } from '../store.ts'
import { Card, Notice } from './common.tsx'

type VaultForm = { enabled: boolean; threshold: string; daily: string; delayHours: string; blockUnlimited: boolean }

export function parseGuardians(text: string): Address[] {
  return text
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => {
      if (!isAddress(s, { strict: false })) throw new Error(`not an address: ${s}`)
      return getAddress(s)
    })
}

export function Onboarding({ onReady }: { onReady: (rec: AccountRecord) => void }) {
  const [name, setName] = useState('')
  const [guardians, setGuardians] = useState('')
  const [threshold, setThreshold] = useState('2')
  const [vault, setVault] = useState<VaultForm>({ enabled: true, threshold: '10', daily: '50', delayHours: '24', blockUnlimited: true })
  const [watch, setWatch] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const hasWebAuthn = typeof window !== 'undefined' && !!window.PublicKeyCredential

  function params(key: P256PublicKey): InitParams {
    const g = parseGuardians(guardians)
    const t = Number(threshold)
    if (g.length > 0 && (!Number.isInteger(t) || t < 1 || t > g.length)) throw new Error(`guardian threshold must be 1..${g.length}`)
    const delay = BigInt(Math.round(Number(vault.delayHours) * 3600))
    return buildInitParams({
      signers: [passkeySignerStruct(key)],
      guardians: g,
      guardianThreshold: g.length ? t : 0,
      policy: vault.enabled
        ? recommendedPolicy({
            nativeThreshold: parseEther(vault.threshold || '0'),
            dailyLimit: parseEther(vault.daily || '0'),
            delay,
            blockUnlimitedApprovals: vault.blockUnlimited,
          })
        : VAULT_OFF,
    })
  }

  async function create(kind: 'passkey' | 'virtual') {
    setBusy(true)
    setError(null)
    try {
      const label = name.trim() || 'My xDEVS account'
      let rec: Omit<AccountRecord, 'address' | 'params' | 'salt'>
      let key: P256PublicKey
      if (kind === 'passkey') {
        const p = await createPasskey(label)
        key = p.publicKey
        rec = { label, kind, passkey: p, createdAt: Date.now() }
      } else {
        const soft = await SoftPasskey.generate(location.hostname, location.origin)
        key = soft.publicKey
        rec = { label: `${label} (virtual passkey, dev)`, kind, virtualJwk: await soft.exportJwk(), createdAt: Date.now() }
      }
      const p = params(key)
      const salt = saltFrom(BigInt(toHex(crypto.getRandomValues(new Uint8Array(8)))))
      const address = await counterfactualAddress(publicClient, p, salt)
      const full: AccountRecord = { ...rec, address, params: p, salt }
      addAccount(full)
      onReady(full)
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  function addWatch() {
    try {
      if (!isAddress(watch.trim(), { strict: false })) throw new Error('not an address')
      const rec: AccountRecord = { address: getAddress(watch.trim()), label: 'Watch-only', kind: 'watch', createdAt: Date.now() }
      addAccount(rec)
      onReady(rec)
    } catch (e) {
      setError((e as Error).message)
    }
  }

  return (
    <div className="stack">
      <Card title="Create an account — no seed phrase">
        <p>
          Your account is an xDEVS smart account controlled by a <b>passkey</b> (Face ID, Touch ID, Windows Hello or a
          security key). There is no seed phrase to write down or lose. The account address is known right away; the
          contract is created by your first transaction{config.paymasterSponsor ? ', whose gas is sponsored' : ''}.
        </p>
        <label>
          Account name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="My xDEVS account" />
        </label>
        <fieldset>
          <legend>Guardians (optional, recommended)</legend>
          <p className="muted">
            Friends, family or your other devices. A quorum of them can cancel a suspicious queued transfer, freeze the
            account and recover it to a new passkey. Be aware: a quorum is trusted for safety too (it can recover the
            account), so pick people who will not collude.
          </p>
          <textarea rows={3} value={guardians} onChange={(e) => setGuardians(e.target.value)} placeholder="0x… one per line" />
          <label className="inline">
            Quorum <input className="narrow" value={threshold} onChange={(e) => setThreshold(e.target.value)} /> of the guardians
          </label>
        </fieldset>
        <fieldset>
          <legend>
            <label className="inline">
              <input type="checkbox" checked={vault.enabled} onChange={(e) => setVault({ ...vault, enabled: e.target.checked })} /> Guardian Vault
            </label>
          </legend>
          {vault.enabled && (
            <div className="grid2">
              <label>
                Instant limit per transfer (tXDS)
                <input value={vault.threshold} onChange={(e) => setVault({ ...vault, threshold: e.target.value })} />
              </label>
              <label>
                Instant limit per day (tXDS)
                <input value={vault.daily} onChange={(e) => setVault({ ...vault, daily: e.target.value })} />
              </label>
              <label>
                Delay for everything else (hours, 1–168)
                <input value={vault.delayHours} onChange={(e) => setVault({ ...vault, delayHours: e.target.value })} />
              </label>
              <label className="inline">
                <input type="checkbox" checked={vault.blockUnlimited} onChange={(e) => setVault({ ...vault, blockUnlimited: e.target.checked })} />
                Block unlimited token approvals
              </label>
            </div>
          )}
          <p className="muted">
            Larger transfers, approvals and calls to unknown contracts wait out the delay, so a thief with your device
            cannot drain the account instantly. Turning the vault off later is itself delayed.
          </p>
        </fieldset>
        <div className="row">
          <button type="button" disabled={busy || !hasWebAuthn} onClick={() => void create('passkey')}>
            {busy ? 'Working…' : 'Create with passkey'}
          </button>
          {config.devTools && (
            <button type="button" className="secondary" disabled={busy} onClick={() => void create('virtual')}>
              Virtual passkey (dev only)
            </button>
          )}
        </div>
        {!hasWebAuthn && <Notice kind="warn">This browser has no WebAuthn support.</Notice>}
        {config.devTools && (
          <p className="muted">
            Dev only: the virtual passkey is a P-256 key kept in this browser's storage. It signs exactly like a real
            passkey but is only as safe as a hot key. Never use it with real value.
          </p>
        )}
        {error && <Notice kind="danger">{error}</Notice>}
      </Card>
      <Card title="Watch an existing account">
        <div className="row">
          <input value={watch} onChange={(e) => setWatch(e.target.value)} placeholder="0x… smart account or EIP-7702 EOA" />
          <button type="button" className="secondary" onClick={addWatch}>
            Watch
          </button>
        </div>
        <p className="muted">Read-only: see balance, vault policy, queue, guardians, freeze and EOA-lock state.</p>
      </Card>
    </div>
  )
}

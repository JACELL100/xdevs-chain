'use client'
// Create a wallet with a passkey (no seed phrase), or sign in with an existing passkey.
import { AddressChip, Button } from '@xdevs/ui'
import Link from 'next/link'
import { useRouter } from 'next/navigation'
import { useState } from 'react'
import { type Address, getAddress, isAddress, toHex } from 'viem'
import { IconCheck, IconClock, IconKey, IconShield, IconUsers } from '../components/icons.tsx'
import { PasskeyVisual, type PasskeyState } from '../components/PasskeyVisual.tsx'
import { AddressFull, DemoTag, Field, Notice, Toggle } from '../components/ui.tsx'
import { config, explorerAddress, faucetLink, rpId } from '../config.ts'
import { type InitParams, VAULT_OFF, buildInitParams, recommendedPolicy, saltFrom } from '../lib/account.ts'
import { fmtDuration, parseAmount } from '../lib/format.ts'
import { discoverPasskey, matchPasskeySigner, rpIdMatchesHost } from '../lib/passkey-auth.ts'
import { SoftPasskey, createPasskey, passkeySignerStruct } from '../lib/signers.ts'
import type { P256PublicKey } from '../lib/webauthn.ts'
import type { AccountRecord } from '../store.ts'
import { humanError, useWallet } from '../wallet/WalletProvider.tsx'

type Step = 'intro' | 'create' | 'creating' | 'created' | 'signin' | 'signin-account'
type VaultForm = { enabled: boolean; threshold: string; daily: string; delayHours: string; blockUnlimited: boolean }

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

function passkeyEnvironmentProblem(): string | null {
  if (typeof window === 'undefined') return null
  if (!window.isSecureContext) return 'Passkeys need a secure page (HTTPS, or localhost while developing).'
  if (!window.PublicKeyCredential) return 'This browser does not support passkeys. Try a recent Safari, Chrome, Edge or Firefox.'
  if (!rpIdMatchesHost(rpId(), location.hostname)) return `This page (${location.hostname}) cannot use passkeys for ${rpId()}. Check NEXT_PUBLIC_RP_ID.`
  return null
}

export function Onboarding() {
  const w = useWallet()
  const router = useRouter()
  const [step, setStep] = useState<Step>('intro')
  const [label, setLabel] = useState('')
  const [vault, setVault] = useState<VaultForm>({ enabled: true, threshold: '10', daily: '50', delayHours: '24', blockUnlimited: true })
  const [custom, setCustom] = useState(false)
  const [guardiansText, setGuardiansText] = useState('')
  const [quorum, setQuorum] = useState('2')
  const [error, setError] = useState<string | null>(null)
  const [pk, setPk] = useState<PasskeyState>('idle')
  const [created, setCreated] = useState<AccountRecord | null>(null)
  const [signinKey, setSigninKey] = useState<Awaited<ReturnType<typeof discoverPasskey>> | null>(null)
  const [who, setWho] = useState('')
  const [busy, setBusy] = useState(false)

  async function guardianList(): Promise<Address[]> {
    const parts = guardiansText
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter(Boolean)
    const out: Address[] = []
    for (const p of parts) {
      if (isAddress(p, { strict: false })) out.push(getAddress(p))
      else out.push((await w.backend.resolveRecipient(p)).address)
    }
    if (new Set(out.map((a) => a.toLowerCase())).size !== out.length) throw new Error('A guardian is listed twice.')
    return out
  }

  async function buildParams(key: P256PublicKey): Promise<InitParams> {
    const g = await guardianList()
    const q = Number(quorum)
    if (g.length && (!Number.isInteger(q) || q < 1 || q > g.length)) throw new Error(`Choose a quorum between 1 and ${g.length}.`)
    const hours = Number(vault.delayHours)
    if (vault.enabled && (!Number.isFinite(hours) || hours < 1 || hours > 168)) throw new Error('The vault delay must be between 1 and 168 hours.')
    return buildInitParams({
      signers: [passkeySignerStruct(key)],
      guardians: g,
      guardianThreshold: g.length ? q : 0,
      policy: vault.enabled
        ? recommendedPolicy({
            nativeThreshold: parseAmount(vault.threshold || '0'),
            dailyLimit: parseAmount(vault.daily || '0'),
            delay: BigInt(Math.round(hours * 3600)),
            blockUnlimitedApprovals: vault.blockUnlimited,
          })
        : VAULT_OFF,
    })
  }

  async function create(kind: 'passkey' | 'virtual') {
    setError(null)
    setBusy(true)
    setStep('creating')
    setPk('scanning')
    try {
      const name = label.trim() || 'My xDEVS account'
      if (w.demo) {
        await sleep(1_600)
        setPk('done')
        setCreated({ address: w.rec?.address ?? ('0x' as Address), label: name, kind: 'passkey', createdAt: Date.now() })
        setStep('created')
        return
      }
      let key: P256PublicKey
      let base: Omit<AccountRecord, 'address'>
      if (kind === 'passkey') {
        const problem = passkeyEnvironmentProblem()
        if (problem) throw new Error(problem)
        const p = await createPasskey(name, 'xDEVS Wallet', rpId())
        key = p.publicKey
        base = { label: name, kind: 'passkey', passkey: p, createdAt: Date.now() }
      } else {
        const soft = await SoftPasskey.generate(rpId(), location.origin)
        key = soft.publicKey
        base = { label: `${name} (virtual passkey, dev)`, kind: 'virtual', virtualJwk: await soft.exportJwk(), createdAt: Date.now() }
      }
      const params = await buildParams(key)
      const salt = saltFrom(BigInt(toHex(crypto.getRandomValues(new Uint8Array(8)))))
      const address = await w.backend.counterfactualAddress(params, salt)
      const rec: AccountRecord = { ...base, address, params, salt }
      w.addAndSelect(rec)
      setCreated(rec)
      setPk('done')
      setStep('created')
    } catch (e) {
      setPk('error')
      setError(humanError(e))
      setStep('create')
    } finally {
      setBusy(false)
    }
  }

  async function signIn() {
    setError(null)
    setBusy(true)
    setPk('scanning')
    try {
      if (w.demo) {
        await sleep(1_200)
        setPk('done')
        router.push('/')
        return
      }
      const problem = passkeyEnvironmentProblem()
      if (problem) throw new Error(problem)
      const a = await discoverPasskey(rpId())
      const i = w.persisted.accounts.findIndex((r) => r.passkey?.credentialId === a.credentialId)
      if (i >= 0) {
        w.selectAccount(i)
        setPk('done')
        router.push('/')
        return
      }
      setSigninKey(a)
      setPk('done')
      setStep('signin-account')
    } catch (e) {
      setPk('error')
      setError(humanError(e))
    } finally {
      setBusy(false)
    }
  }

  async function linkAccount() {
    if (!signinKey) return
    setError(null)
    setBusy(true)
    try {
      const r = await w.backend.resolveRecipient(who)
      const st = await w.backend.readAccount(r.address)
      if (!st.deployed)
        throw new Error('This account is not activated yet, so its passkey cannot be checked on-chain. Open the wallet on the device where you created it, or activate it there first.')
      const key = await matchPasskeySigner(st.signers, signinKey)
      if (!key) throw new Error('This passkey is not a signer of that account. Check the address, or use the passkey you created it with.')
      w.addAndSelect({
        address: r.address,
        label: r.name ?? 'My xDEVS account',
        kind: 'passkey',
        passkey: { credentialId: signinKey.credentialId, publicKey: key, rpId: signinKey.rpId, createdAt: Date.now() },
        createdAt: Date.now(),
      })
      router.push('/')
    } catch (e) {
      setError(humanError(e))
    } finally {
      setBusy(false)
    }
  }

  function watchOnly() {
    try {
      if (!isAddress(who.trim(), { strict: false })) throw new Error('Enter a 0x address to watch.')
      w.addAndSelect({ address: getAddress(who.trim()), label: 'Watch-only', kind: 'watch', createdAt: Date.now() })
      router.push('/')
    } catch (e) {
      setError(humanError(e))
    }
  }

  const summary = vault.enabled
    ? `Instant up to ${vault.threshold || 0} ${config.symbol} per transfer and ${vault.daily || 0} per day. Everything larger waits ${fmtDuration(Math.round(Number(vault.delayHours || 0) * 3600))}.`
    : 'Off: every transaction executes instantly.'

  if (step === 'intro')
    return (
      <div className="xu-page w-page">
        <section className="w-hero">
          <div className="w-hero-copy">
            <p className="w-eyebrow">xDEVS Wallet {w.demo ? <DemoTag /> : null}</p>
            <h1 className="w-hero-title">
              A wallet with
              <br />
              no seed phrase.
            </h1>
            <p className="w-hero-lede">
              Your account is unlocked with Face ID, a fingerprint or Windows Hello. Large transfers wait in your Guardian
              Vault, so a stolen device can&apos;t drain you.
            </p>
            <div className="xh-ctas">
              <Button size="lg" onClick={() => setStep('create')}>
                Create wallet
              </Button>
              <Button size="lg" variant="ghost" onClick={() => setStep('signin')}>
                Sign in with passkey
              </Button>
            </div>
            <ul className="w-hero-points">
              <li>
                <IconKey /> Passkey only, nothing to write down
              </li>
              <li>
                <IconClock /> Big transfers wait out a cancel window
              </li>
              <li>
                <IconUsers /> Guardians can freeze and recover
              </li>
            </ul>
          </div>
          <div className="w-hero-visual" aria-hidden="true">
            <div className="w-device">
              <div className="w-device-notch" />
              <PasskeyVisual state="scanning" size={150} />
              <p className="w-device-title">Create a passkey for</p>
              <p className="w-device-host">wallet.xdevschain.tech</p>
              <div className="w-device-chip">
                <IconShield size={14} /> Guardian Vault on · 24 h
              </div>
            </div>
          </div>
        </section>
      </div>
    )

  if (step === 'signin' || step === 'signin-account')
    return (
      <div className="xu-page w-page is-narrow">
        <section className="w-flow">
          <PasskeyVisual state={pk} size={112} />
          {step === 'signin' ? (
            <>
              <h1 className="w-flow-title">Sign in with your passkey</h1>
              <p className="w-flow-lede">Your device will ask for Face ID, your fingerprint or your PIN. Nothing is signed or sent.</p>
              {error ? <Notice tone="alert">{error}</Notice> : null}
              <div className="w-flow-actions">
                <Button size="lg" onClick={() => void signIn()} disabled={busy}>
                  {busy ? 'Waiting for your passkey…' : 'Continue with passkey'}
                </Button>
                <Button variant="quiet" onClick={() => setStep('intro')}>
                  Back
                </Button>
              </div>
            </>
          ) : (
            <>
              <h1 className="w-flow-title">Which account is this passkey for?</h1>
              <p className="w-flow-lede">
                This device has no record of the account yet. Enter its address or .xdevs name: the wallet checks on-chain that
                your passkey is one of its signers.
              </p>
              <Field label="Account address or name" htmlFor="who">
                <input id="who" className="w-input" value={who} onChange={(e) => setWho(e.target.value)} placeholder="0x… or yourname.xdevs" autoComplete="off" spellCheck={false} />
              </Field>
              {error ? <Notice tone="alert">{error}</Notice> : null}
              <div className="w-flow-actions">
                <Button size="lg" onClick={() => void linkAccount()} disabled={busy || !who.trim()}>
                  {busy ? 'Checking…' : 'Verify and sign in'}
                </Button>
                <Button variant="quiet" onClick={watchOnly} disabled={!isAddress(who.trim(), { strict: false })}>
                  Watch read-only instead
                </Button>
              </div>
            </>
          )}
        </section>
      </div>
    )

  if (step === 'created' && created)
    return (
      <div className="xu-page w-page is-narrow">
        <section className="w-flow">
          <PasskeyVisual state="done" size={112} />
          <h1 className="w-flow-title">Your wallet is ready</h1>
          <p className="w-flow-lede">
            No seed phrase was created, and none ever will be. Your passkey is the key{config.devTools && created.kind === 'virtual' ? ' (here: a dev-only virtual passkey)' : ''}.
          </p>
          <div className="w-created-addr">
            <span className="w-label">Your address</span>
            <AddressFull address={created.address} />
            <AddressChip address={created.address} href={explorerAddress(created.address)} />
          </div>
          <ul className="w-done-list">
            <li>
              <IconCheck /> It can receive {config.symbol} right away.
            </li>
            <li>
              <IconCheck /> The account contract is created by your first transaction{config.paymasterSponsor ? ', and its gas is sponsored' : ''}.
            </li>
            <li>
              <IconCheck /> {vault.enabled ? `Guardian Vault on: ${summary}` : 'Guardian Vault off. You can turn it on later, instantly.'}
            </li>
          </ul>
          <div className="w-flow-actions">
            <Button size="lg" href="/" linkComponent={Link}>
              Open my wallet
            </Button>
            <Button size="lg" variant="ghost" href={faucetLink(created.address)} external>
              Get test {config.symbol}
            </Button>
          </div>
        </section>
      </div>
    )

  // create / creating
  return (
    <div className="xu-page w-page is-narrow">
      <section className="w-flow is-left">
        <div className="w-flow-head">
          <PasskeyVisual state={pk} size={72} />
          <div>
            <h1 className="w-flow-title">Create your wallet</h1>
            <p className="w-flow-lede">One passkey, two decisions. You can change both later.</p>
          </div>
        </div>
        <Field label="Account name" hint="Only stored on this device, and in your passkey's label." htmlFor="label">
          <input id="label" className="w-input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="My xDEVS account" maxLength={48} autoComplete="off" />
        </Field>

        <div className="w-choice">
          <Toggle
            checked={vault.enabled}
            onChange={(v) => setVault({ ...vault, enabled: v })}
            label={
              <>
                <IconShield size={15} /> Guardian Vault <span className="w-reco">recommended</span>
              </>
            }
            hint={summary}
          />
          {vault.enabled ? (
            <>
              <button type="button" className="w-disclose" aria-expanded={custom} onClick={() => setCustom((v) => !v)}>
                {custom ? 'Hide limits' : 'Customize limits'}
              </button>
              {custom ? (
                <div className="w-grid2">
                  <Field label={`Instant per transfer (${config.symbol})`} htmlFor="thr">
                    <input id="thr" className="w-input" inputMode="decimal" value={vault.threshold} onChange={(e) => setVault({ ...vault, threshold: e.target.value })} />
                  </Field>
                  <Field label={`Instant per day (${config.symbol})`} htmlFor="day">
                    <input id="day" className="w-input" inputMode="decimal" value={vault.daily} onChange={(e) => setVault({ ...vault, daily: e.target.value })} />
                  </Field>
                  <Field label="Delay for larger actions (hours)" hint="1 to 168 hours" htmlFor="delay">
                    <input id="delay" className="w-input" inputMode="numeric" value={vault.delayHours} onChange={(e) => setVault({ ...vault, delayHours: e.target.value })} />
                  </Field>
                  <Toggle checked={vault.blockUnlimited} onChange={(v) => setVault({ ...vault, blockUnlimited: v })} label="Block unlimited token approvals" />
                </div>
              ) : null}
            </>
          ) : (
            <Notice tone="warn">Without the vault, anyone holding your unlocked device can move everything at once.</Notice>
          )}
        </div>

        <div className="w-choice">
          <p className="w-choice-title">
            <IconUsers size={15} /> Guardians <span className="w-muted">optional</span>
          </p>
          <p className="w-choice-text">
            People or devices you trust. A quorum of them can cancel a suspicious transfer, freeze the account and recover it
            to a new passkey. Adding them now is instant; changing them later waits out the vault delay.
          </p>
          <Field label="Guardian addresses or .xdevs names" hint="One per line. You can skip this and add them later." htmlFor="guardians">
            <textarea id="guardians" className="w-input" rows={3} value={guardiansText} onChange={(e) => setGuardiansText(e.target.value)} placeholder={'theo.xdevs\n0x…'} spellCheck={false} />
          </Field>
          {guardiansText.trim() ? (
            <Field label="Quorum" hint="How many guardians must agree" htmlFor="quorum">
              <input id="quorum" className="w-input is-narrow" inputMode="numeric" value={quorum} onChange={(e) => setQuorum(e.target.value)} />
            </Field>
          ) : null}
        </div>

        {error ? <Notice tone="alert">{error}</Notice> : null}
        <div className="w-flow-actions">
          <Button size="lg" onClick={() => void create('passkey')} disabled={busy}>
            {step === 'creating' ? 'Waiting for your passkey…' : 'Create passkey'}
          </Button>
          {config.devTools && !w.demo ? (
            <Button variant="ghost" onClick={() => void create('virtual')} disabled={busy}>
              Virtual passkey (dev only)
            </Button>
          ) : null}
          <Button variant="quiet" onClick={() => setStep('intro')} disabled={busy}>
            Back
          </Button>
        </div>
        <p className="w-fine">
          The passkey&apos;s private key never leaves your device&apos;s secure hardware. The wallet stores only its public key
          and your account address in this browser.
        </p>
      </section>
    </div>
  )
}

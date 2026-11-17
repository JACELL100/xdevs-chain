'use client'
// MetaMask users: keep your address, add the protections. EIP-7702 delegation to the xDEVS Smart Account plus the
// XIP-0001 EOA lock, so the old private key can no longer bypass the Guardian Vault with raw transactions.
import { AddressChip, Badge, Button, cx } from '@xdevs/ui'
import { useState } from 'react'
import { type Address, type Hex, type WalletClient, encodeFunctionData, parseEther } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { IconCheck, IconKey, IconLock, IconShield } from '../components/icons.tsx'
import { DemoTag, Field, KV, Notice, Page, SectionTitle } from '../components/ui.tsx'
import { config, explorerAddress, publicClient } from '../config.ts'
import { DEMO_EOA, demoEnableEoaLock } from '../data/demo.ts'
import { accountAbi } from '../lib/abi.ts'
import { type AccountState, buildInitParams, recommendedPolicy } from '../lib/account.ts'
import { enableEoaLock, upgradeEoa } from '../lib/eip7702.ts'
import { fmtAmount } from '../lib/format.ts'
import { connectInjected, sendAndWait } from '../lib/injected.ts'
import { ecdsaSignerStruct } from '../lib/signers.ts'
import { humanError, useWallet } from '../wallet/WalletProvider.tsx'

export function EoaLock() {
  const w = useWallet()
  const [me, setMe] = useState<{ address: Address; wallet: WalletClient | null } | null>(null)
  const [st, setSt] = useState<AccountState | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [msg, setMsg] = useState<{ tone: 'ok' | 'alert' | 'info'; text: string } | null>(null)

  async function read(a: Address) {
    setSt(await w.backend.readAccount(a))
  }

  async function connect() {
    setMsg(null)
    setBusy('connect')
    try {
      if (w.demo) {
        setMe({ address: DEMO_EOA, wallet: null })
        await read(DEMO_EOA)
      } else {
        const c = await connectInjected()
        setMe(c)
        await read(c.address)
      }
    } catch (e) {
      setMsg({ tone: 'alert', text: humanError(e) })
    } finally {
      setBusy(null)
    }
  }

  async function lock() {
    if (!me || !st) return
    setBusy('lock')
    setMsg({ tone: 'info', text: w.demo ? 'Confirm in your wallet… (simulated)' : 'Confirm the transaction in your wallet…' })
    try {
      if (w.demo) await demoEnableEoaLock()
      else if (me.wallet) await sendAndWait(me.wallet, me.address, encodeFunctionData({ abi: accountAbi, functionName: 'enableEoaLock' }), { lockSafe: true, gas: 1_000_000n })
      await read(me.address)
      setMsg({ tone: 'ok', text: 'EOA lock on. Your old key can now only act through the account, under the vault rules.' })
    } catch (e) {
      setMsg({ tone: 'alert', text: humanError(e) })
    } finally {
      setBusy(null)
    }
  }

  const delegated = !!st?.is7702
  const lockStatus = st?.eoaLock?.status ?? null
  const step = !st ? 0 : !delegated ? 1 : !st.policy.enabled ? 2 : lockStatus === 'unlocked' ? 3 : 4

  return (
    <Page
      title={<>Protect a MetaMask address {w.demo ? <DemoTag /> : null}</>}
      sub="Keep your existing address and history, and add the Guardian Vault, guardians and recovery. The EOA lock (XIP-0001) then stops the old key from going around them."
    >
      <ol className="w-steps-big">
        {[
          { t: 'Delegate', d: 'EIP-7702 points your address at the xDEVS Smart Account (0x…1005). Same address, same balance.', icon: <IconKey /> },
          { t: 'Turn on the vault', d: 'Large transfers and risky actions wait out a delay you and your guardians can cancel.', icon: <IconShield /> },
          { t: 'Lock the old key', d: 'XIP-0001: the chain refuses raw transactions from the key that skip the account.', icon: <IconLock /> },
        ].map((s, i) => (
          <li key={s.t} className={cx(step > i + 1 && 'is-done', step === i + 1 && 'is-now')}>
            <span className="w-steps-icon">{step > i + 1 ? <IconCheck /> : s.icon}</span>
            <b>
              {i + 1}. {s.t}
            </b>
            <p>{s.d}</p>
          </li>
        ))}
      </ol>

      <div className="w-dash-grid">
        <section className="w-panel">
          <SectionTitle>Your address</SectionTitle>
          {!me ? (
            <div className="w-form">
              <p className="w-fine">Connect MetaMask (or any browser wallet) to see where this address stands. Connecting is read-only.</p>
              <Button onClick={() => void connect()} disabled={busy === 'connect'}>
                {busy === 'connect' ? 'Connecting…' : 'Connect MetaMask'}
              </Button>
            </div>
          ) : st ? (
            <div className="w-form">
              <KV
                rows={[
                  ['Address', <AddressChip key="a" address={st.address} href={explorerAddress(st.address)} />],
                  ['Balance', `${fmtAmount(st.balance)} ${config.symbol}`],
                  ['Delegation', delegated ? <Badge tone="final">xDEVS Smart Account</Badge> : st.deployed ? <Badge tone="warn">other code</Badge> : <Badge tone="dim">plain EOA</Badge>],
                  ['Guardian Vault', delegated ? (st.policy.enabled ? <Badge tone="final">on</Badge> : <Badge tone="warn">off</Badge>) : '—'],
                  [
                    'EOA lock',
                    lockStatus ? (
                      <Badge tone={lockStatus === 'unlocked' ? 'warn' : 'final'} dot>
                        {lockStatus}
                        {st.eoaLock?.protocolActive === false ? ' (not enforced here)' : ''}
                      </Badge>
                    ) : (
                      '—'
                    ),
                  ],
                ]}
              />
              {!delegated ? (
                <Notice tone="warn" title="MetaMask can't do step 1 yet">
                  Wallets only sign EIP-7702 authorizations for their own delegation contract today, not for the xDEVS Smart Account. Until they do, create a
                  passkey wallet instead (strongest: no private key at all), or upgrade on a dev chain with the panel below.
                </Notice>
              ) : !st.policy.enabled ? (
                <Notice tone="info">Turn on the Guardian Vault first: the lock needs it, because the vault is what the key is held to.</Notice>
              ) : lockStatus === 'unlocked' ? (
                <>
                  <Notice tone="warn" title="Your old key can still bypass the vault">
                    Until the lock is on, a thief with the MetaMask key could sign raw transfers that never go through your account.
                  </Notice>
                  <Button onClick={() => void lock()} disabled={!!busy}>
                    {busy === 'lock' ? 'Waiting for your wallet…' : 'Turn on the EOA lock'}
                  </Button>
                  <p className="w-fine">Instant. Turning it off later waits out the vault delay, and your guardians can cancel that.</p>
                </>
              ) : (
                <Notice tone="ok" title={lockStatus === 'sealed' ? 'Sealed' : 'Locked'}>
                  {lockStatus === 'sealed'
                    ? 'The address is sealed (frozen by guardians, or the key was revoked by a recovery): the old key cannot send anything.'
                    : 'The old key can only make value-free calls into its own account, where the vault rules apply.'}
                </Notice>
              )}
              {msg ? <Notice tone={msg.tone}>{msg.text}</Notice> : null}
            </div>
          ) : (
            <p className="w-muted">Reading…</p>
          )}
          {!me && msg ? <Notice tone={msg.tone}>{msg.text}</Notice> : null}
        </section>

        <section className="w-panel">
          <SectionTitle>What the lock allows</SectionTitle>
          <ul className="w-lock-table">
            <li>
              <Badge tone="warn" dot>
                unlocked
              </Badge>
              <p>Everything, like any Ethereum address. The key can skip your vault.</p>
            </li>
            <li>
              <Badge tone="final" dot>
                locked
              </Badge>
              <p>Only value-free calls to itself (one per block, ≤ 2M gas, ≤ 2× base fee). They enter your account, where the vault applies.</p>
            </li>
            <li>
              <Badge tone="alert" dot>
                sealed
              </Badge>
              <p>Nothing at all. Automatic while guardians freeze the account, or after a recovery revoked the key.</p>
            </li>
          </ul>
          <p className="w-fine">
            Enforced by the {config.chainName} nodes at block validity, mempool and RPC (error -32003 XIP0001_*). It never changes finalized blocks.
          </p>
        </section>
      </div>

      {config.devTools && !w.demo ? <DevUpgrade /> : null}
    </Page>
  )
}

/** DEV ONLY: EIP-7702 upgrade with a raw private key (dev chains), because wallets cannot sign it yet. */
function DevUpgrade() {
  const { addAndSelect } = useWallet()
  const [key, setKey] = useState('')
  const [st, setSt] = useState<AccountState | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const valid = /^0x[0-9a-fA-F]{64}$/.test(key)

  async function run(label: string, f: () => Promise<unknown>) {
    setMsg(`${label}…`)
    try {
      await f()
      setMsg(`${label}: done`)
      const { readAccountState } = await import('../lib/account.ts')
      setSt(await readAccountState(publicClient, privateKeyToAccount(key as Hex).address))
    } catch (e) {
      setMsg(`${label}: ${humanError(e)}`)
    }
  }

  return (
    <section className="w-panel w-dev">
      <SectionTitle>Dev panel: upgrade an EOA with a raw key</SectionTitle>
      <Notice tone="alert">Development chains only. Never paste a key that holds real value. The key stays in this tab.</Notice>
      <Field label="Private key" htmlFor="dev-key">
        <input id="dev-key" type="password" className="w-input is-mono" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" />
      </Field>
      <div className="w-flow-actions is-left">
        <Button
          size="sm"
          disabled={!valid}
          onClick={() =>
            void run('EIP-7702 upgrade', () =>
              upgradeEoa(
                publicClient,
                config.rpcUrl,
                key as Hex,
                buildInitParams({ signers: [ecdsaSignerStruct(privateKeyToAccount(key as Hex).address)], policy: recommendedPolicy({ nativeThreshold: parseEther('10'), dailyLimit: parseEther('50') }) }),
              ),
            )
          }
        >
          Upgrade (vault on)
        </Button>
        <Button size="sm" variant="ghost" disabled={!valid} onClick={() => void run('Enable EOA lock', () => enableEoaLock(publicClient, config.rpcUrl, key as Hex))}>
          Enable lock
        </Button>
        <Button
          size="sm"
          variant="quiet"
          disabled={!valid || !st?.is7702}
          onClick={() => addAndSelect({ address: privateKeyToAccount(key as Hex).address, label: 'EIP-7702 account (dev key)', kind: 'dev-key', devKey: key as Hex, createdAt: Date.now() })}
        >
          Use in the wallet
        </Button>
      </div>
      {msg ? <p className="w-fine">{msg}</p> : null}
    </section>
  )
}

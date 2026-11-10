import { useState } from 'react'
import { type Address, type Hex, parseEther } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { config, publicClient } from '../config.ts'
import { type AccountState, buildInitParams, readAccountState, recommendedPolicy } from '../lib/account.ts'
import { enableEoaLock, upgradeEoa } from '../lib/eip7702.ts'
import { ecdsaSignerStruct } from '../lib/signers.ts'
import { type AccountRecord, addAccount } from '../store.ts'
import { Addr, Card, Notice, fmtXds } from './common.tsx'
import { connectInjected } from './eoa.ts'
import { parseGuardians } from './Onboarding.tsx'
import { LockBadge } from './Overview.tsx'

function StatusRows({ s }: { s: AccountState }) {
  return (
    <div className="kv">
      <span>Address</span><Addr a={s.address} full />
      <span>Balance</span><span>{fmtXds(s.balance)} tXDS</span>
      <span>Delegation</span><span>{s.is7702 ? 'EIP-7702 → xDEVS Smart Account (0x…1005)' : s.deployed ? 'has other code' : 'plain EOA (not upgraded)'}</span>
      {s.is7702 && (
        <>
          <span>Guardian Vault</span><span>{s.policy.enabled ? 'on' : 'off'}</span>
          <span>EOA lock (XIP-0001)</span><span><LockBadge state={s} /> {!s.eoaKeyActive && '· EOA key revoked by a recovery'}</span>
        </>
      )}
    </div>
  )
}

export function Eip7702({ onUse }: { onUse: (rec: AccountRecord) => void }) {
  const [mm, setMm] = useState<AccountState | null>(null)
  const [key, setKey] = useState('')
  const [dev, setDev] = useState<AccountState | null>(null)
  const [guardians, setGuardians] = useState('')
  const [quorum, setQuorum] = useState('2')
  const [msg, setMsg] = useState<{ kind: 'ok' | 'danger' | 'info'; text: string } | null>(null)
  const validKey = /^0x[0-9a-fA-F]{64}$/.test(key)

  async function refreshDev() {
    if (!validKey) return
    setDev(await readAccountState(publicClient, privateKeyToAccount(key as Hex).address))
  }

  async function run(label: string, f: () => Promise<unknown>) {
    setMsg({ kind: 'info', text: `${label}…` })
    try {
      await f()
      setMsg({ kind: 'ok', text: `${label}: done` })
      await refreshDev()
    } catch (e) {
      setMsg({ kind: 'danger', text: `${label}: ${(e as Error).message.split('\n')[0]}` })
    }
  }

  return (
    <div className="stack">
      <Card title="Keep your MetaMask address, add the protections">
        <p>
          With <b>EIP-7702</b> an existing address (EOA) can point its code at the xDEVS Smart Account and keep its
          address and balance. It then gets the Guardian Vault, guardians, recovery and passkey signers. But its old
          private key can still sign plain transactions that skip all of that. The <b>EOA lock (XIP-0001)</b> closes that
          gap at the protocol level: once enabled, the chain only accepts transactions from that key that go
          through the account itself, so the vault rules apply to the key too. While frozen, or after a recovery revoked
          the key, the address is <b>sealed</b>: the old key cannot send anything at all.
        </p>
        <Notice kind="warn">
          MetaMask and most wallets only sign EIP-7702 authorizations for their own delegation contract, not for the xDEVS
          Smart Account at 0x…1005. Until wallets support it, the upgrade below needs the raw private key (dev panel).
          Connecting MetaMask here is read-only.
        </Notice>
        <button type="button" className="secondary" onClick={() => void connectInjected().then(async ({ address }) => setMm(await readAccountState(publicClient, address))).catch((e) => setMsg({ kind: 'danger', text: (e as Error).message }))}>
          Connect MetaMask (read-only)
        </button>
        {mm && (
          <>
            <StatusRows s={mm} />
            <button type="button" className="link" onClick={() => { const rec: AccountRecord = { address: mm.address, label: 'MetaMask (watch-only)', kind: 'watch', createdAt: Date.now() }; addAccount(rec); onUse(rec) }}>
              Watch this address in the wallet
            </button>
          </>
        )}
      </Card>

      {config.devTools && (
        <Card title="Dev panel: upgrade an EOA with a raw key (DEV ONLY)">
          <Notice kind="danger">Development chains only. Never paste a key that holds real value. The key stays in this browser.</Notice>
          <input type="password" value={key} onChange={(e) => { setKey(e.target.value); setDev(null) }} placeholder="0x… private key" />
          <button type="button" className="secondary" disabled={!validKey} onClick={() => void refreshDev()}>Load</button>
          {dev && <StatusRows s={dev} />}
          {dev && !dev.is7702 && (
            <>
              <label>Guardians (one per line)<textarea rows={3} value={guardians} onChange={(e) => setGuardians(e.target.value)} /></label>
              <label className="inline">Quorum <input className="narrow" value={quorum} onChange={(e) => setQuorum(e.target.value)} /></label>
              <button
                type="button"
                onClick={() =>
                  void run('EIP-7702 upgrade + initialize', async () => {
                    const g = parseGuardians(guardians)
                    const params = buildInitParams({
                      signers: [ecdsaSignerStruct(privateKeyToAccount(key as Hex).address as Address)],
                      guardians: g,
                      guardianThreshold: g.length ? Number(quorum) : 0,
                      policy: recommendedPolicy({ nativeThreshold: parseEther('10'), dailyLimit: parseEther('50') }),
                    })
                    await upgradeEoa(publicClient, config.rpcUrl, key as Hex, params)
                  })
                }
              >
                Upgrade (type-4 tx: delegate + initialize with the vault on)
              </button>
            </>
          )}
          {dev?.is7702 && dev.eoaLock?.status === 'unlocked' && (
            <button type="button" disabled={!dev.policy.enabled} onClick={() => void run('enable EOA lock', () => enableEoaLock(publicClient, config.rpcUrl, key as Hex))}>
              Enable EOA lock (instant)
            </button>
          )}
          {dev?.is7702 && (
            <button type="button" className="secondary" onClick={() => { const rec: AccountRecord = { address: dev.address, label: 'EIP-7702 account (dev key)', kind: 'dev-key', devKey: key as Hex, createdAt: Date.now() }; addAccount(rec); onUse(rec) }}>
              Use this account in the wallet
            </button>
          )}
          <p className="muted">
            Disabling the lock later is a vault-delayed action, cancellable by you or the guardian quorum. After the lock,
            the key can still act through the account (value-0 self-calls, ≤ 2M gas, ≤ 2× base fee, one per block) and
            this wallet uses UserOperations through the bundler, which never come from the EOA.
          </p>
        </Card>
      )}
      {msg && <Notice kind={msg.kind}>{msg.text}</Notice>}
    </div>
  )
}

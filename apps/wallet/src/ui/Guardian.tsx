import { useState } from 'react'
import { type Address, type Hex, type WalletClient, encodeFunctionData, getAddress, isAddress, isHex, pad } from 'viem'
import { config, publicClient } from '../config.ts'
import { accountAbi } from '../lib/abi.ts'
import { type AccountState, readAccountState } from '../lib/account.ts'
import { type SignerStruct, ecdsaSignerStruct } from '../lib/signers.ts'
import { type QueuedAction, formatCountdown, listVaultQueue } from '../lib/vault.ts'
import { Addr, Card, Notice, useNow } from './common.tsx'
import { connectInjected, devKeyWallet, sendAndWait } from './eoa.ts'
import { describeCall } from './Vault.tsx'

/** Parses "0x<address>" (ECDSA signer) or "x,y" (passkey public key) per line. */
function parseSigners(text: string): SignerStruct[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => {
      if (isAddress(l, { strict: false })) return ecdsaSignerStruct(getAddress(l))
      const [x, y] = l.split(/[\s,]+/)
      if (isHex(x) && isHex(y)) return { kind: 2, x: pad(x as Hex), y: pad(y as Hex) }
      throw new Error(`not a signer: ${l}`)
    })
}

export function GuardianTools() {
  const now = useNow()
  const [guardian, setGuardian] = useState<{ address: Address; wallet: WalletClient } | null>(null)
  const [devKey, setDevKey] = useState('')
  const [target, setTarget] = useState('')
  const [state, setState] = useState<AccountState | null>(null)
  const [queue, setQueue] = useState<QueuedAction[]>([])
  const [signers, setSigners] = useState('')
  const [msg, setMsg] = useState<{ kind: 'ok' | 'danger' | 'info'; text: string } | null>(null)

  async function load() {
    setMsg(null)
    try {
      if (!isAddress(target.trim(), { strict: false })) throw new Error('enter the protected account address')
      const a = getAddress(target.trim())
      const s = await readAccountState(publicClient, a)
      setState(s)
      setQueue(s.deployed ? await listVaultQueue(publicClient, a, BigInt(Math.floor(Date.now() / 1000))) : [])
    } catch (e) {
      setMsg({ kind: 'danger', text: (e as Error).message.split('\n')[0] })
    }
  }

  async function act(label: string, data: Hex) {
    if (!guardian || !state) return
    setMsg({ kind: 'info', text: `${label}: confirm in your wallet…` })
    try {
      await sendAndWait(guardian.wallet, state.address, data)
      setMsg({ kind: 'ok', text: `${label}: done` })
      await load()
    } catch (e) {
      setMsg({ kind: 'danger', text: `${label}: ${(e as Error).message.split('\n')[0]}` })
    }
  }

  const isGuardian = !!guardian && !!state?.guardians.some((g) => g.toLowerCase() === guardian.address.toLowerCase())
  const open = queue.filter((q) => q.status === 'queued' || q.status === 'ready')
  return (
    <div className="stack">
      <Card title="Act as a guardian">
        <p>
          Guardians protect someone else's account. Each guardian action below is a <b>vote</b>; it takes effect when
          the account's quorum is reached. Guardians are ordinary addresses (MetaMask, hardware wallet, another xDEVS
          account) and pay their own gas.
        </p>
        {!guardian ? (
          <div className="row">
            <button type="button" onClick={() => void connectInjected().then(setGuardian).catch((e) => setMsg({ kind: 'danger', text: (e as Error).message }))}>
              Connect MetaMask
            </button>
            {config.devTools && (
              <>
                <input type="password" value={devKey} onChange={(e) => setDevKey(e.target.value)} placeholder="dev private key 0x… (DEV ONLY)" />
                <button type="button" className="secondary" disabled={!/^0x[0-9a-fA-F]{64}$/.test(devKey)} onClick={() => setGuardian(devKeyWallet(devKey as Hex))}>
                  Use dev key
                </button>
              </>
            )}
          </div>
        ) : (
          <p>Guardian: <Addr a={guardian.address} /> {state && (isGuardian ? <span className="badge ok">guardian of this account</span> : <span className="badge warn">not a guardian of this account</span>)}</p>
        )}
        <div className="row">
          <input value={target} onChange={(e) => setTarget(e.target.value)} placeholder="protected account 0x…" />
          <button type="button" className="secondary" onClick={() => void load()}>Load</button>
        </div>
        {msg && <Notice kind={msg.kind}>{msg.text}</Notice>}
      </Card>
      {state && (
        <>
          <Card title="Queued actions — cancel (quorum vote)">
            <p className="muted">Quorum: {state.guardianThreshold} of {state.guardians.length}.</p>
            {open.length === 0 && <p className="muted">Nothing open.</p>}
            <ul className="queue">
              {open.map((a) => (
                <li key={String(a.id)}>
                  <b>#{String(a.id)}</b> {a.status === 'queued' ? `executable in ${formatCountdown(a.executeAfter - now)}` : 'executable NOW'} · votes {a.cancelVotes}/{state.guardianThreshold}
                  <ul className="calls">{a.calls.map((c, i) => <li key={i}>{describeCall(state.address, c)}</li>)}</ul>
                  <button type="button" className="danger" disabled={!isGuardian} onClick={() => void act(`cancel #${a.id}`, encodeFunctionData({ abi: accountAbi, functionName: 'cancel', args: [a.id] }))}>
                    Vote to cancel
                  </button>
                </li>
              ))}
            </ul>
          </Card>
          <Card title="Freeze">
            <p>
              {state.frozen ? `Frozen until ${new Date(Number(state.frozenUntil) * 1000).toLocaleString()}.` : 'Not frozen.'}{' '}
              {state.nextFreezeAllowedAt > now && `Next freeze possible in ${formatCountdown(state.nextFreezeAllowedAt - now)} (anti-hostage cooldown).`}
            </p>
            <p className="muted">
              Freezing stops every signer of the account (including a thief) until the quorum unfreezes it, a recovery
              completes, or it expires (≤ 60 days). On an EIP-7702 account it also SEALS the address at the protocol level.
            </p>
            <div className="row">
              <button type="button" className="danger" disabled={!isGuardian || state.frozen} onClick={() => void act('freeze vote', encodeFunctionData({ abi: accountAbi, functionName: 'freeze' }))}>Vote to freeze</button>
              <button type="button" className="secondary" disabled={!isGuardian || !state.frozen} onClick={() => void act('unfreeze vote', encodeFunctionData({ abi: accountAbi, functionName: 'unfreeze' }))}>Vote to unfreeze</button>
            </div>
          </Card>
          <Card title="Social recovery">
            <p className="muted">
              New signer set, one per line: an address (secp256k1 key) or a passkey public key "x,y". Approvals are per
              exact set; at the quorum the recovery becomes executable after {formatCountdown(state.recoveryDelay)} by anyone.
            </p>
            <textarea rows={3} value={signers} onChange={(e) => setSigners(e.target.value)} />
            <div className="row">
              <button type="button" disabled={!isGuardian} onClick={() => { try { void act('approve recovery', encodeFunctionData({ abi: accountAbi, functionName: 'approveRecovery', args: [parseSigners(signers)] })) } catch (e) { setMsg({ kind: 'danger', text: (e as Error).message }) } }}>Approve recovery</button>
              <button type="button" className="secondary" disabled={!guardian} onClick={() => { try { void act('execute recovery', encodeFunctionData({ abi: accountAbi, functionName: 'executeRecovery', args: [parseSigners(signers)] })) } catch (e) { setMsg({ kind: 'danger', text: (e as Error).message }) } }}>Execute (when ready)</button>
            </div>
          </Card>
        </>
      )}
    </div>
  )
}

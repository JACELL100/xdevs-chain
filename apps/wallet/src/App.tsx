import { useEffect, useState } from 'react'
import { config, publicClient } from './config.ts'
import { type AccountRecord, load, save } from './store.ts'
import { Approvals } from './ui/Approvals.tsx'
import { Addr, Notice, short, useAccountState } from './ui/common.tsx'
import { Eip7702 } from './ui/Eip7702.tsx'
import { GuardianTools } from './ui/Guardian.tsx'
import { Onboarding } from './ui/Onboarding.tsx'
import { Overview } from './ui/Overview.tsx'
import { Send } from './ui/Send.tsx'
import { Vault, useVaultQueue } from './ui/Vault.tsx'
import { formatCountdown } from './lib/vault.ts'

type Tab = 'overview' | 'send' | 'vault' | 'approvals' | 'guardian' | 'eoa' | 'new'
const TABS: [Tab, string][] = [
  ['overview', 'Overview'],
  ['send', 'Send'],
  ['vault', 'Guardian Vault'],
  ['approvals', 'Approvals'],
  ['guardian', 'Guardian tools'],
  ['eoa', 'MetaMask / EIP-7702'],
]

function useChainCheck() {
  const [msg, setMsg] = useState<string | null>(null)
  useEffect(() => {
    publicClient
      .getChainId()
      .then((id) => setMsg(id === config.chainId ? null : `RPC chain id ${id} differs from VITE_CHAIN_ID ${config.chainId}`))
      .catch((e) => setMsg(`RPC ${config.rpcUrl} unreachable: ${(e as Error).message.split('\n')[0]}`))
  }, [])
  return msg
}

export function App() {
  const [persisted, setPersisted] = useState(load)
  const [tab, setTab] = useState<Tab>(persisted.active >= 0 ? 'overview' : 'new')
  const rec: AccountRecord | null = persisted.accounts[persisted.active] ?? null
  const { state, error, refresh } = useAccountState(rec?.address ?? null)
  const { queue } = useVaultQueue(rec?.address ?? null, !!state?.deployed)
  const chainMsg = useChainCheck()
  const open = queue.filter((a) => a.status === 'queued' || a.status === 'ready')

  function select(i: number) {
    const p = { ...load(), active: i }
    save(p)
    setPersisted(p)
    setTab('overview')
  }

  function onReady() {
    setPersisted(load())
    setTab('overview')
  }

  return (
    <div className="app">
      <header className="top">
        <div className="brand">
          <b>xDEVS Wallet</b> <span className="badge">{config.chainId === 7117 ? 'devnet' : `chain ${config.chainId}`} · testnet reference</span>
        </div>
        <div className="accounts">
          <select value={persisted.active} onChange={(e) => (Number(e.target.value) < 0 ? setTab('new') : select(Number(e.target.value)))}>
            {persisted.accounts.map((a, i) => (
              <option key={a.address} value={i}>
                {a.label} · {short(a.address)}
              </option>
            ))}
            <option value={-1}>{persisted.accounts.length ? '+ add account' : 'no account yet'}</option>
          </select>
          <button type="button" className="secondary" onClick={() => setTab('new')}>
            New
          </button>
        </div>
      </header>
      {chainMsg && <Notice kind="danger">{chainMsg}</Notice>}
      {error && <Notice kind="warn">{error}</Notice>}
      {open.length > 0 && tab !== 'vault' && (
        <Notice kind="warn">
          <b>Alert:</b> {open.length} action{open.length > 1 ? 's are' : ' is'} waiting in your Guardian Vault (next executable in{' '}
          {formatCountdown(open.reduce((m, a) => (a.executeAfter < m ? a.executeAfter : m), open[0].executeAfter) - BigInt(Math.floor(Date.now() / 1000)))}). Not you?{' '}
          <button type="button" className="link" onClick={() => setTab('vault')}>
            Review and cancel
          </button>
        </Notice>
      )}
      <nav className="tabs">
        {TABS.map(([t, label]) => (
          <button key={t} type="button" className={tab === t ? 'active' : ''} disabled={!rec && t !== 'guardian' && t !== 'eoa'} onClick={() => setTab(t)}>
            {label}
          </button>
        ))}
      </nav>
      <main>
        {tab === 'new' && <Onboarding onReady={onReady} />}
        {tab === 'overview' && rec && <Overview rec={rec} state={state} refresh={refresh} />}
        {tab === 'send' && rec && <Send rec={rec} state={state} refresh={refresh} />}
        {tab === 'vault' && rec && <Vault rec={rec} state={state} refresh={refresh} />}
        {tab === 'approvals' && rec && <Approvals rec={rec} state={state} refresh={refresh} />}
        {tab === 'guardian' && <GuardianTools />}
        {tab === 'eoa' && <Eip7702 onUse={onReady} />}
      </main>
      <footer>
        {rec && (
          <span>
            Active: <Addr a={rec.address} />
          </span>
        )}
        <span>
          Testnet reference wallet. Testnet tokens have no value. Previews and warnings are advice, not fact. Bundler:{' '}
          {config.bundlerUrl}
        </span>
      </footer>
    </div>
  )
}

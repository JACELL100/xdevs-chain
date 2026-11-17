'use client'
// App-wide wallet state: backend (live or demo), chain status, the active account and its polled state / vault queue.
import { type ReactNode, createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'
import type { Address } from 'viem'
import { config } from '../config.ts'
import { DEMO_ACCOUNT, demoBackend } from '../data/demo.ts'
import { liveBackend } from '../data/live.ts'
import type { Backend, ChainStatus, OpInput, OpResult } from '../data/types.ts'
import type { AccountState } from '../lib/account.ts'
import type { QueuedAction } from '../lib/vault.ts'
import { type AccountRecord, type Persisted, addAccount, forgetAccount, load, setActive } from '../store.ts'

const DEMO_SESSION_KEY = 'xdevs-wallet:demo'

/** Runtime demo opt-in ("Preview with demo data" on the launching-soon screen, or ?demo=1). Session only. */
function runtimeDemo(): boolean {
  try {
    if (typeof window === 'undefined') return false
    const q = new URLSearchParams(window.location.search).get('demo')
    if (q === '1') sessionStorage.setItem(DEMO_SESSION_KEY, '1')
    if (q === '0') sessionStorage.removeItem(DEMO_SESSION_KEY)
    return sessionStorage.getItem(DEMO_SESSION_KEY) === '1'
  } catch {
    return false
  }
}

export const DEMO_RECORD: AccountRecord = {
  address: DEMO_ACCOUNT,
  label: 'Demo account',
  kind: 'passkey',
  passkey: { credentialId: 'demo', publicKey: { x: '0x01', y: '0x02' }, rpId: 'demo', createdAt: 0 },
  createdAt: 0,
}

type Ctx = {
  ready: boolean
  backend: Backend
  demo: boolean
  setDemo: (on: boolean) => void
  chain: ChainStatus
  recheckChain: () => void
  persisted: Persisted
  rec: AccountRecord | null
  state: AccountState | null
  stateError: string | null
  queue: QueuedAction[]
  /** Verified XNS primary name of the active account, if any. */
  primaryName: string | null
  refresh: () => Promise<void>
  selectAccount: (i: number) => void
  addAndSelect: (rec: AccountRecord) => void
  forget: (address: Address) => void
}

const WalletCtx = createContext<Ctx | null>(null)

export function useWallet(): Ctx {
  const c = useContext(WalletCtx)
  if (!c) throw new Error('useWallet outside <WalletProvider>')
  return c
}

export function WalletProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false)
  const [demo, setDemoState] = useState(config.demo)
  const [persisted, setPersisted] = useState<Persisted>({ accounts: [], active: -1, history: {} })
  const [chain, setChain] = useState<ChainStatus>({ kind: 'checking' })
  const [state, setState] = useState<AccountState | null>(null)
  const [stateError, setStateError] = useState<string | null>(null)
  const [queue, setQueue] = useState<QueuedAction[]>([])
  const [primaryName, setPrimaryName] = useState<string | null>(null)
  const backend = demo ? demoBackend : liveBackend

  // Client-only boot (static export: the server render has no storage).
  useEffect(() => {
    setDemoState(config.demo || runtimeDemo())
    setPersisted(load())
    setReady(true)
  }, [])

  const setDemo = useCallback((on: boolean) => {
    try {
      if (on) sessionStorage.setItem(DEMO_SESSION_KEY, '1')
      else sessionStorage.removeItem(DEMO_SESSION_KEY)
    } catch {
      /* ignore */
    }
    setDemoState(on || config.demo)
    setState(null)
    setQueue([])
  }, [])

  const rec: AccountRecord | null = demo ? DEMO_RECORD : (persisted.accounts[persisted.active] ?? null)

  const recheckChain = useCallback(() => {
    setChain({ kind: 'checking' })
    void backend.chainStatus().then(setChain)
  }, [backend])

  useEffect(() => {
    if (!ready) return
    let stop = false
    const check = () => void backend.chainStatus().then((s) => !stop && setChain(s))
    check()
    const t = setInterval(check, 30_000)
    return () => {
      stop = true
      clearInterval(t)
    }
  }, [ready, backend])

  const address = rec?.address ?? null
  const chainOk = chain.kind === 'ok'
  const busy = useRef(false)
  const refresh = useCallback(async () => {
    if (!address || !chainOk || busy.current) return
    busy.current = true
    try {
      const s = await backend.readAccount(address)
      setState(s)
      setStateError(null)
      setQueue(s.deployed ? await backend.vaultQueue(address) : [])
    } catch (e) {
      setStateError((e as Error).message.split('\n')[0])
    } finally {
      busy.current = false
    }
  }, [address, chainOk, backend])

  useEffect(() => {
    setState(null)
    setQueue([])
    setPrimaryName(null)
    if (!address || !chainOk) return
    void backend.primaryName(address).then(setPrimaryName, () => setPrimaryName(null))
    void refresh()
    const t = setInterval(() => void refresh(), 6_000)
    return () => clearInterval(t)
  }, [address, chainOk, refresh, backend])

  const value = useMemo<Ctx>(
    () => ({
      ready,
      backend,
      demo,
      setDemo,
      chain,
      recheckChain,
      persisted,
      rec,
      state,
      stateError,
      queue,
      primaryName,
      refresh,
      selectAccount: (i) => setPersisted(setActive(i)),
      addAndSelect: (r) => setPersisted(addAccount(r)),
      forget: (a) => setPersisted(forgetAccount(a)),
    }),
    [ready, backend, demo, setDemo, chain, recheckChain, persisted, rec, state, stateError, queue, primaryName, refresh],
  )
  return <WalletCtx.Provider value={value}>{children}</WalletCtx.Provider>
}

export type OpPhase = 'idle' | 'running' | 'done' | 'error'

/** Runs a UserOperation for the active account, with status text for the UI. */
export function useOp(onDone?: (r: OpResult) => void) {
  const { backend, rec, refresh } = useWallet()
  const [phase, setPhase] = useState<OpPhase>('idle')
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<OpResult | null>(null)
  const running = useRef(false)
  const run = useCallback(
    async (input: OpInput): Promise<OpResult | null> => {
      if (!rec || running.current) return null
      running.current = true
      setPhase('running')
      setError(null)
      setResult(null)
      try {
        const r = await backend.sendOp(rec, input, setStatus)
        setResult(r)
        setPhase('done')
        setStatus(r.queuedIds.length ? `Waiting in your Guardian Vault as action #${r.queuedIds.join(', #')}` : 'Confirmed')
        await refresh()
        onDone?.(r)
        return r
      } catch (e) {
        setPhase('error')
        setStatus(null)
        setError(humanError(e))
        return null
      } finally {
        running.current = false
      }
    },
    [rec, backend, refresh, onDone],
  )
  const reset = useCallback(() => {
    setPhase('idle')
    setStatus(null)
    setError(null)
    setResult(null)
  }, [])
  return { run, phase, status, error, result, reset }
}

/** Short, human error text (no stack traces, never any key material). */
export function humanError(e: unknown): string {
  const name = (e as { name?: string }).name
  if (name === 'NotAllowedError') return 'The passkey prompt was cancelled or timed out.'
  if (name === 'InvalidStateError') return 'This passkey already exists on this device.'
  if (name === 'SecurityError') return 'Passkeys need a secure (HTTPS) page on the wallet domain.'
  const msg = ((e as { shortMessage?: string }).shortMessage ?? (e as Error)?.message ?? String(e)).split('\n')[0]
  if (/fetch failed|Failed to fetch|NetworkError|did not answer/i.test(msg)) return 'The network is not reachable right now. Nothing was sent.'
  return msg
}

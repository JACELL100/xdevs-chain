import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { type Address, type Hex, formatEther } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { config, publicClient } from '../config.ts'
import { type AccountState, type Call, ENTRY_POINT, callsToCallData, readAccountState, SMART_ACCOUNT_FACTORY, factoryData } from '../lib/account.ts'
import { sendUserOp, type SentUserOp } from '../lib/send.ts'
import { type OwnerSigner, SoftPasskey, ecdsaSigner, platformPasskeySigner } from '../lib/signers.ts'
import type { AccountRecord } from '../store.ts'

export const short = (a: string) => `${a.slice(0, 6)}…${a.slice(-4)}`

export function fmtXds(v: bigint, digits = 4): string {
  const s = formatEther(v)
  const [i, f = ''] = s.split('.')
  const frac = f.slice(0, digits).replace(/0+$/, '')
  return frac ? `${i}.${frac}` : i
}

export function Addr({ a, full }: { a: string; full?: boolean }) {
  const [copied, setCopied] = useState(false)
  return (
    <span className="addr" title={a}>
      <code>{full ? a : short(a)}</code>
      <button
        type="button"
        className="link"
        onClick={() => {
          void navigator.clipboard?.writeText(a)
          setCopied(true)
          setTimeout(() => setCopied(false), 1200)
        }}
      >
        {copied ? 'copied' : 'copy'}
      </button>
      {config.explorerUrl && (
        <a className="link" href={`${config.explorerUrl.replace(/\/$/, '')}/address/${a}`} target="_blank" rel="noreferrer">
          explorer
        </a>
      )}
    </span>
  )
}

export function Card({ title, children, actions }: { title: string; children: ReactNode; actions?: ReactNode }) {
  return (
    <section className="card">
      <header>
        <h2>{title}</h2>
        {actions}
      </header>
      {children}
    </section>
  )
}

export function Notice({ kind = 'info', children }: { kind?: 'info' | 'warn' | 'danger' | 'ok'; children: ReactNode }) {
  return <div className={`notice ${kind}`}>{children}</div>
}

/** Signer for an account record, or null for watch-only accounts. */
export async function signerFor(rec: AccountRecord): Promise<OwnerSigner | null> {
  if (rec.kind === 'passkey' && rec.passkey) return platformPasskeySigner(rec.passkey)
  if (rec.kind === 'virtual' && rec.virtualJwk) return (await SoftPasskey.fromJwk(rec.virtualJwk, location.hostname, location.origin)).signer()
  if (rec.kind === 'dev-key' && rec.devKey) return ecdsaSigner(privateKeyToAccount(rec.devKey), 'Dev key (EIP-7702 EOA)')
  return null
}

/** Polls the account state. */
export function useAccountState(address: Address | null, intervalMs = 4_000) {
  const [state, setState] = useState<AccountState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const refresh = useCallback(async () => {
    if (!address) return
    try {
      setState(await readAccountState(publicClient, address))
      setError(null)
    } catch (e) {
      setError((e as Error).message.split('\n')[0])
    }
  }, [address])
  useEffect(() => {
    setState(null)
    void refresh()
    const t = setInterval(() => void refresh(), intervalMs)
    return () => clearInterval(t)
  }, [refresh, intervalMs])
  return { state, error, refresh }
}

export function useNow(intervalMs = 1_000): bigint {
  const [now, setNow] = useState(() => BigInt(Math.floor(Date.now() / 1000)))
  useEffect(() => {
    const t = setInterval(() => setNow(BigInt(Math.floor(Date.now() / 1000))), intervalMs)
    return () => clearInterval(t)
  }, [intervalMs])
  return now
}

/** Runs UserOperations for the active account with status reporting. */
export function useUserOp(rec: AccountRecord | null, onDone?: () => void) {
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<SentUserOp | null>(null)
  const busy = useRef(false)
  const run = useCallback(
    async (input: { calls?: Call[]; callData?: Hex; sponsored?: boolean }): Promise<SentUserOp | null> => {
      if (!rec || busy.current) return null
      busy.current = true
      setError(null)
      setResult(null)
      try {
        const signer = await signerFor(rec)
        if (!signer) throw new Error('This account is watch-only in this browser: no signer.')
        const callData = input.callData ?? callsToCallData(input.calls ?? [])
        const r = await sendUserOp({
          client: publicClient,
          bundlerUrl: config.bundlerUrl,
          chainId: config.chainId,
          sender: rec.address,
          callData,
          signer,
          factory: rec.params && rec.salt ? { address: SMART_ACCOUNT_FACTORY, data: factoryData(rec.params, rec.salt) } : null,
          sponsor: input.sponsored ? config.paymasterSponsor : null,
          onStatus: setStatus,
        })
        setResult(r)
        setStatus(r.queuedIds.length ? `Queued by the Guardian Vault as action #${r.queuedIds.join(', #')}` : 'Done')
        onDone?.()
        return r
      } catch (e) {
        setStatus(null)
        setError((e as Error).message.split('\n')[0])
        return null
      } finally {
        busy.current = false
      }
    },
    [rec, onDone],
  )
  return { run, status, error, result }
}

export function OpStatus({ status, error }: { status: string | null; error: string | null }) {
  if (error) return <Notice kind="danger">{error}</Notice>
  if (status) return <Notice kind="info">{status}</Notice>
  return null
}

export { ENTRY_POINT }

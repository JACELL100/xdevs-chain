'use client'

import { useState } from 'react'
import { ENCLAVE, enclaveAddChainParameters } from '@/lib/site'
import { Check, Copy, Plus } from './icons'

export function CopyButton({ value, label = 'Copy' }: { value: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      type="button"
      className={`copy-btn${done ? ' is-done' : ''}`}
      aria-label={`${label}: ${value}`}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value)
          setDone(true)
          setTimeout(() => setDone(false), 1600)
        } catch {
          /* clipboard blocked: nothing to do */
        }
      }}
    >
      {done ? <Check /> : <Copy />}
      <span aria-live="polite">{done ? 'Copied' : 'Copy'}</span>
    </button>
  )
}

type Eip1193 = { request: (a: { method: string; params?: unknown[] }) => Promise<unknown> }

/** Adds Enclave to MetaMask (or any EIP-3085 wallet) and switches to it. */
export function AddNetworkButton({ className = 'btn is-primary' }: { className?: string }) {
  const [msg, setMsg] = useState<string | null>(null)
  const add = async () => {
    const eth = (window as unknown as { ethereum?: Eip1193 }).ethereum
    if (!eth) {
      setMsg(`No browser wallet found. Add it by hand: RPC ${ENCLAVE.rpcUrl}, chain id ${ENCLAVE.chainId}, symbol tXDS.`)
      return
    }
    try {
      await eth.request({ method: 'wallet_addEthereumChain', params: [enclaveAddChainParameters()] })
      setMsg('Enclave is in your wallet.')
    } catch (e) {
      const code = (e as { code?: number }).code
      setMsg(code === 4001 ? 'Request cancelled in the wallet.' : 'The wallet refused the request. Try adding it by hand.')
    }
  }
  return (
    <span style={{ display: 'inline-grid', gap: 10 }}>
      <button type="button" className={className} onClick={add}>
        <Plus />
        Add Enclave to MetaMask
      </button>
      {msg ? (
        <span className="small" role="status">
          {msg}
        </span>
      ) : null}
    </span>
  )
}

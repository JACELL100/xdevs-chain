'use client'

import { useState } from 'react'
import { cx } from './format'
import { ENCLAVE, XDEVS_LINKS, enclaveAddChainParameters } from './network'

type Eip1193 = { request(args: { method: string; params?: unknown[] }): Promise<unknown> }

export type AddToWalletProps = {
  label?: string
  variant?: 'primary' | 'ghost'
  size?: 'sm' | 'md' | 'lg'
  /** Called after the wallet accepted the request. */
  onAdded?: () => void
  className?: string
}

type State = { kind: 'idle' | 'busy' | 'ok' | 'err' | 'nowallet'; msg?: string }

/** Adds xDEVS Chain Enclave to the browser wallet (EIP-3085 wallet_addEthereumChain) and switches to it. */
export function AddToWallet({ label = 'Add Enclave to wallet', variant = 'ghost', size = 'md', onAdded, className }: AddToWalletProps) {
  const [state, setState] = useState<State>({ kind: 'idle' })

  const add = async () => {
    const eth = (globalThis as { ethereum?: Eip1193 }).ethereum
    if (!eth) {
      setState({ kind: 'nowallet' })
      return
    }
    setState({ kind: 'busy' })
    try {
      await eth.request({ method: 'wallet_addEthereumChain', params: [enclaveAddChainParameters()] })
      try {
        await eth.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: ENCLAVE.chainIdHex }] })
      } catch {
        /* adding succeeded; switching is optional */
      }
      setState({ kind: 'ok', msg: `${ENCLAVE.name} added.` })
      onAdded?.()
    } catch (e) {
      const err = e as { code?: number; message?: string }
      setState({ kind: 'err', msg: err.code === 4001 ? 'Request rejected in the wallet.' : (err.message ?? 'The wallet refused the request.') })
    }
  }

  return (
    <span className={cx('xu-addwallet', className)}>
      <button
        type="button"
        className={cx('xh-btn xu-btn', variant === 'primary' ? 'xh-btn-primary' : 'xh-btn-ghost', size !== 'lg' && `is-${size}`)}
        onClick={add}
        disabled={state.kind === 'busy'}
      >
        {state.kind === 'busy' ? 'Check your wallet…' : label}
      </button>
      {state.kind === 'ok' ? <p className="xu-addwallet-msg is-ok" role="status">{state.msg}</p> : null}
      {state.kind === 'err' ? <p className="xu-addwallet-msg is-err" role="alert">{state.msg}</p> : null}
      {state.kind === 'nowallet' ? (
        <p className="xu-addwallet-msg" role="status">
          No browser wallet found. Try the{' '}
          <a href={XDEVS_LINKS.wallet} className="xh-textlink">
            xDEVS wallet
          </a>{' '}
          or add chain id {ENCLAVE.chainId} with RPC {ENCLAVE.rpcUrl} manually.
        </p>
      ) : null}
    </span>
  )
}

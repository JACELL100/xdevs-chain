import { config } from '../config.ts'
import type { AccountState } from '../lib/account.ts'
import { formatCountdown } from '../lib/vault.ts'
import type { AccountRecord } from '../store.ts'
import { Addr, Card, Notice, OpStatus, fmtXds, useNow, useUserOp } from './common.tsx'

export function LockBadge({ state }: { state: AccountState }) {
  if (!state.is7702 || !state.eoaLock) return null
  const l = state.eoaLock
  const cls = l.status === 'unlocked' ? 'badge warn' : 'badge ok'
  return (
    <span className={cls} title={`XIP-0001 lock word ${l.word} (${l.source === 'node' ? 'xdevs_getEoaLock' : 'storage read'})`}>
      EOA {l.status}
      {l.protocolActive === false ? ' (not enforced on this chain)' : l.protocolActive === null ? ' (enforcement unknown)' : ''}
    </span>
  )
}

export function Overview({ rec, state, refresh }: { rec: AccountRecord; state: AccountState | null; refresh: () => void }) {
  const now = useNow()
  const op = useUserOp(rec, refresh)
  if (!state) return <Card title="Account">Loading…</Card>
  const needsDeploy = !state.deployed && !!rec.params
  const canPay = state.balance > 0n
  return (
    <div className="stack">
      <Card title={rec.label}>
        <div className="kv">
          <span>Address</span>
          <Addr a={rec.address} full />
          <span>Balance</span>
          <b>{fmtXds(state.balance)} tXDS</b>
          <span>Type</span>
          <span>
            {state.is7702 ? 'EIP-7702 account (your existing address, upgraded)' : state.deployed ? 'Smart account (factory)' : 'Smart account — not deployed yet'}{' '}
            <LockBadge state={state} />
          </span>
          <span>Signer</span>
          <span>{rec.kind === 'passkey' ? 'Passkey (this device)' : rec.kind === 'virtual' ? 'Virtual passkey (DEV ONLY)' : rec.kind === 'dev-key' ? 'Raw key (DEV ONLY)' : 'none (watch-only)'}</span>
          <span>Guardian Vault</span>
          <span>{state.deployed ? (state.policy.enabled ? `on — ${fmtXds(state.policy.nativeThreshold)} tXDS instant limit, ${formatCountdown(state.policy.delay)} delay` : 'off') : rec.params?.policy.enabled ? 'on (from your first transaction)' : 'off'}</span>
          <span>Guardians</span>
          <span>{state.guardians.length ? `${state.guardianThreshold} of ${state.guardians.length}` : state.deployed ? 'none' : `${rec.params?.guardianThreshold ?? 0} of ${rec.params?.guardians.length ?? 0}`}</span>
        </div>
      </Card>
      {state.frozen && (
        <Notice kind="danger">
          Frozen by your guardians until {new Date(Number(state.frozenUntil) * 1000).toLocaleString()} ({formatCountdown(state.frozenUntil - now)}). Nothing can
          be signed or executed until they unfreeze it, a recovery completes, or the freeze expires.
        </Notice>
      )}
      {needsDeploy && (
        <Card title="Activate your account">
          <p>
            The address above already works for receiving tXDS. The smart-account contract is created by your first
            transaction, signed with your passkey.
          </p>
          {config.paymasterSponsor ? (
            <p className="muted">Gas for the activation is sponsored by the xDEVS paymaster (sponsor {config.paymasterSponsor}).</p>
          ) : (
            <p className="muted">
              No gas sponsor is configured: fund the address first
              {config.faucetUrl ? (
                <>
                  {' '}
                  from the{' '}
                  <a href={`${config.faucetUrl}${config.faucetUrl.includes('?') ? '&' : '?'}address=${rec.address}`} target="_blank" rel="noreferrer">
                    faucet
                  </a>
                </>
              ) : null}
              .
            </p>
          )}
          <button type="button" disabled={rec.kind === 'watch' || (!config.paymasterSponsor && !canPay)} onClick={() => void op.run({ callData: '0x', sponsored: !!config.paymasterSponsor })}>
            Activate account
          </button>
          <OpStatus status={op.status} error={op.error} />
        </Card>
      )}
      {!needsDeploy && config.faucetUrl && (
        <p className="muted">
          Need testnet tXDS?{' '}
          <a href={`${config.faucetUrl}${config.faucetUrl.includes('?') ? '&' : '?'}address=${rec.address}`} target="_blank" rel="noreferrer">
            Open the faucet
          </a>{' '}
          (testnet tokens have no value).
        </p>
      )}
    </div>
  )
}

'use client'

import { AddToWallet, Badge, Button, Card, ENCLAVE, ExternalIcon, Hash, Stat } from '@xdevs/ui'
import { useCallback, useEffect, useId, useMemo, useState } from 'react'
import { COOLDOWN_SECONDS, config, txUrl } from '@/lib/config'
import { type CooldownMap, loadCooldowns, remainingSeconds, saveCooldowns, withCooldown } from '@/lib/cooldown'
import { DEMO_ADDRESS, DEMO_SCENARIOS, type DemoScenario, demoClaim, demoStatus, parseScenario } from '@/lib/demo'
import { type ClaimOutcome, type FaucetStatus, claim, fetchStatus, formatAmount, formatDuration } from '@/lib/faucet-api'
import { checkRecipient } from '@/lib/recipient'
import Turnstile from './Turnstile'

type Availability = 'checking' | 'open' | 'empty' | 'down' | 'unavailable' | 'soon'

function availabilityOf(s: FaucetStatus | null): Availability {
  if (!s) return 'checking'
  if (!s.reachable) return 'soon'
  if (s.healthy) return 'open'
  if (s.error === 'rpc_unavailable' || s.balance === null) return 'down'
  if (s.balance === undefined) return 'unavailable' // e.g. 503 misconfigured
  return 'empty'
}

const AVAIL_BADGE: Record<Availability, { tone: 'final' | 'warn' | 'alert' | 'dim'; text: string }> = {
  checking: { tone: 'dim', text: 'checking' },
  open: { tone: 'final', text: 'open' },
  empty: { tone: 'warn', text: 'refilling' },
  down: { tone: 'alert', text: 'network down' },
  unavailable: { tone: 'alert', text: 'unavailable' },
  soon: { tone: 'dim', text: 'launching soon' },
}

const STATUS_POLL_MS = 60_000

export default function FaucetApp() {
  const inputId = useId()
  const [scenario, setScenario] = useState<DemoScenario>('ready')
  const [status, setStatus] = useState<FaucetStatus | null>(null)
  const [input, setInput] = useState('')
  const [touched, setTouched] = useState(false)
  const [token, setToken] = useState<string | null>(null)
  const [resetSignal, setResetSignal] = useState(0)
  const [sending, setSending] = useState(false)
  const [result, setResult] = useState<ClaimOutcome | null>(null)
  const [cooldowns, setCooldowns] = useState<CooldownMap>({})
  const [now, setNow] = useState(() => Date.now())

  // Status: demo fixture or GET /api/status, refreshed every minute while visible.
  useEffect(() => {
    if (config.demo) {
      const sc = parseScenario(window.location.search)
      setScenario(sc)
      setStatus(demoStatus(sc))
      if (sc === 'success' || sc === 'limited') {
        setInput(DEMO_ADDRESS)
        setTouched(true)
        setResult(demoClaim(DEMO_ADDRESS, sc))
      }
      return
    }
    setCooldowns(loadCooldowns())
    const ctrl = new AbortController()
    const load = () => fetchStatus(config.apiBase, ctrl.signal).then(setStatus)
    void load()
    const id = setInterval(() => {
      if (document.visibilityState === 'visible') void load()
    }, STATUS_POLL_MS)
    return () => {
      ctrl.abort()
      clearInterval(id)
    }
  }, [])

  const check = useMemo(() => checkRecipient(input), [input])
  const avail = availabilityOf(status)
  const drip = status?.dripAmount ?? config.dripAmount
  const symbol = status?.symbol ?? ENCLAVE.nativeCurrency.symbol
  const localWait = check.ok ? remainingSeconds(cooldowns, check.value, now) : 0

  // 1 s tick only while a countdown is on screen.
  const ticking = localWait > 0 || (result !== null && !result.ok && result.kind === 'rate_limited')
  useEffect(() => {
    if (!ticking) return
    const id = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [ticking])

  const nameBlocked = check.ok && check.kind === 'name' && !config.xnsEnabled
  const fieldError = touched && !check.ok && !check.empty ? check.message : null
  const captchaReady = config.demo ? token === 'demo' : !!token
  const canSubmit = avail === 'open' && check.ok && !nameBlocked && captchaReady && !sending && localWait === 0

  const remember = useCallback((recipient: string, seconds: number) => {
    setCooldowns((m) => {
      const t = Date.now()
      const next = withCooldown(m, recipient, t + seconds * 1000, t)
      saveCooldowns(next)
      return next
    })
    setNow(Date.now())
  }, [])

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    setTouched(true)
    if (!canSubmit || !check.ok) return
    setSending(true)
    setResult(null)
    const outcome = config.demo
      ? await new Promise<ClaimOutcome>((r) => setTimeout(() => r(demoClaim(check.value, scenario)), 900))
      : await claim(config.apiBase, check.value, token ?? '')
    setSending(false)
    setResult(outcome)
    if (outcome.ok) remember(check.value, COOLDOWN_SECONDS)
    else if (outcome.kind === 'rate_limited') remember(check.value, outcome.retryAfter ?? COOLDOWN_SECONDS)
    // Turnstile tokens are single use: always get a fresh one.
    setToken(null)
    setResetSignal((n) => n + 1)
  }

  const buttonLabel =
    avail === 'soon'
      ? 'Opens at launch'
      : avail === 'down'
        ? 'Network unreachable'
        : avail === 'unavailable'
          ? 'Temporarily unavailable'
        : avail === 'empty'
          ? 'Paused for refill'
          : sending
            ? 'Sending…'
            : localWait > 0
              ? `Next claim in ${formatDuration(localWait)}`
              : `Send me ${drip} ${symbol}`

  return (
    <div className="fx-layout">
      <div className="fx-main">
        {config.demo ? <DemoBar scenario={scenario} /> : null}
        {avail === 'soon' ? <LaunchingSoon /> : null}
        {avail === 'down' ? (
          <Notice tone="alert" title="The Enclave network is unreachable">
            The faucet is up, but it cannot reach the Enclave RPC node, so claims are paused. Check the{' '}
            <a className="xh-textlink" href={config.statusUrl}>
              status page
            </a>
            .
          </Notice>
        ) : null}
        {avail === 'unavailable' ? (
          <Notice tone="alert" title="The faucet is temporarily unavailable">
            Claims are paused while the service is being fixed. Please try again later.
          </Notice>
        ) : null}
        {avail === 'empty' ? (
          <Notice tone="warn" title="The faucet is being refilled">
            Its hot wallet is below the safety floor, so claims are paused until the team tops it up. This usually takes a few
            hours.
          </Notice>
        ) : null}

        <form className="fx-claim xu-card" onSubmit={submit} noValidate aria-describedby={`${inputId}-hint`}>
          <div className="fx-claim-body">
            <label className="fx-label" htmlFor={inputId}>
              Wallet address{config.xnsEnabled ? <> or <span className="xh-mono">.xdevs</span> name</> : null}
            </label>
            <div className={`fx-field${fieldError ? ' is-invalid' : ''}${check.ok ? ' is-valid' : ''}`}>
              <input
                id={inputId}
                className="fx-input"
                type="text"
                inputMode="text"
                autoComplete="off"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                placeholder={config.xnsEnabled ? '0x… or alice.xdevs' : '0x…'}
                value={input}
                onChange={(e) => {
                  setInput(e.target.value)
                  if (result) setResult(null)
                }}
                onBlur={() => setTouched(true)}
                aria-invalid={fieldError ? true : undefined}
                aria-describedby={`${inputId}-hint ${inputId}-err`}
                maxLength={256}
              />
              {check.ok ? (
                <span className="fx-field-ok" aria-hidden="true">
                  <CheckIcon />
                </span>
              ) : null}
            </div>
            <p id={`${inputId}-err`} className="fx-field-msg" aria-live="polite">
              {fieldError ??
                (nameBlocked
                  ? '.xdevs names will work once the Enclave name service is live. Paste your 0x address for now.'
                  : localWait > 0
                    ? `This address claimed recently. Next claim in ${formatDuration(localWait)}.`
                    : '')}
            </p>
            <p id={`${inputId}-hint`} className="fx-hint">
              {drip} {symbol} per claim, once every 24 hours per address and per network. Funds arrive in the next block.
            </p>

            <div className="fx-captcha">
              {avail !== 'open' ? null : config.demo ? (
                <DemoCaptcha checked={token === 'demo'} onToggle={() => setToken(token === 'demo' ? null : 'demo')} />
              ) : config.turnstileSiteKey ? (
                <Turnstile siteKey={config.turnstileSiteKey} onToken={setToken} resetSignal={resetSignal} />
              ) : (
                <p className="fx-captcha-fail">Captcha is not configured for this build (NEXT_PUBLIC_TURNSTILE_SITE_KEY).</p>
              )}
            </div>

            <button type="submit" className="xh-btn xh-btn-primary xu-btn fx-submit" disabled={!canSubmit} aria-disabled={!canSubmit}>
              {sending ? <span className="fx-spinner" aria-hidden="true" /> : null}
              {buttonLabel}
            </button>
            {avail === 'open' && check.ok && !captchaReady && !sending && localWait === 0 && !nameBlocked ? (
              <p className="fx-hint fx-center">Complete the captcha to continue.</p>
            ) : null}
          </div>
          <div aria-live="polite">{result ? <ResultPanel result={result} now={now} /> : null}</div>
        </form>

        <ol className="fx-steps" aria-label="How it works">
          <li>
            <span className="fx-step-n">1</span>
            <span>
              <b>Add Enclave</b> to your wallet (chain {ENCLAVE.chainId}).
            </span>
          </li>
          <li>
            <span className="fx-step-n">2</span>
            <span>
              <b>Paste your address</b> and pass the captcha.
            </span>
          </li>
          <li>
            <span className="fx-step-n">3</span>
            <span>
              <b>
                Receive {drip} {symbol}
              </b>{' '}
              in about two seconds, final on arrival.
            </span>
          </li>
        </ol>
      </div>

      <aside className="fx-side" aria-label="Faucet details">
        <Card
          title="Faucet"
          actions={
            <Badge tone={AVAIL_BADGE[avail].tone} dot>
              {AVAIL_BADGE[avail].text}
            </Badge>
          }
        >
          <div className="fx-stats">
            <Stat label="per claim" value={`${formatAmount(drip)} ${symbol}`} />
            <Stat label="cooldown" value="24 h" hint="per address and IP" />
            <Stat
              label="faucet balance"
              loading={avail === 'checking'}
              value={status?.balance ? `${formatAmount(status.balance, 0)} ${symbol}` : '—'}
              hint={avail === 'soon' ? 'available at launch' : avail === 'down' ? 'RPC unreachable' : undefined}
            />
            <Stat label="chain id" value={<span className="xh-num">{status?.chainId ?? ENCLAVE.chainId}</span>} />
          </div>
          {status?.address ? (
            <div className="fx-faucet-addr">
              <span className="fx-k">hot wallet</span>
              <Hash value={status.address} href={`${config.explorerUrl}/address/${status.address}`} head={6} tail={4} />
            </div>
          ) : null}
        </Card>

        <Card title="Add Enclave to your wallet">
          <dl className="fx-params">
            <div>
              <dt>Network</dt>
              <dd>{ENCLAVE.name}</dd>
            </div>
            <div>
              <dt>Chain id</dt>
              <dd className="xh-num">
                {ENCLAVE.chainId} <span className="xh-quiet">({ENCLAVE.chainIdHex})</span>
              </dd>
            </div>
            <div>
              <dt>Currency</dt>
              <dd>
                {ENCLAVE.nativeCurrency.name} ({ENCLAVE.nativeCurrency.symbol})
              </dd>
            </div>
            <div>
              <dt>RPC</dt>
              <dd>
                <Hash value={ENCLAVE.rpcUrl} full />
              </dd>
            </div>
          </dl>
          <AddToWallet label="Add Enclave to MetaMask" variant="primary" size="md" className="fx-add" />
        </Card>

        <div className="fx-warning" role="note">
          <span className="fx-warning-icon" aria-hidden="true">
            <WarnIcon />
          </span>
          <div>
            <p className="fx-warning-title">tXDS has no value. Never buy it.</p>
            <p>
              EnclaveXDS is free test currency. It cannot be sold and will not convert to mainnet XDS. Anyone selling tXDS is
              running a scam. This faucet never asks for a seed phrase or private key.
            </p>
          </div>
        </div>
      </aside>
    </div>
  )
}

function ResultPanel({ result, now }: { result: ClaimOutcome; now: number }) {
  const [shownAt] = useState(now)
  if (result.ok) {
    const href = result.explorerUrl ?? txUrl(result.txHash)
    return (
      <div className="fx-result is-ok" role="status">
        <span className="fx-result-icon">
          <CheckIcon />
        </span>
        <div className="fx-result-body">
          <p className="fx-result-title">
            {formatAmount(result.amount)} {result.symbol} are on the way
            {config.demo ? <Badge tone="dim">demo</Badge> : null}
          </p>
          <p className="fx-result-text">Sent in one transaction. It is final as soon as the next block lands.</p>
          <div className="fx-result-tx">
            <span className="fx-k">transaction</span>
            <Hash value={result.txHash} href={href} head={10} tail={8} />
          </div>
          <div className="fx-result-actions">
            <Button href={href} external variant="ghost" size="sm" icon={<ExternalIcon />}>
              View in explorer
            </Button>
            <span className="fx-hint">Next claim for this address in 24 h.</span>
          </div>
        </div>
      </div>
    )
  }
  const wait = result.kind === 'rate_limited' ? Math.max(0, (result.retryAfter ?? 0) - Math.floor((now - shownAt) / 1000)) : 0
  const view: Record<string, { tone: 'warn' | 'alert'; title: string; text: string }> = {
    rate_limited: {
      tone: 'warn',
      title: wait > 0 ? `Already claimed. Try again in ${formatDuration(wait)}` : 'You can claim again now',
      text: `${result.message.replace(/ Try again in about .*$/, '')} The limit is one claim per 24 hours for each address and each network.`,
    },
    faucet_empty: { tone: 'warn', title: 'The faucet is being refilled', text: result.message },
    network_down: { tone: 'alert', title: 'The Enclave network is unreachable', text: result.message },
    unreachable: {
      tone: 'alert',
      title: 'Could not reach the faucet',
      text: `${result.message} If the Enclave testnet has not launched yet, this is expected.`,
    },
    captcha: { tone: 'warn', title: 'Captcha check failed', text: `${result.message} Solve it again and resend.` },
    invalid_recipient: { tone: 'alert', title: 'That address cannot receive tXDS', text: result.message },
    names_not_supported: { tone: 'warn', title: '.xdevs names are not live yet', text: result.message },
  }
  const v = view[result.kind] ?? { tone: 'alert' as const, title: 'The claim did not go through', text: result.message }
  return (
    <div className={`fx-result is-${v.tone}`} role="alert">
      <span className="fx-result-icon">{v.tone === 'warn' ? <ClockIcon /> : <WarnIcon />}</span>
      <div className="fx-result-body">
        <p className="fx-result-title">
          {v.title}
          {config.demo ? <Badge tone="dim">demo</Badge> : null}
        </p>
        <p className="fx-result-text">{v.text}</p>
        {result.code ? <p className="fx-code">{result.code}</p> : null}
      </div>
    </div>
  )
}

function LaunchingSoon() {
  return (
    <div className="fx-soon">
      <div className="fx-soon-glow" aria-hidden="true" />
      <Badge tone="dim" dot>
        not live yet
      </Badge>
      <h2 className="fx-soon-title">The Enclave testnet is launching soon.</h2>
      <p className="fx-soon-text">
        The faucet opens the moment the network goes live. Add Enclave to your wallet now and you are ready on day one.
      </p>
      <div className="fx-soon-actions">
        <Button href={config.statusUrl} variant="ghost" size="sm" external icon={<ExternalIcon />}>
          Network status
        </Button>
        <Button href="https://xdevschain.tech/network/endpoints" variant="quiet" size="sm" external icon={<ExternalIcon />}>
          Planned endpoints
        </Button>
      </div>
    </div>
  )
}

function Notice({ tone, title, children }: { tone: 'warn' | 'alert'; title: string; children: React.ReactNode }) {
  return (
    <div className={`fx-notice is-${tone}`} role="status">
      <span className="fx-result-icon">{tone === 'warn' ? <ClockIcon /> : <WarnIcon />}</span>
      <div>
        <p className="fx-result-title">{title}</p>
        <p className="fx-result-text">{children}</p>
      </div>
    </div>
  )
}

function DemoCaptcha({ checked, onToggle }: { checked: boolean; onToggle: () => void }) {
  return (
    <button type="button" className={`fx-democaptcha${checked ? ' is-on' : ''}`} onClick={onToggle} aria-pressed={checked}>
      <span className="fx-democaptcha-box">{checked ? <CheckIcon /> : null}</span>
      <span>{checked ? 'Verified' : 'Verify you are human'}</span>
      <span className="fx-democaptcha-tag">Turnstile · demo</span>
    </button>
  )
}

function DemoBar({ scenario }: { scenario: DemoScenario }) {
  return (
    <nav className="fx-demobar" aria-label="Demo scenarios">
      <Badge tone="warn">Demo data</Badge>
      <span className="fx-demobar-text">Fixture responses, no real claims.</span>
      <span className="fx-demobar-links">
        {DEMO_SCENARIOS.map((s) => (
          <a key={s} href={`?scenario=${s}`} aria-current={s === scenario ? 'true' : undefined}>
            {s}
          </a>
        ))}
      </span>
    </nav>
  )
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path d="M3.5 8.5 6.5 11.5 12.5 4.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}
function WarnIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path d="M8 2.2 14.2 13H1.8Z" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
      <path d="M8 6.4v3.2M8 11.3v.1" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  )
}
function ClockIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <circle cx="8" cy="8" r="5.8" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <path d="M8 4.8V8l2.2 1.6" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  )
}

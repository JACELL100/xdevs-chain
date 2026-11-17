'use client'

import { AddToWallet, Badge, Button, Card, ENCLAVE, ExternalIcon, Stat } from '@xdevs/ui'
import { truncateHex } from '@xdevs/ui/format'
import { useEffect, useMemo, useState } from 'react'
import { config, docsLinks, PLANNED } from '@/lib/config'
import { demoIncidents, DEMO_VALIDATORS } from '@/lib/demo'
import { type Incident, parseIncidents } from '@/lib/incidents'
import { COMPONENT_ORDER, type ComponentState, median, overallStatus } from '@/lib/model'
import {
  type Health,
  blockIntervals,
  blockTimeStats,
  finality,
  formatAgo,
  headHealth,
  validatorStats,
} from '@/lib/stats'
import { useNetwork } from '@/lib/useNetwork'
import incidentsJson from '@/content/incidents.json'
import validatorsJson from '@/content/validators.json'
import BlockTimeChart from './BlockTimeChart'
import ChainParams from './ChainParams'
import Incidents from './Incidents'

const HEALTH_TONE: Record<Health, 'final' | 'warn' | 'alert' | 'dim'> = { up: 'final', degraded: 'warn', down: 'alert', unknown: 'dim' }
const HEALTH_LABEL: Record<Health, string> = { up: 'operational', degraded: 'degraded', down: 'down', unknown: 'checking' }

function useNow(ms = 1000) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), ms)
    return () => clearInterval(id)
  }, [ms])
  return now
}

const knownValidators: { id: string; name?: string }[] = config.demo
  ? DEMO_VALIDATORS
  : (Array.isArray(validatorsJson) ? (validatorsJson as { id?: unknown; name?: unknown }[]) : [])
      .filter((v) => typeof v.id === 'string')
      .map((v) => ({ id: String(v.id), name: typeof v.name === 'string' ? v.name : undefined }))

export default function Dashboard() {
  const net = useNetwork()
  const now = useNow()
  const [incidents, setIncidents] = useState<Incident[]>([])
  useEffect(() => {
    setIncidents(config.demo ? demoIncidents(Date.now()) : parseIncidents(incidentsJson).incidents)
  }, [])

  const intervals = useMemo(() => blockIntervals(net.blocks), [net.blocks])
  const bt = useMemo(() => blockTimeStats(intervals), [intervals])
  const latestCert = net.certs[net.certs.length - 1]
  const fin = latestCert ? finality(latestCert, net.validatorCount) : null
  const vals = useMemo(() => validatorStats(net.certs, knownValidators), [net.certs])
  const sinceBlock = net.head ? Math.max(0, now / 1000 - net.head.timestamp) : null
  const headH = headHealth(sinceBlock, PLANNED.blockTimeSeconds)
  const comps = COMPONENT_ORDER.map((id) => net.components[id])
  const overall = overallStatus(net.phase, net.everReached, config.launched, comps, net.head ? headH : 'unknown')
  const prelaunch = overall.level === 'prelaunch'
  const lat = median(net.latencies.slice(-10))

  return (
    <>
      {config.demo ? (
        <p className="st-demo">
          <Badge tone="warn">Demo data</Badge> A seeded simulation of a 4-validator Enclave. Nothing here is live.
        </p>
      ) : null}

      <section className={`st-hero is-${overall.level}`} aria-live="polite">
        <div className="st-hero-glow" aria-hidden="true" />
        <span className="st-hero-dot" aria-hidden="true" />
        <div className="st-hero-text">
          <h1 className="st-hero-title">{overall.title}</h1>
          <p className="st-hero-sub">
            {prelaunch ? (
              <>
                The public testnet is being prepared. This page goes live with it: blocks, finality, validators and every public
                endpoint, checked from your browser.
              </>
            ) : overall.level === 'connecting' ? (
              <>Contacting {config.rpcUrl.replace(/^https?:\/\//, '')}…</>
            ) : (
              <>
                {ENCLAVE.name} · chain {net.chainId ?? ENCLAVE.chainId}
                {net.updatedAt ? ` · checked ${formatAgo((now - net.updatedAt) / 1000)} ago from your browser` : ''}
              </>
            )}
          </p>
        </div>
        <div className="st-hero-actions">
          <AddToWallet label="Add Enclave to wallet" size="sm" />
        </div>
      </section>

      {prelaunch ? <Prelaunch /> : null}

      {!prelaunch && overall.level !== 'connecting' ? (
        <>
          <div className="st-kpis">
            <Kpi
              label="block height"
              value={net.head ? `#${net.head.number.toLocaleString('en-US')}` : '—'}
              hint={
                sinceBlock !== null ? (
                  <span className="st-kpi-fresh">
                    <span className={`st-pip is-${headH}`} aria-hidden="true" />
                    last block {formatAgo(sinceBlock)} ago
                  </span>
                ) : undefined
              }
            />
            <Kpi
              label="block time (median)"
              value={bt ? `${bt.median.toFixed(1)} s` : '—'}
              hint={bt ? `p95 ${bt.p95.toFixed(1)} s · target ${PLANNED.blockTimeSeconds} s` : `target ${PLANNED.blockTimeSeconds} s`}
            />
            <Kpi
              label="finality"
              value={fin ? `${fin.signed} / ${fin.total}` : net.certSupport === 'no' ? 'n/a' : '—'}
              hint={fin ? (fin.quorum ? 'validators signed · final' : 'below 2/3 quorum') : 'commit certificates'}
            />
            <Kpi label="peers" value={net.peers ?? '—'} hint="net_peerCount of the RPC node" />
            <Kpi label="rpc latency" value={lat !== null ? `${Math.round(lat)} ms` : '—'} hint="median, from your browser" />
          </div>

          <div className="st-row">
            <Card
              title="Block time"
              className="st-chart-card"
              actions={bt ? <span className="st-card-meta">last {bt.count} blocks</span> : null}
            >
              {intervals.length > 1 ? (
                <BlockTimeChart intervals={intervals} target={PLANNED.blockTimeSeconds} />
              ) : (
                <p className="st-muted">Collecting blocks…</p>
              )}
            </Card>
            <Card title="Finality" actions={latestCert ? <span className="st-card-meta">block #{latestCert.height.toLocaleString('en-US')}</span> : null}>
              {fin && latestCert ? (
                <FinalityMeter signed={fin.signed} total={fin.total} quorum={fin.quorum} round={latestCert.round} source={net.validatorSource} />
              ) : (
                <p className="st-muted">
                  {net.certSupport === 'no'
                    ? 'This RPC node does not serve xdevs_getCommitCertificate.'
                    : 'Waiting for the first commit certificate…'}
                </p>
              )}
            </Card>
          </div>

          <Card
            title="Validators"
            flush
            actions={<span className="st-card-meta">participation in the last {net.certs.length || '—'} blocks</span>}
          >
            {vals.length ? (
              <ValidatorTable vals={vals} head={net.head?.number ?? null} />
            ) : (
              <p className="st-muted st-pad">No commit certificates yet, so no signatures to show.</p>
            )}
          </Card>
        </>
      ) : null}

      <section className="st-section" aria-labelledby="components-h">
        <div className="st-section-head">
          <h2 id="components-h" className="st-h2">
            Components
          </h2>
          <span className="st-card-meta">checked every 30 s from your browser</span>
        </div>
        <div className="st-comps">
          {comps.map((c) => (
            <ComponentCard key={c.id} c={c} prelaunch={prelaunch} />
          ))}
        </div>
      </section>

      <div className="st-row st-row-even">
        <Card title="Incidents & maintenance">
          <Incidents incidents={incidents} now={now} />
        </Card>
        <Card title="Chain parameters">
          <ChainParams />
        </Card>
      </div>
    </>
  )
}

function Kpi({ label, value, hint }: { label: string; value: React.ReactNode; hint?: React.ReactNode }) {
  return (
    <div className="st-kpi">
      <Stat label={label} value={value} hint={hint} />
    </div>
  )
}

function FinalityMeter({ signed, total, quorum, round, source }: { signed: number; total: number; quorum: boolean; round: number; source: 'rpc' | 'config' }) {
  const threshold = (2 / 3) * 100
  return (
    <div className="st-fin">
      <div className="st-fin-top">
        <span className="st-fin-big xh-num">
          {signed}
          <span className="xh-quiet">/{total}</span>
        </span>
        <Badge tone={quorum ? 'final' : 'alert'} dot>
          {quorum ? 'final' : 'no quorum'}
        </Badge>
      </div>
      <div className="st-fin-bar" role="img" aria-label={`${signed} of ${total} validators signed; more than two thirds needed`}>
        {Array.from({ length: total }, (_, i) => (
          <span key={i} className={i < signed ? 'is-on' : ''} />
        ))}
        <i className="st-fin-mark" style={{ left: `${threshold}%` }} aria-hidden="true" />
      </div>
      <div className="st-fin-legend">
        <span>signatures in the latest commit certificate</span>
        <span>2/3 quorum</span>
      </div>
      <p className="st-fin-note">
        Committed in round {round}. Every block is final the moment it is committed: no confirmations to wait for.
        {source === 'config' ? ' Validator set size from the genesis config until xdevs_validators ships.' : ''}
      </p>
    </div>
  )
}

function ValidatorTable({ vals, head }: { vals: ReturnType<typeof validatorStats>; head: number | null }) {
  return (
    <div className="xu-table-wrap">
      <table className="xu-table st-vals">
        <thead>
          <tr>
            <th>Validator</th>
            <th className="is-right">Last signed</th>
            <th className="is-right">Missed</th>
            <th>Participation</th>
          </tr>
        </thead>
        <tbody>
          {vals.map((v) => {
            const behind = v.lastSigned !== null && head !== null ? head - v.lastSigned : null
            const tone = v.lastSigned === null ? 'alert' : v.missed === 0 ? 'final' : (behind ?? 0) > 3 ? 'alert' : 'warn'
            return (
              <tr key={v.id}>
                <td>
                  <span className="st-val">
                    <span className={`st-pip is-${tone === 'final' ? 'up' : tone === 'warn' ? 'degraded' : 'down'}`} aria-hidden="true" />
                    <span className="st-val-text">
                      <span className="st-val-name">{v.name ?? truncateHex(v.id, 6, 4)}</span>
                      {v.name ? <span className="st-val-id xh-mono" title={v.id}>{truncateHex(v.id, 6, 4)}</span> : null}
                    </span>
                  </span>
                </td>
                <td className="is-right xh-num">
                  {v.lastSigned === null ? (
                    <span className="st-bad">not in window</span>
                  ) : (
                    <>
                      #{v.lastSigned.toLocaleString('en-US')}
                      {behind ? <span className="st-dim"> · {behind} behind</span> : null}
                    </>
                  )}
                </td>
                <td className="is-right xh-num">
                  {v.missed}
                  <span className="st-dim"> / {v.window}</span>
                </td>
                <td>
                  <span className="st-part">
                    <span className="st-part-bar">
                      <span style={{ width: `${(v.uptime * 100).toFixed(1)}%` }} className={v.uptime < 0.9 ? 'is-low' : ''} />
                    </span>
                    <span className="xh-num st-part-v">{(v.uptime * 100).toFixed(v.uptime === 1 ? 0 : 1)}%</span>
                  </span>
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function ComponentCard({ c, prelaunch }: { c: ComponentState; prelaunch: boolean }) {
  // Before launch nothing is expected to answer: say "not live yet" instead of a wall of red.
  const notLive = prelaunch && c.health === 'down' && c.id !== 'website'
  const tone = notLive ? 'dim' : HEALTH_TONE[c.health]
  return (
    <div className={`st-comp is-${notLive ? 'prelaunch' : c.health}`}>
      <div className="st-comp-top">
        <span className="st-comp-label">{c.label}</span>
        <Badge tone={tone} dot>
          {notLive ? 'not live yet' : HEALTH_LABEL[c.health]}
        </Badge>
      </div>
      <p className="st-comp-desc">{c.description}</p>
      <p className="st-comp-detail">
        <span>{notLive ? 'Launches with Enclave' : c.detail}</span>
        {c.latencyMs !== null && !notLive ? <span className="xh-num">{Math.round(c.latencyMs)} ms</span> : null}
      </p>
      <p className="st-comp-url xh-mono" title={c.url}>
        {c.url.replace(/^(https?|wss?):\/\//, '')}
      </p>
    </div>
  )
}

function Prelaunch() {
  const items: [string, string][] = [
    ['chain id', `${ENCLAVE.chainId}`],
    ['currency', `${ENCLAVE.nativeCurrency.symbol}`],
    ['block time', `${PLANNED.blockTimeSeconds} s`],
    ['validators', `${PLANNED.validators} at genesis`],
  ]
  return (
    <section className="st-pre" aria-label="Planned parameters">
      <div className="st-pre-grid">
        {items.map(([k, v]) => (
          <div key={k} className="st-kpi">
            <Stat label={k} value={v} hint="planned" />
          </div>
        ))}
      </div>
      <div className="st-pre-links">
        <Button href={docsLinks.endpoints} external variant="ghost" size="sm" icon={<ExternalIcon />}>
          Planned endpoints
        </Button>
        <Button href={docsLinks.quickstart} external variant="quiet" size="sm" icon={<ExternalIcon />}>
          Run a local chain meanwhile
        </Button>
      </div>
    </section>
  )
}

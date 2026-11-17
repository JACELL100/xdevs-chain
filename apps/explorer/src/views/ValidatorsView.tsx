'use client'

import { Card, Table, type TableColumn } from '@xdevs/ui'
import { predeploys } from '@xdevs/sdk'
import { Addr, Native, PageHead, Pill, SourceNote } from '../components/bits'
import { config } from '../lib/config'
import { loadValidators } from '../lib/data'
import { formatDate, formatUnits } from '../lib/format'
import { networkDown, useNet } from '../lib/net'
import type { ValidatorRow } from '../lib/types'
import { useLoad } from '../lib/useLoad'
import { NotReachable } from '../components/NotReachable'
import { Page, Problem, useTitle } from './common'

const STATUS_TONE = { active: 'ok', candidate: 'info', jailed: 'warn', tombstoned: 'danger', exited: 'neutral', unknown: 'neutral' } as const

export function ValidatorsView() {
  useTitle('Validators')
  const net = useNet()
  const r = useLoad(() => loadValidators(), 'validators', 60_000)
  if (!r.data && (networkDown(net) || (r.error && !config.demo && net.rpc === 'down')))
    return (
      <Page>
        <NotReachable what="the validator set" />
      </Page>
    )

  const set = r.data
  const rows = [...(set?.validators ?? [])].sort((a, b) => order(a) - order(b) || cmp(BigInt(b.totalStake), BigInt(a.totalStake)))
  const totalPower = rows.reduce((n, v) => n + BigInt(v.votingPower ?? '0x0'), 0n)
  const totalStake = rows.reduce((n, v) => n + BigInt(v.totalStake), 0n)
  const active = rows.filter((v) => v.status === 'active')
  const quorum = (totalPower * 2n) / 3n + 1n

  const cols: TableColumn<ValidatorRow>[] = [
    { key: 'rank', header: '#', render: (_v, i) => <span className="x-num x-dim">{i + 1}</span>, width: 40 },
    {
      key: 'who',
      header: 'Validator',
      render: (v) => (
        <span className="x-val">
          <span className="x-val-avatar" aria-hidden="true" style={{ background: avatar(v.operator) }} />
          <span className="x-val-id">
            <span className="x-strong">{v.name ?? 'Unnamed validator'}</span>
            <Addr address={v.operator} copy={false} plain />
          </span>
        </span>
      ),
    },
    { key: 'status', header: 'Status', render: (v) => <Pill tone={STATUS_TONE[v.status]} dot={v.status === 'active'}>{v.status}{v.status === 'jailed' && v.jailedUntil ? ` · until ${formatDate(v.jailedUntil).slice(5, 16)}` : ''}</Pill> },
    { key: 'stake', header: 'Total stake', align: 'right', render: (v) => <Native value={v.totalStake} digits={0} /> },
    {
      key: 'power',
      header: 'Voting power',
      render: (v) => {
        const p = BigInt(v.votingPower ?? '0x0')
        const share = totalPower > 0n ? Number((p * 10000n) / totalPower) / 100 : 0
        return (
          <span className="x-power-cell">
            <span className="x-meter is-thin" aria-hidden="true"><span style={{ width: `${share}%` }} /></span>
            <span className="x-num">{share ? `${share.toFixed(2)}%` : '—'}</span>
          </span>
        )
      },
    },
    { key: 'commission', header: 'Commission', align: 'right', render: (v) => <span className="x-num">{(v.commissionBps / 100).toFixed(v.commissionBps % 100 ? 1 : 0)}%</span> },
    { key: 'uptime', header: 'Uptime', align: 'right', render: (v) => (v.uptimeBps === null ? <span className="x-dim">—</span> : <span className={`x-num ${v.uptimeBps < 9500 ? 'x-warn-text' : ''}`}>{(v.uptimeBps / 100).toFixed(2)}%</span>) },
  ]

  return (
    <Page>
      <PageHead eyebrow={config.networkName} title="Validators">
        <span className="x-muted">
          Read from the Staking predeploy <a className="x-link x-mono" href={`/address/${predeploys.STAKING}`}>0x…1000</a>. Blocks are final once validators holding more than two thirds of the voting power precommit.
        </span>
      </PageHead>
      {r.error && !set ? <Problem error={r.error} onRetry={r.reload} what="the validator set" /> : null}
      <section className="x-stats is-4" aria-label="Validator set">
        <Fact label="active validators" value={set ? String(active.length) : null} hint={set ? `${rows.length} registered` : undefined} />
        <Fact label="total stake" value={set ? `${formatUnits(totalStake, 18, 0)}` : null} hint={config.nativeSymbol} />
        <Fact label="finality quorum" value={set && totalPower > 0n ? `> 2/3` : set ? '—' : null} hint={set && totalPower > 0n ? `${quorum.toLocaleString('en-US')} of ${totalPower.toLocaleString('en-US')} power` : undefined} />
        <Fact label="epoch" value={set ? (set.epoch !== null ? set.epoch.toLocaleString('en-US') : '—') : null} hint={set?.via === 'rpc-validators' ? 'xdevs_validators' : 'Staking.currentEpoch()'} />
      </section>
      <Card title="Validator set" flush className="x-section">
        <Table caption="Validators" columns={cols} rows={rows} rowKey={(v) => v.operator} loading={!set && !r.error} skeletonRows={7} empty={<p className="x-empty-row">No validators registered in the Staking predeploy.</p>} className="x-table" />
      </Card>
      {set ? <SourceNote source={config.demo ? 'demo' : 'rpc'} /> : null}
    </Page>
  )
}

function Fact({ label, value, hint }: { label: string; value: string | null; hint?: string }) {
  return (
    <div className="xu-stat x-stat">
      <span className="xu-stat-label">{label}</span>
      <span className="xu-stat-value">{value ?? <span className="xu-skel" style={{ width: '50%', height: 26, display: 'inline-block' }} />}</span>
      {hint ? <span className="xu-stat-hint">{hint}</span> : null}
    </div>
  )
}

const order = (v: ValidatorRow) => ({ active: 0, candidate: 1, jailed: 2, exited: 3, tombstoned: 4, unknown: 5 })[v.status]
const cmp = (a: bigint, b: bigint) => (a === b ? 0 : a > b ? 1 : -1)
/** Deterministic, calm two-tone avatar from the operator address. */
function avatar(a: string): string {
  const h = parseInt(a.slice(2, 6), 16) % 360
  return `linear-gradient(135deg, hsl(${h} 38% 42%), hsl(${(h + 40) % 360} 30% 22%))`
}

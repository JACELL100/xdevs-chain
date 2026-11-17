import { Badge } from '@xdevs/ui'
import type { Incident } from '@/lib/incidents'
import { arrangeIncidents } from '@/lib/incidents'

const STATUS_LABEL: Record<Incident['status'], string> = {
  investigating: 'investigating',
  identified: 'identified',
  monitoring: 'monitoring',
  resolved: 'resolved',
  scheduled: 'scheduled',
  in_progress: 'in progress',
  completed: 'completed',
}

const fmt = (iso: string) =>
  new Date(iso).toLocaleString('en-GB', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', timeZone: 'UTC' }) + ' UTC'

function Item({ i }: { i: Incident }) {
  const done = i.status === 'resolved' || i.status === 'completed'
  const tone = done ? 'dim' : i.kind === 'maintenance' ? 'neutral' : i.severity === 'minor' ? 'warn' : 'alert'
  return (
    <li className="st-inc">
      <div className="st-inc-head">
        <span className="st-inc-title">{i.title}</span>
        <Badge tone={tone} dot={!done}>
          {i.kind === 'maintenance' ? 'maintenance · ' : ''}
          {STATUS_LABEL[i.status]}
        </Badge>
      </div>
      <p className="st-inc-when">
        {fmt(i.startsAt)}
        {i.endsAt ? ` → ${fmt(i.endsAt)}` : ''}
        {i.components.length ? ` · ${i.components.join(', ')}` : ''}
      </p>
      {i.updates.length ? (
        <ol className="st-inc-updates">
          {i.updates.map((u) => (
            <li key={u.at}>
              <time dateTime={u.at}>{fmt(u.at)}</time>
              <span>{u.text}</span>
            </li>
          ))}
        </ol>
      ) : null}
    </li>
  )
}

export default function Incidents({ incidents, now }: { incidents: Incident[]; now: number }) {
  const { active, upcoming, past } = arrangeIncidents(incidents, now)
  if (!active.length && !upcoming.length && !past.length) {
    return (
      <div className="st-inc-empty">
        <span className="st-ok-dot" aria-hidden="true" />
        <div>
          <p className="st-inc-empty-title">No incidents or maintenance</p>
          <p className="st-inc-empty-text">Nothing reported in the last 30 days. Planned work is announced here in advance.</p>
        </div>
      </div>
    )
  }
  return (
    <div className="st-inc-groups">
      {[
        ['Ongoing', active],
        ['Scheduled maintenance', upcoming],
        ['Past 30 days', past],
      ].map(([label, list]) =>
        (list as Incident[]).length ? (
          <section key={label as string} aria-label={label as string}>
            <h3 className="st-sub">{label as string}</h3>
            <ul className="st-inc-list">
              {(list as Incident[]).map((i) => (
                <Item key={i.id} i={i} />
              ))}
            </ul>
          </section>
        ) : null,
      )}
    </div>
  )
}

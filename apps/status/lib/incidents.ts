// Incident / maintenance notes from content/incidents.json (static, edited by hand in a PR).
//
// Shape of one entry:
// {
//   "id": "2026-12-01-rpc",                 unique
//   "title": "Public RPC returning 502s",
//   "kind": "incident" | "maintenance",
//   "severity": "minor" | "major" | "critical"          (incidents; maintenance ignores it)
//   "status": "investigating" | "identified" | "monitoring" | "resolved" | "scheduled" | "in_progress" | "completed",
//   "components": ["rpc", "ws", "explorer", "faucet", "bundler", "website", "chain"],   optional
//   "startsAt": "2026-12-01T10:00:00Z",
//   "endsAt": "2026-12-01T11:30:00Z",        optional
//   "updates": [{ "at": "2026-12-01T10:05:00Z", "text": "We are looking into it." }]   newest first or any order
// }

export type IncidentKind = 'incident' | 'maintenance'
export type IncidentStatus =
  | 'investigating'
  | 'identified'
  | 'monitoring'
  | 'resolved'
  | 'scheduled'
  | 'in_progress'
  | 'completed'

export interface Incident {
  id: string
  title: string
  kind: IncidentKind
  severity?: 'minor' | 'major' | 'critical'
  status: IncidentStatus
  components: string[]
  startsAt: string
  endsAt?: string
  updates: { at: string; text: string }[]
}

const STATUSES: IncidentStatus[] = ['investigating', 'identified', 'monitoring', 'resolved', 'scheduled', 'in_progress', 'completed']

const isDate = (v: unknown): v is string => typeof v === 'string' && !Number.isNaN(Date.parse(v))

/** Validate and normalize; malformed entries are dropped (and reported via the second value). */
export function parseIncidents(raw: unknown): { incidents: Incident[]; rejected: number } {
  if (!Array.isArray(raw)) return { incidents: [], rejected: 0 }
  const incidents: Incident[] = []
  let rejected = 0
  for (const r of raw) {
    const o = r as Record<string, unknown> | null
    if (
      !o ||
      typeof o.id !== 'string' ||
      typeof o.title !== 'string' ||
      (o.kind !== 'incident' && o.kind !== 'maintenance') ||
      !STATUSES.includes(o.status as IncidentStatus) ||
      !isDate(o.startsAt)
    ) {
      rejected++
      continue
    }
    const updates = Array.isArray(o.updates)
      ? (o.updates as Record<string, unknown>[])
          .filter((u) => u && isDate(u.at) && typeof u.text === 'string')
          .map((u) => ({ at: u.at as string, text: u.text as string }))
          .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
      : []
    const inc: Incident = {
      id: o.id,
      title: o.title,
      kind: o.kind,
      status: o.status as IncidentStatus,
      components: Array.isArray(o.components) ? o.components.filter((c): c is string => typeof c === 'string') : [],
      startsAt: o.startsAt,
      updates,
    }
    if (o.severity === 'minor' || o.severity === 'major' || o.severity === 'critical') inc.severity = o.severity
    if (isDate(o.endsAt)) inc.endsAt = o.endsAt
    incidents.push(inc)
  }
  return { incidents, rejected }
}

export function isActive(i: Incident, now: number): boolean {
  if (i.status === 'resolved' || i.status === 'completed') return false
  if (i.status === 'scheduled') return Date.parse(i.startsAt) <= now && (!i.endsAt || Date.parse(i.endsAt) > now)
  return true
}

/** Active first, then upcoming maintenance, then past (newest first). Past entries older than `days` are dropped. */
export function arrangeIncidents(list: Incident[], now: number, days = 30) {
  const cutoff = now - days * 86_400_000
  const active = list.filter((i) => isActive(i, now))
  const upcoming = list.filter((i) => i.status === 'scheduled' && Date.parse(i.startsAt) > now)
  const past = list
    .filter((i) => !active.includes(i) && !upcoming.includes(i))
    .filter((i) => Date.parse(i.endsAt ?? i.startsAt) >= cutoff)
  const byStart = (a: Incident, b: Incident) => Date.parse(b.startsAt) - Date.parse(a.startsAt)
  return {
    active: active.sort(byStart),
    upcoming: upcoming.sort((a, b) => Date.parse(a.startsAt) - Date.parse(b.startsAt)),
    past: past.sort(byStart),
  }
}

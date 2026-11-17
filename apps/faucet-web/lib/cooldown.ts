// Per-browser memory of recent claims, so the page can show "next claim in …" before the user
// spends a captcha solve. Convenience only: the Worker enforces the real 24 h limit per address AND IP.

export interface CooldownMap {
  /** lowercase recipient -> unix ms when the next claim is allowed */
  [recipient: string]: number
}

const KEY = 'xdevs-faucet-cooldowns-v1'

/** Drop expired entries and anything malformed. */
export function prune(map: unknown, now: number): CooldownMap {
  const out: CooldownMap = {}
  if (typeof map !== 'object' || map === null) return out
  for (const [k, v] of Object.entries(map as Record<string, unknown>)) {
    if (typeof v === 'number' && Number.isFinite(v) && v > now) out[k.toLowerCase()] = v
  }
  return out
}

export function withCooldown(map: CooldownMap, recipient: string, until: number, now: number): CooldownMap {
  return { ...prune(map, now), [recipient.toLowerCase()]: until }
}

/** Seconds until `recipient` may claim again (0 = now). */
export function remainingSeconds(map: CooldownMap, recipient: string, now: number): number {
  const until = map[recipient.toLowerCase()]
  if (!until || until <= now) return 0
  return Math.ceil((until - now) / 1000)
}

export function loadCooldowns(now = Date.now()): CooldownMap {
  try {
    const raw = globalThis.localStorage?.getItem(KEY)
    return raw ? prune(JSON.parse(raw), now) : {}
  } catch {
    return {}
  }
}

export function saveCooldowns(map: CooldownMap): void {
  try {
    globalThis.localStorage?.setItem(KEY, JSON.stringify(map))
  } catch {
    /* private mode / storage blocked: the page still works without it */
  }
}

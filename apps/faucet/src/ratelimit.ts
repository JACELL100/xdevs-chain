// Once-per-24h limits per recipient address AND per client IP, stored in Workers KV.
//
// ATOMICITY CAVEAT: Workers KV has no compare-and-set and is eventually consistent
// (a write can take ~60 s or more to become visible in other Cloudflare locations).
// We reduce the race by doing check+reserve *inside* the single FaucetSigner Durable
// Object, behind its mutex, so all claims are serialized in one location and see each
// other's writes. A residual window remains (KV's per-location cache can still serve a
// stale "absent" value), so in the worst case a user may get a second drip within 24h.
// The financial impact is bounded (worthless tXDS, small hot wallet, balance floor).
// If this ever matters, move the markers into the Durable Object's own storage, which is
// strongly consistent (see docs/runbooks/faucet.md "Limits").
//
// TODO(github-login): plan §20.3 also wants an optional per-GitHub-account limit.
// Add a GitHub OAuth flow (GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET), put the numeric
// GitHub user id in a signed session cookie, and add a third key `gh:<id>` here.

import { RATE_LIMIT_WINDOW_SECONDS } from "./env.ts";
import { HttpError } from "./http.ts";

interface Marker {
  at: number; // unix ms when the claim was reserved
}

export function addressKey(address: string): string {
  return `addr:${address.toLowerCase()}`;
}

/**
 * Key for a client IP. IPv4 is used as-is; IPv6 is truncated to its /64 prefix because
 * a single subscriber usually controls a whole /64 and could otherwise rotate addresses.
 */
export function ipKey(ip: string): string {
  const trimmed = ip.trim().toLowerCase();
  if (!trimmed.includes(":")) return `ip:${trimmed}`;
  const groups = expandIpv6(trimmed);
  if (!groups) return `ip:${trimmed}`;
  return `ip6:${groups.slice(0, 4).join(":")}::/64`;
}

function expandIpv6(ip: string): string[] | null {
  const noZone = ip.split("%")[0] ?? ip;
  const parts = noZone.split("::");
  if (parts.length > 2) return null;
  const head = parts[0] ? parts[0].split(":") : [];
  const tail = parts.length === 2 && parts[1] ? parts[1].split(":") : [];
  // IPv4-mapped tail (e.g. ::ffff:1.2.3.4) counts as two groups; we only need the /64, so keep a placeholder.
  const tailGroups = tail.flatMap((g) => (g.includes(".") ? ["0", "0"] : [g]));
  const missing = 8 - head.length - tailGroups.length;
  if (parts.length === 1 && missing !== 0) return null;
  if (missing < 0) return null;
  const all = [...head, ...Array<string>(missing).fill("0"), ...tailGroups];
  if (!all.every((g) => /^[0-9a-f]{1,4}$/.test(g))) return null;
  return all.map((g) => g.replace(/^0+(?=.)/, ""));
}

/** Throws 429 (with Retry-After) if any key was used in the last 24h. */
export async function assertNotLimited(kv: KVNamespace, keys: { key: string; label: string }[], now: number): Promise<void> {
  for (const { key, label } of keys) {
    const raw = await kv.get(key);
    if (raw === null) continue;
    let at = now;
    try {
      const m = JSON.parse(raw) as Marker;
      if (typeof m.at === "number") at = m.at;
    } catch {
      /* corrupt marker: treat as fresh */
    }
    const remainingMs = at + RATE_LIMIT_WINDOW_SECONDS * 1000 - now;
    if (remainingMs <= 0) continue; // KV TTL expiry is lazy; ignore markers older than the window
    const retryAfter = Math.ceil(remainingMs / 1000);
    const hours = Math.ceil(retryAfter / 3600);
    throw new HttpError(
      429,
      "rate_limited",
      `This ${label} already received tXDS in the last 24 hours. Try again in about ${hours} hour${hours === 1 ? "" : "s"}.`,
      { "Retry-After": String(retryAfter) },
    );
  }
}

export async function reserve(kv: KVNamespace, keys: string[], now: number): Promise<void> {
  const value = JSON.stringify({ at: now } satisfies Marker);
  await Promise.all(keys.map((k) => kv.put(k, value, { expirationTtl: RATE_LIMIT_WINDOW_SECONDS })));
}

export async function release(kv: KVNamespace, keys: string[]): Promise<void> {
  await Promise.all(keys.map((k) => kv.delete(k).catch(() => undefined)));
}

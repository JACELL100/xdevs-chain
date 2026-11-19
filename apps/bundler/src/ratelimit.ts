// Per-key token buckets (bounded memory) and client-IP resolution behind Cloudflare / a reverse proxy.
import type { IncomingMessage } from 'node:http'
import { isIP } from 'node:net'
import type { TrustProxy } from './config.ts'

export class TokenBuckets {
  private readonly buckets = new Map<string, { tokens: number; at: number }>()
  readonly ratePerSec: number
  readonly burst: number
  private readonly maxKeys: number
  private readonly now: () => number
  /**
   * @param ratePerSec refill rate
   * @param burst bucket size
   * @param maxKeys oldest keys are evicted beyond this (memory bound against IP spraying)
   */
  constructor(ratePerSec: number, burst: number, maxKeys = 20_000, now: () => number = Date.now) {
    this.ratePerSec = ratePerSec
    this.burst = burst
    this.maxKeys = maxKeys
    this.now = now
  }

  private refill(k: string): { tokens: number; at: number } {
    const t = this.now()
    let b = this.buckets.get(k)
    if (!b) {
      b = { tokens: this.burst, at: t }
      if (this.buckets.size >= this.maxKeys) {
        const oldest = this.buckets.keys().next().value
        if (oldest !== undefined) this.buckets.delete(oldest)
      }
    } else {
      this.buckets.delete(k) // re-insert: Map order = least recently used first
      b.tokens = Math.min(this.burst, b.tokens + ((t - b.at) / 1000) * this.ratePerSec)
      b.at = t
    }
    this.buckets.set(k, b)
    return b
  }

  /** Takes `n` tokens if available. */
  take(k: string, n = 1): boolean {
    const b = this.refill(k)
    if (b.tokens < n) return false
    b.tokens -= n
    return true
  }

  /** True if `n` tokens are available (does not consume). */
  check(k: string, n = 1): boolean {
    return this.refill(k).tokens >= n
  }

  /** Seconds until `n` tokens are available. */
  retryAfter(k: string, n = 1): number {
    const b = this.refill(k)
    return b.tokens >= n ? 0 : Math.ceil((n - b.tokens) / this.ratePerSec)
  }

  get size(): number {
    return this.buckets.size
  }
}

const normalize = (ip: string) => (ip.startsWith('::ffff:') && isIP(ip.slice(7)) === 4 ? ip.slice(7) : ip)

/**
 * The client IP used for rate limiting. Proxy headers are honoured only when the TCP peer is a trusted proxy
 * (cloudflared on localhost), so a client reaching the port directly cannot spoof another IP.
 * IPv6 clients are grouped by /64 (one subscriber usually owns a whole /64).
 */
export function clientIp(req: IncomingMessage, trust: TrustProxy, trustedProxies: string[]): string {
  const peer = normalize(req.socket.remoteAddress ?? 'unknown')
  let ip = peer
  if (trust !== 'none' && trustedProxies.map(normalize).includes(peer)) {
    const h = trust === 'cloudflare' ? req.headers['cf-connecting-ip'] : req.headers['x-forwarded-for']
    const v = typeof h === 'string' ? (trust === 'xff' ? h.split(',')[0] : h)?.trim() : undefined
    if (v && isIP(normalize(v))) ip = normalize(v)
  }
  if (isIP(ip) === 6) ip = `${ip.split(':').slice(0, 4).join(':')}::/64`
  return ip
}

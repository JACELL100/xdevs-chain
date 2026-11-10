// Guardian Vault alerts (plan §16.2): delivers the alert_outbox rows the indexer wrote in the same DB transaction as
// the block. At-least-once per destination, exponential backoff, at most MAX_ATTEMPTS. Secrets only from config/env and
// never logged (webhook URLs may embed tokens: logs name them by index only).

import { createHmac } from 'node:crypto'
import type { AlertConfig } from './config.ts'
import type { Db } from './db.ts'

export const MAX_ATTEMPTS = 8

export interface AlertPayload {
  type: string
  chainId: number
  account: string
  blockNumber: number
  txHash: string
  timestamp: number
  verified: boolean
  details: Record<string, unknown>
  explorerUrl: string | null
}

const fmtTime = (t: number) => new Date(t * 1000).toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC')

/** Plain-text Telegram / log message. No markup, so attacker-chosen data cannot inject formatting. */
export function formatAlertText(p: AlertPayload): string {
  const d = p.details
  const who = `account ${p.account}`
  const lines: string[] = []
  switch (p.type) {
    case 'vault.queued':
      lines.push(`xDEVS Guardian Vault: action #${BigInt(String(d.id))} was QUEUED on ${who}.`)
      lines.push(`Sends ${d.valueFormatted ?? d.value} to ${d.target}; executable after ${fmtTime(Number(d.executeAfter))}.`)
      lines.push('If this was not you, cancel it now from your wallet or ask your guardians to cancel it.')
      break
    case 'recovery.approved':
      lines.push(`xDEVS: a social RECOVERY of ${who} was approved by guardian ${d.guardian} (${d.approvals} approval(s)).`)
      lines.push('If you did not ask for a recovery, contact your guardians and cancel it.')
      break
    case 'recovery.ready':
      lines.push(`xDEVS: a social recovery of ${who} reached its threshold and can execute after ${fmtTime(Number(d.readyAt))}.`)
      break
    case 'recovery.executed':
      lines.push(`xDEVS: a social recovery of ${who} was EXECUTED. Its signer set was replaced.`)
      break
    case 'account.frozen':
      lines.push(`xDEVS: ${who} was FROZEN by its guardians until ${fmtTime(Number(d.frozenUntil))}.`)
      break
    case 'account.unfrozen':
      lines.push(`xDEVS: ${who} was unfrozen.`)
      break
    case 'eoaLock.changed':
      lines.push(`xDEVS: the XIP-0001 EOA lock of ${who} is now ${String(d.status).toUpperCase()}.`)
      break
    default:
      lines.push(`xDEVS alert ${p.type} on ${who}.`)
  }
  lines.push(`Block ${p.blockNumber}, tx ${p.txHash}.`)
  if (!p.verified) lines.push('Note: the emitting contract is not a verified xDEVS smart account.')
  if (p.explorerUrl) lines.push(p.explorerUrl)
  return lines.join('\n')
}

export function signBody(body: string, secret: string): string {
  return `sha256=${createHmac('sha256', secret).update(body).digest('hex')}`
}

export function nextDelayMs(attempts: number): number {
  return Math.min(30_000 * 2 ** Math.max(0, attempts - 1), 6 * 3600_000)
}

interface OutboxRow {
  id: number
  destination: string
  payload: AlertPayload
  attempts: number
}

export class AlertDispatcher {
  private readonly db: Db
  private readonly cfg: AlertConfig
  private readonly fetchFn: typeof fetch
  private readonly log: (m: string) => void
  private running = false
  private again = false

  constructor(db: Db, cfg: AlertConfig, fetchFn: typeof fetch = fetch, log: (m: string) => void = (m) => console.log(`[alerts] ${m}`)) {
    this.db = db
    this.cfg = cfg
    this.fetchFn = fetchFn
    this.log = log
  }

  /** Non-blocking trigger (called after each committed batch). */
  poke(): void {
    if (this.running) {
      this.again = true
      return
    }
    this.running = true
    void this.drain()
      .catch((e) => this.log(`dispatch failed: ${(e as Error).message}`))
      .finally(() => {
        this.running = false
        if (this.again) {
          this.again = false
          this.poke()
        }
      })
  }

  private async drain(): Promise<void> {
    while ((await this.runOnce()) > 0) {
      /* keep going while rows are due */
    }
  }

  /** Delivers due rows once. Returns how many rows were attempted. */
  async runOnce(limit = 50): Promise<number> {
    const rows = (
      await this.db.query<OutboxRow>(
        `SELECT id, destination, payload, attempts FROM alert_outbox
         WHERE delivered_at IS NULL AND NOT failed AND next_attempt_at <= now() ORDER BY id LIMIT $1`,
        [limit],
      )
    ).rows
    for (const row of rows) {
      const payload = typeof row.payload === 'string' ? (JSON.parse(row.payload) as AlertPayload) : row.payload
      try {
        await this.deliver(row.destination, payload)
        await this.db.query('UPDATE alert_outbox SET delivered_at = now(), attempts = attempts + 1, last_error = NULL WHERE id = $1', [row.id])
      } catch (err) {
        const attempts = Number(row.attempts) + 1
        const failed = attempts >= MAX_ATTEMPTS
        const msg = (err as Error).message.slice(0, 300)
        await this.db.query(
          `UPDATE alert_outbox SET attempts = $2, last_error = $3, failed = $4, next_attempt_at = now() + ($5 || ' milliseconds')::interval WHERE id = $1`,
          [row.id, attempts, msg, failed, String(nextDelayMs(attempts))],
        )
        this.log(`delivery to ${row.destination.split(':')[0]}${failed ? ' failed permanently' : ' failed, will retry'}: ${msg}`)
      }
    }
    return rows.length
  }

  private async deliver(destination: string, payload: AlertPayload): Promise<void> {
    const [kind, ...rest] = destination.split(':')
    const target = rest.join(':')
    if (kind === 'webhook') {
      const url = this.cfg.webhookUrls[Number(target)]
      if (!url) throw new Error(`webhook #${target} is no longer configured`)
      const body = JSON.stringify(payload)
      const headers: Record<string, string> = { 'content-type': 'application/json', 'user-agent': 'xdevs-indexer-alerts/0.1' }
      if (this.cfg.webhookSecret) headers['x-xdevs-signature'] = signBody(body, this.cfg.webhookSecret)
      const res = await this.fetchFn(url, { method: 'POST', headers, body, signal: AbortSignal.timeout(10_000) })
      if (!res.ok) throw new Error(`webhook #${target} answered HTTP ${res.status}`)
      return
    }
    if (kind === 'telegram') {
      if (!this.cfg.telegramBotToken) throw new Error('TELEGRAM_BOT_TOKEN is not set')
      const url = `${this.cfg.telegramApiBase}/bot${this.cfg.telegramBotToken}/sendMessage`
      const res = await this.fetchFn(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ chat_id: target, text: formatAlertText(payload), disable_web_page_preview: true }),
        signal: AbortSignal.timeout(10_000),
      })
      if (!res.ok) throw new Error(`telegram answered HTTP ${res.status}`) // never echo the URL: it contains the token
      return
    }
    throw new Error(`unknown destination kind "${kind}"`)
  }
}

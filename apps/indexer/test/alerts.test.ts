import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createHmac } from 'node:crypto'
import { AlertDispatcher, formatAlertText, MAX_ATTEMPTS, nextDelayMs, signBody, type AlertPayload } from '../src/alerts.ts'
import { cfg, freshDb, silent } from './helpers.ts'

const payload: AlertPayload = {
  type: 'vault.queued',
  chainId: 7117,
  account: '0xacc0000000000000000000000000000000000004',
  blockNumber: 12,
  txHash: `0x${'ab'.repeat(32)}`,
  timestamp: 1_800_000_000,
  verified: true,
  details: { id: '0x1', target: '0xb0b0000000000000000000000000000000000002', value: '0x1', valueFormatted: '5000 tXDS', executeAfter: 1_800_086_400 },
  explorerUrl: 'https://xdevschain-explorer.xdevs.in/address/0xacc0000000000000000000000000000000000004',
}

test('alert text is plain and actionable', () => {
  const text = formatAlertText(payload)
  assert.match(text, /action #1 was QUEUED/)
  assert.match(text, /5000 tXDS to 0xb0b0/)
  assert.match(text, /2027-01-16 08:00:00 UTC|UTC/)
  assert.match(text, /cancel it now/)
  assert.match(text, /xdevschain-explorer/)
  assert.match(formatAlertText({ ...payload, type: 'account.frozen', verified: false, details: { frozenUntil: 1_800_000_000 } }), /FROZEN[\s\S]*not a verified/)
})

test('HMAC signature and backoff', () => {
  assert.equal(signBody('{"a":1}', 's3cret'), `sha256=${createHmac('sha256', 's3cret').update('{"a":1}').digest('hex')}`)
  assert.equal(nextDelayMs(1), 30_000)
  assert.equal(nextDelayMs(2), 60_000)
  assert.equal(nextDelayMs(30), 6 * 3600_000)
})

test('dispatcher delivers webhooks (signed) and Telegram, retries failures, never leaks the bot token', async () => {
  const db = await freshDb()
  const seen: { url: string; headers: Record<string, string>; body: string }[] = []
  let failWebhook = true
  const fakeFetch = (async (url: string, init: RequestInit) => {
    seen.push({ url, headers: init.headers as Record<string, string>, body: String(init.body) })
    if (url.startsWith('https://hooks') && failWebhook) return new Response('nope', { status: 502 })
    return new Response('{}', { status: 200 })
  }) as unknown as typeof fetch
  const logs: string[] = []
  const alerts = { ...cfg().alerts, webhookUrls: ['https://hooks.example/secret-path'], webhookSecret: 'whsec', telegramBotToken: '123:SECRET' }
  const d = new AlertDispatcher(db, alerts, fakeFetch, (m) => logs.push(m))
  for (const dest of ['webhook:0', 'telegram:777'])
    await db.query(`INSERT INTO alert_outbox (dedupe_key, event, account, destination, payload) VALUES ($1, 'vault.queued', $2, $3, $4::jsonb)`, [
      `k:${dest}`, payload.account, dest, JSON.stringify(payload),
    ])

  assert.equal(await d.runOnce(), 2)
  const hook = seen.find((s) => s.url.startsWith('https://hooks'))!
  assert.equal(hook.headers['x-xdevs-signature'], signBody(hook.body, 'whsec'))
  assert.deepEqual(JSON.parse(hook.body), payload)
  const tg = seen.find((s) => s.url.includes('/bot123:SECRET/sendMessage'))!
  assert.equal(JSON.parse(tg.body).chat_id, '777')
  assert.ok(!logs.join('\n').includes('SECRET') && !logs.join('\n').includes('secret-path'), 'secrets never logged')

  let rows = (await db.query<{ destination: string; delivered_at: unknown; attempts: number; failed: boolean }>('SELECT * FROM alert_outbox ORDER BY id')).rows
  assert.equal(rows[0]!.delivered_at, null)
  assert.equal(Number(rows[0]!.attempts), 1)
  assert.notEqual(rows[1]!.delivered_at, null)

  // Not due yet (backoff), then due.
  assert.equal(await d.runOnce(), 0)
  await db.query(`UPDATE alert_outbox SET next_attempt_at = now() - interval '1 second'`)
  failWebhook = false
  assert.equal(await d.runOnce(), 1)
  rows = (await db.query<{ destination: string; delivered_at: unknown; attempts: number; failed: boolean }>('SELECT * FROM alert_outbox ORDER BY id')).rows
  assert.notEqual(rows[0]!.delivered_at, null)

  // Permanent failure after MAX_ATTEMPTS.
  await db.query(`INSERT INTO alert_outbox (dedupe_key, event, account, destination, payload, attempts) VALUES ('gone', 'vault.queued', $1, 'webhook:5', $2::jsonb, $3)`, [
    payload.account, JSON.stringify(payload), MAX_ATTEMPTS - 1,
  ])
  await d.runOnce()
  const gone = (await db.query<{ failed: boolean }>(`SELECT failed FROM alert_outbox WHERE dedupe_key = 'gone'`)).rows[0]!
  assert.equal(gone.failed, true)
  void silent
  await db.close()
})

// Operator CLI (writes to the DB the API never writes): alert subscriptions, labels, reset.
//   npm run cli -- subscribe <account> <telegramChatId>
//   npm run cli -- unsubscribe <account> <telegramChatId>
//   npm run cli -- label <address> "<label>"
//   npm run cli -- status
//   npm run cli -- reset --yes        (drops every table; the next start re-indexes from START_BLOCK)

import { isAddress } from 'viem'
import { loadConfig } from './config.ts'
import { createDb, migrate } from './db.ts'

const [cmd, ...args] = process.argv.slice(2)
const usage = 'usage: cli subscribe|unsubscribe <account> <chatId> | label <address> <text> | status | reset --yes'

async function main() {
  const cfg = loadConfig()
  const db = await createDb(cfg.databaseUrl)
  try {
    await migrate(db)
    const addr = (a: string | undefined) => {
      if (!a || !isAddress(a, { strict: false })) throw new Error(`not an address: ${a}\n${usage}`)
      return a.toLowerCase()
    }
    switch (cmd) {
      case 'subscribe': {
        const chat = args[1]
        if (!chat || !/^-?\d+$|^@[A-Za-z0-9_]{5,}$/.test(chat)) throw new Error('chat id must be a number or @channelname')
        await db.query(`INSERT INTO alert_subscriptions (account, channel, target) VALUES ($1, 'telegram', $2) ON CONFLICT DO NOTHING`, [addr(args[0]), chat])
        console.log('subscribed')
        break
      }
      case 'unsubscribe':
        await db.query(`DELETE FROM alert_subscriptions WHERE account = $1 AND channel = 'telegram' AND target = $2`, [addr(args[0]), args[1]])
        console.log('unsubscribed')
        break
      case 'label': {
        const text = args.slice(1).join(' ').trim().slice(0, 80)
        if (!text) throw new Error(usage)
        await db.query(`INSERT INTO labels (address, label) VALUES ($1, $2) ON CONFLICT (address) DO UPDATE SET label = EXCLUDED.label`, [addr(args[0]), text])
        console.log('labelled')
        break
      }
      case 'status': {
        const c = (await db.query('SELECT block_number, block_hash, updated_at FROM indexer_cursor')).rows[0]
        const s = (await db.query('SELECT * FROM chain_stats')).rows[0]
        const o = (await db.query(`SELECT count(*) FILTER (WHERE delivered_at IS NULL AND NOT failed) AS pending, count(*) FILTER (WHERE failed) AS failed FROM alert_outbox`)).rows[0]
        console.log(JSON.stringify({ cursor: c ?? null, stats: s, alerts: o }, null, 2))
        break
      }
      case 'reset':
        if (args[0] !== '--yes') throw new Error('reset drops all indexed data; run with --yes')
        await db.exec('DROP SCHEMA public CASCADE; CREATE SCHEMA public;')
        console.log('database reset')
        break
      default:
        throw new Error(usage)
    }
  } finally {
    await db.close()
  }
}

main().catch((err) => {
  console.error((err as Error).message)
  process.exit(1)
})

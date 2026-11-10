// Entry point. `node src/main.ts` runs migrations, then the indexer and/or the API (MODE=all|indexer|api).
// `node src/main.ts migrate` only migrates.

import { AlertDispatcher } from './alerts.ts'
import { createApiServer } from './api/server.ts'
import { loadConfig } from './config.ts'
import { createDb, migrate } from './db.ts'
import { FatalIndexerError, Indexer } from './indexer.ts'
import { HttpRpc } from './rpc.ts'

async function main() {
  const cfg = loadConfig()
  const db = await createDb(cfg.databaseUrl)
  const applied = await migrate(db)
  if (applied.length) console.log(`[db] applied migrations: ${applied.join(', ')}`)
  if (process.argv[2] === 'migrate') return db.close()

  const rpc = new HttpRpc(cfg.rpcUrl, { timeoutMs: cfg.rpcTimeoutMs, useBatch: cfg.rpcBatch })
  const ctrl = new AbortController()
  const stop = () => {
    console.log('shutting down…')
    ctrl.abort()
  }
  process.once('SIGINT', stop)
  process.once('SIGTERM', stop)

  const tasks: Promise<unknown>[] = []
  let timer: NodeJS.Timeout | undefined
  if (cfg.mode !== 'api') {
    const indexer = new Indexer(db, rpc, cfg)
    const alerts = new AlertDispatcher(db, cfg.alerts)
    indexer.onCommit = () => alerts.poke()
    timer = setInterval(() => alerts.poke(), 30_000) // retries of failed deliveries
    tasks.push(
      indexer.run(ctrl.signal).catch((err) => {
        console.error(`[indexer] ${err instanceof FatalIndexerError ? 'FATAL' : 'stopped'}: ${(err as Error).message}`)
        process.exitCode = 1
        ctrl.abort()
      }),
    )
  }
  if (cfg.mode !== 'indexer') {
    const server = createApiServer({ db, rpc, cfg })
    server.listen(cfg.port, cfg.host, () => console.log(`[api] listening on http://${cfg.host}:${cfg.port}`))
    tasks.push(new Promise<void>((resolve) => ctrl.signal.addEventListener('abort', () => server.close(() => resolve()), { once: true })))
  }
  await Promise.all(tasks)
  if (timer) clearInterval(timer)
  await db.close()
}

main().catch((err) => {
  console.error((err as Error).message)
  process.exit(1)
})

// Entry point: `node src/index.ts` with the configuration in the environment (see README / deploy/bundler/bundler.env.example).
import type { AddressInfo } from 'node:net'
import { Bundler } from './bundler.ts'
import { loadConfig, loadOperatorKey } from './config.ts'
import { createLogger } from './log.ts'
import { createBundlerServer } from './server.ts'

export { Bundler } from './bundler.ts'
export { createBundlerServer } from './server.ts'
export { loadConfig } from './config.ts'

export async function startBundler(env: Record<string, string | undefined> = process.env) {
  const cfg = loadConfig(env)
  const privateKey = loadOperatorKey(env)
  const log = createLogger(cfg.logLevel)
  const rpcUrl = env.RPC_URL ?? 'http://127.0.0.1:8545'
  const port = Number(env.PORT ?? 4337)
  const host = env.HOST ?? '127.0.0.1'
  const bundler = new Bundler({ rpcUrl, privateKey, config: cfg, logger: log })
  await bundler.init()
  bundler.start()
  const server = createBundlerServer(bundler, { logger: log })
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, host, resolve)
  })
  const addr = server.address() as AddressInfo
  log.info('listening', { url: `http://${addr.address}:${addr.port}`, trustProxy: cfg.trustProxy, cors: cfg.corsOrigins })
  let stopped: Promise<void> | undefined
  const stop = () => {
    stopped ??= (async () => {
      log.info('shutting down')
      await new Promise<void>((r) => {
        server.close(() => r())
        server.closeIdleConnections()
        setTimeout(r, 5_000).unref()
      })
      await bundler.stop()
      log.info('stopped')
    })()
    return stopped
  }
  return { bundler, server, url: `http://${addr.address}:${addr.port}`, stop }
}

if (import.meta.main) {
  startBundler()
    .then(({ stop }) => {
      const onSignal = (sig: string) => {
        void stop().then(
          () => process.exit(0),
          (e) => {
            console.error(JSON.stringify({ level: 'error', msg: 'shutdown failed', signal: sig, error: (e as Error).message }))
            process.exit(1)
          },
        )
      }
      process.once('SIGTERM', () => onSignal('SIGTERM'))
      process.once('SIGINT', () => onSignal('SIGINT'))
    })
    .catch((e) => {
      console.error(JSON.stringify({ level: 'error', msg: 'startup failed', error: (e as Error).message }))
      process.exit(1)
    })
}

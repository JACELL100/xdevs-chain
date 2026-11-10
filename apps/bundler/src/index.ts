// Entry point: `BUNDLER_PRIVATE_KEY=0x… RPC_URL=http://127.0.0.1:8545 node src/index.ts`
// DEV ONLY: single operator, no ERC-7562 rules, no mempool. Bind to localhost; never expose publicly.
import type { AddressInfo } from 'node:net'
import type { Address, Hex } from 'viem'
import { Bundler } from './bundler.ts'
import { createBundlerServer } from './server.ts'

export { Bundler } from './bundler.ts'
export { createBundlerServer } from './server.ts'

export async function startBundler(env: Record<string, string | undefined> = process.env) {
  const privateKey = env.BUNDLER_PRIVATE_KEY as Hex | undefined
  if (!privateKey || !/^0x[0-9a-fA-F]{64}$/.test(privateKey)) {
    throw new Error('BUNDLER_PRIVATE_KEY (0x + 64 hex) is required: a funded key of the dev chain')
  }
  const rpcUrl = env.RPC_URL ?? 'http://127.0.0.1:8545'
  const port = Number(env.PORT ?? 4337)
  const host = env.HOST ?? '127.0.0.1'
  const bundler = new Bundler({
    rpcUrl,
    privateKey,
    entryPoint: env.ENTRY_POINT as Address | undefined,
    beneficiary: env.BENEFICIARY as Address | undefined,
    log: env.QUIET ? () => {} : undefined,
  })
  await bundler.init()
  const server = createBundlerServer(bundler)
  await new Promise<void>((resolve) => server.listen(port, host, resolve))
  const addr = server.address() as AddressInfo
  if (!env.QUIET) console.log(`[bundler] DEV-ONLY bundler listening on http://${addr.address}:${addr.port} (RPC ${rpcUrl})`)
  return { bundler, server, url: `http://${addr.address}:${addr.port}` }
}

if (import.meta.main) {
  startBundler().catch((e) => {
    console.error(`[bundler] ${(e as Error).message}`)
    process.exit(1)
  })
}

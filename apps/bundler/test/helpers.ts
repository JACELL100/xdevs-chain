// Shared test fixture: a dev chain (xDEVS node when its binary is available, else anvil) + devnet setup.
import { createServer } from 'node:net'
import { type Chain, type Devnet, setupDevnet, startChain } from '../scripts/devnet.ts'

export async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const s = createServer()
    s.once('error', reject)
    s.listen(0, '127.0.0.1', () => {
      const port = (s.address() as { port: number }).port
      s.close(() => resolve(port))
    })
  })
}

/** `XDEVS_TEST_CHAIN=anvil|xdevs|auto` (default auto: the xDEVS node if found). */
export async function devChain(opts: { token?: boolean } = {}): Promise<{ chain: Chain; devnet: Devnet }> {
  const kind = (process.env.XDEVS_TEST_CHAIN ?? 'auto') as 'xdevs' | 'anvil' | 'auto'
  const chain = await startChain(kind, await freePort())
  try {
    const devnet = await setupDevnet(chain.rpcUrl, opts)
    return { chain, devnet }
  } catch (e) {
    await chain.stop()
    throw e
  }
}

export async function jsonRpc(url: string, method: string, params: unknown[] = []): Promise<{ result?: any; error?: any }> {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  })
  return (await res.json()) as { result?: any; error?: any }
}

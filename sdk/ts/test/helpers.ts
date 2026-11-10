import { type Chain, createPublicClient, custom } from 'viem'
import { xdevsActions, xdevsDevnet } from '../src/index.ts'

export type Call = { method: string; params: unknown }

/**
 * A public client whose transport never touches the network: every request is recorded in `calls`
 * and answered by `handler` (throw to simulate a JSON-RPC error).
 */
export function mockClient(handler: (call: Call) => unknown, chain: Chain = xdevsDevnet) {
  const calls: Call[] = []
  const client = createPublicClient({
    chain,
    transport: custom(
      {
        async request({ method, params }: { method: string; params?: unknown }) {
          const call = { method, params }
          calls.push(call)
          return handler(call)
        },
      },
      { retryCount: 0 },
    ),
  }).extend(xdevsActions)
  return { client, calls }
}

/** A JSON-RPC error object as an EIP-1193 provider would throw it. */
export function rpcError(code: number, message: string): Error {
  return Object.assign(new Error(message), { code })
}

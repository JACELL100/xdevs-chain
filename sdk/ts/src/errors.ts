import { BaseError } from 'viem'
import { XDEVS_RPC_METHODS } from './methods.ts'
import type { XdevsRpcMethod } from './types.ts'

/** JSON-RPC 2.0 "Method not found". */
const METHOD_NOT_FOUND = -32601
/** EIP-1474 "Method not supported". */
const METHOD_NOT_SUPPORTED = -32004
const METHOD_NOT_FOUND_MESSAGE = /method[^]*(not found|does not exist|not available|not supported|unsupported)/i

/**
 * Thrown when the connected node does not (yet) implement an `xdevs_` method — typically because the
 * node predates the roadmap phase that adds it (see {@link XDEVS_RPC_METHODS}).
 */
export class XdevsMethodNotAvailableError extends BaseError {
  readonly method: XdevsRpcMethod
  readonly phase: number

  constructor(method: XdevsRpcMethod, cause?: Error) {
    const { phase, spec } = XDEVS_RPC_METHODS[method]
    super(`The connected node does not implement \`${method}\`.`, {
      name: 'XdevsMethodNotAvailableError',
      cause,
      metaMessages: [
        `\`${method}\` (plan ${spec}) ships in xDEVS Chain Phase ${phase} (plan §19.2).`,
        'Upgrade the node, or connect to an RPC endpoint that runs a Phase ' + phase + '+ xDEVS node.',
      ],
    })
    this.method = method
    this.phase = phase
  }
}

/** Thrown for XNS names that fail the `.xdevs` label rules (mirrors `XNSLabel.sol`). */
export class InvalidXnsNameError extends BaseError {
  readonly input: string
  readonly reason: string

  constructor(input: string, reason: string) {
    super(`Invalid XNS name ${JSON.stringify(input)}: ${reason}.`, { name: 'InvalidXnsNameError' })
    this.input = input
    this.reason = reason
  }
}

/** True if `error` (or any error in its `cause` chain) is a JSON-RPC "method not found/supported" error. */
export function isMethodNotFoundError(error: unknown): boolean {
  let e: unknown = error
  for (let depth = 0; e && depth < 16; depth++) {
    if (typeof e === 'object') {
      const code = (e as { code?: unknown }).code
      if (code === METHOD_NOT_FOUND || code === METHOD_NOT_SUPPORTED) return true
      const message = (e as { message?: unknown }).message
      // Some servers report unknown methods as -32600/-32603/-32000 with a "method ... not found" message.
      if (
        (code === -32600 || code === -32603 || code === -32000) &&
        typeof message === 'string' &&
        !/revert/i.test(message) &&
        METHOD_NOT_FOUND_MESSAGE.test(message)
      )
        return true
      e = (e as { cause?: unknown }).cause
    } else break
  }
  return false
}

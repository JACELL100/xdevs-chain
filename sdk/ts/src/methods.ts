import type { XdevsRpcMethod } from './types.ts'

export type XdevsMethodInfo = {
  /** Roadmap phase (plan §27) in which the xDEVS node starts serving the method (plan §19.2). */
  phase: 2 | 3 | 4 | 5
  /** Plan section describing the feature. */
  spec: string
  description: string
}

/**
 * Every `xdevs_` JSON-RPC method this SDK knows, with the phase in which the node implements it
 * (plan §19.2). Calling a method on a node older than its phase raises {@link XdevsMethodNotAvailableError}.
 *
 * Not wrapped yet: `xdevs_subscribe("vaultEvents")` (Phase 4, WebSocket only).
 */
export const XDEVS_RPC_METHODS = {
  xdevs_getCommitCertificate: { phase: 2, spec: '§8.1', description: 'Commit certificate for a height' },
  xdevs_validators: { phase: 3, spec: '§11', description: 'Active validator set, power, uptime' },
  xdevs_epochInfo: { phase: 3, spec: '§11.5', description: 'Current epoch and next transition' },
  xdevs_simulateTransaction: { phase: 4, spec: '§16.4', description: 'Simulate a tx and return asset changes and warnings' },
  xdevs_checkRecipient: { phase: 4, spec: '§16.3', description: 'First-time / look-alike / XNS / contract checks for a recipient' },
  xdevs_getApprovals: { phase: 4, spec: '§16.5', description: 'Live token approvals of an owner' },
  xdevs_getVaultQueue: { phase: 4, spec: '§16.2', description: 'Pending Guardian Vault actions' },
  xdevs_resolveName: { phase: 4, spec: '§16.3', description: 'XNS name → address' },
  xdevs_reverseName: { phase: 4, spec: '§16.3', description: 'Address → primary XNS name' },
  xdevs_localFee: { phase: 5, spec: '§15.2', description: 'Local fee market price for a contract' },
} as const satisfies Record<XdevsRpcMethod, XdevsMethodInfo>

import type { Address } from 'viem'

/**
 * Fixed genesis addresses of the xDEVS Chain system contracts (plan §18, §18.2, §10.3).
 *
 * Must stay byte-for-byte in sync with `contracts/src/common/Predeploys.sol`
 * (enforced by `test/predeploys.test.ts`, which parses the Solidity file).
 */
export const predeploys = {
  /** Caller used for protocol-initiated system calls (`msg.sender` of `onEpoch`, etc.). Not a contract. */
  SYSTEM_ADDRESS: '0xffffFFFfFFffffffffffffffFfFFFfffFFFfFFfE',
  /** Validators, delegation, rewards, slashing, epoch transitions. */
  STAKING: '0x0000000000000000000000000000000000001000',
  /** Proposals, voting by stake, timelocked execution. */
  GOVERNANCE: '0x0000000000000000000000000000000000001001',
  /** Receives 20% of base fees; grants by governance. */
  TREASURY: '0x0000000000000000000000000000000000001002',
  /** Gas limit, min base fee, local fee params. */
  FEE_CONFIG: '0x0000000000000000000000000000000000001003',
  /** `.xdevs` name registry (ENS-compatible). */
  XNS_REGISTRY: '0x0000000000000000000000000000000000001004',
  /** xDEVS Smart Account implementation (EIP-7702 delegation target). */
  SMART_ACCOUNT_IMPL: '0x0000000000000000000000000000000000001005',
  /** Deterministic smart-account creation. */
  SMART_ACCOUNT_FACTORY: '0x0000000000000000000000000000000000001006',
  /** Gas sponsorship. */
  PAYMASTER: '0x0000000000000000000000000000000000001007',
  /** Permit2-style expiring allowances for any ERC-20. */
  PERMIT2_EXPIRING: '0x0000000000000000000000000000000000001008',
  /** Wrapped XDS (ERC-20). */
  WXDS: '0x0000000000000000000000000000000000001009',
  /** RIP-7212 / EIP-7951 P-256 signature verification precompile. */
  P256_VERIFY: '0x0000000000000000000000000000000000000100',
} as const satisfies Record<string, Address>

export type PredeployName = keyof typeof predeploys

/** Canonical Multicall3 address (same on every EVM chain; inserted in xDEVS genesis, plan §18). */
export const MULTICALL3_ADDRESS = '0xcA11bde05977b3631167028862bE2a173976CA11' as const satisfies Address

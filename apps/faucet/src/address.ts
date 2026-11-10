// Recipient validation. Checksum-agnostic: any-case hex is accepted and normalized to lowercase.

import { HttpError } from "./http.ts";

const HEX_ADDRESS = /^0x[0-9a-fA-F]{40}$/;
/** XNS names: label(s) + ".xdevs" (plan §1.4 / §16.3). */
const XNS_NAME = /^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+xdevs$/;

/** Protocol system caller (plan §18: msg.sender == 0xffff…fffe). */
export const SYSTEM_CALLER = "0xfffffffffffffffffffffffffffffffffffffffe";

/**
 * Highest "reserved low" address. Everything in 0x0…0000–0x0…ffff is refused:
 * the zero address, EVM precompiles (0x01–0x11, 0x100) and the xDEVS system
 * predeploys 0x…1000–0x…1009 (Staking, Governance, Treasury, FeeConfig,
 * XNSRegistry, XdevsSmartAccount impl, SmartAccountFactory, Paymaster,
 * Permit2Expiring, WXDS) plus room for future predeploys. No real key-derived
 * address lives in this range.
 */
const RESERVED_LOW_MAX = 0xffffn;

export type Recipient = { kind: "address"; address: `0x${string}` } | { kind: "name"; name: string };

export function parseRecipient(raw: unknown): Recipient {
  if (typeof raw !== "string") throw new HttpError(400, "invalid_address", "Provide a wallet address (0x…) or a .xdevs name.");
  const input = raw.trim();
  if (input.length === 0 || input.length > 256) {
    throw new HttpError(400, "invalid_address", "Provide a wallet address (0x…) or a .xdevs name.");
  }
  const lower = input.toLowerCase();
  if (lower.endsWith(".xdevs")) {
    if (!XNS_NAME.test(lower)) throw new HttpError(400, "invalid_name", "That is not a valid .xdevs name.");
    return { kind: "name", name: lower };
  }
  if (!HEX_ADDRESS.test(input)) {
    throw new HttpError(400, "invalid_address", "That is not a valid address: expected 0x followed by 40 hex characters.");
  }
  return { kind: "address", address: lower as `0x${string}` };
}

/** Throws 400 if the (lowercase) address must never receive faucet funds. */
export function assertAllowedRecipient(address: `0x${string}`, faucetAddress?: string): void {
  const value = BigInt(address);
  if (value === 0n) throw new HttpError(400, "reserved_address", "The zero address cannot receive tokens.");
  if (value <= RESERVED_LOW_MAX) {
    throw new HttpError(400, "reserved_address", "That is a system contract or precompile address, not a wallet.");
  }
  if (address === SYSTEM_CALLER) {
    throw new HttpError(400, "reserved_address", "That is the protocol system address, not a wallet.");
  }
  if (faucetAddress && address === faucetAddress.toLowerCase()) {
    throw new HttpError(400, "reserved_address", "That is the faucet's own address.");
  }
}

/**
 * Resolve a .xdevs name to an address.
 * TODO(XNS): implement once the node exposes `xdevs_resolveName` (plan §16.3 / §19 RPC table).
 * Until then names are rejected with a clear message so the UI can ask for a 0x address.
 */
export async function resolveName(_name: string): Promise<`0x${string}`> {
  throw new HttpError(
    400,
    "names_not_supported",
    ".xdevs names are not supported by the faucet yet (XNS is not live). Please paste your 0x address.",
  );
}

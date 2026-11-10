// viem plumbing: config parsing, faucet account, JSON-RPC client.
// The private key is only ever passed to privateKeyToAccount(); it is never logged,
// echoed in errors, or returned by any endpoint.

import { createPublicClient, defineChain, formatEther, http, parseEther, type PublicClient } from "viem";
import { privateKeyToAccount, type PrivateKeyAccount } from "viem/accounts";
import { DEFAULT_DRIP_TXDS, DEFAULT_MIN_BALANCE_TXDS, type Env } from "./env.ts";
import { HttpError } from "./http.ts";

export const SYMBOL = "tXDS";

export interface FaucetConfig {
  chainId: number;
  rpcUrl: string;
  dripWei: bigint;
  minBalanceWei: bigint;
}

function misconfigured(what: string): HttpError {
  // `what` names the setting only, never its value.
  console.error(`faucet misconfigured: ${what}`);
  return new HttpError(503, "misconfigured", "Faucet is temporarily unavailable.");
}

function parseTxds(value: string | undefined, fallback: string, name: string): bigint {
  const v = (value ?? "").trim() || fallback;
  if (!/^\d+(\.\d{1,18})?$/.test(v)) throw misconfigured(name);
  return parseEther(v);
}

export function loadConfig(env: Env): FaucetConfig {
  const chainId = Number(env.CHAIN_ID);
  if (!Number.isSafeInteger(chainId) || chainId <= 0) throw misconfigured("CHAIN_ID");
  let rpcUrl: URL;
  try {
    rpcUrl = new URL(env.RPC_URL);
  } catch {
    throw misconfigured("RPC_URL");
  }
  if (rpcUrl.protocol !== "https:" && rpcUrl.protocol !== "http:") throw misconfigured("RPC_URL");
  const dripWei = parseTxds(env.DRIP_AMOUNT, DEFAULT_DRIP_TXDS, "DRIP_AMOUNT");
  if (dripWei === 0n) throw misconfigured("DRIP_AMOUNT");
  const minBalanceWei = parseTxds(env.MIN_FAUCET_BALANCE, DEFAULT_MIN_BALANCE_TXDS, "MIN_FAUCET_BALANCE");
  return { chainId, rpcUrl: rpcUrl.toString(), dripWei, minBalanceWei };
}

export function loadAccount(env: Env): PrivateKeyAccount {
  const key = (env.FAUCET_PRIVATE_KEY ?? "").trim();
  // Validate the shape ourselves so a malformed key never reaches a library error message.
  if (!/^0x[0-9a-fA-F]{64}$/.test(key)) throw misconfigured("FAUCET_PRIVATE_KEY (expected 0x + 64 hex)");
  try {
    return privateKeyToAccount(key as `0x${string}`);
  } catch {
    throw misconfigured("FAUCET_PRIVATE_KEY (not a valid secp256k1 key)");
  }
}

export function makeClient(cfg: FaucetConfig): PublicClient {
  const chain = defineChain({
    id: cfg.chainId,
    name: "xDEVS Chain",
    nativeCurrency: { name: "Testnet XDS", symbol: SYMBOL, decimals: 18 },
    rpcUrls: { default: { http: [cfg.rpcUrl] } },
  });
  return createPublicClient({
    chain,
    transport: http(cfg.rpcUrl, { retryCount: 1, timeout: 10_000 }),
  });
}

export function rpcUnavailable(err: unknown): HttpError {
  // Log a short reason only; viem's long messages include request bodies (signed txs are public, but keep logs lean).
  const e = err as { shortMessage?: string; name?: string } | undefined;
  console.error(`rpc error: ${e?.name ?? "Error"}: ${e?.shortMessage ?? "unknown"}`);
  return new HttpError(503, "rpc_unavailable", "The xDEVS Chain RPC node is unavailable right now. Please try again later.");
}

export function formatTxds(wei: bigint): string {
  return formatEther(wei);
}

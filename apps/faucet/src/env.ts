// Bindings, vars and secrets for the tXDS faucet Worker.
// See wrangler.toml and docs/runbooks/faucet.md for how each one is set.

/** Minimal structural type for the Durable Object namespace we use (keeps tests runtime-agnostic). */
export interface SignerNamespace {
  idFromName(name: string): DurableObjectId;
  get(id: DurableObjectId): { fetch(request: Request): Promise<Response> };
}

export interface Env {
  // --- bindings ---
  /** KV namespace holding per-address / per-IP rate-limit markers (24 h TTL). */
  RATE_LIMIT: KVNamespace;
  /** Durable Object that serializes every drip so nonces never collide. */
  FAUCET_SIGNER: SignerNamespace;

  // --- secrets (wrangler secret put ...) ---
  /** 0x-prefixed 32-byte hex private key of the faucet HOT wallet. Never logged. */
  FAUCET_PRIVATE_KEY: string;
  /** Cloudflare Turnstile secret key (server side). */
  TURNSTILE_SECRET_KEY: string;

  // --- vars (wrangler.toml [vars]) ---
  /** JSON-RPC endpoint of an xDEVS Chain node (public RPC full node). */
  RPC_URL: string;
  /** Decimal chain id, e.g. "717171" for xdevs-testnet-1. */
  CHAIN_ID: string;
  /** Turnstile site key (public, embedded in the page). */
  TURNSTILE_SITE_KEY: string;
  /** Amount per claim in whole tXDS (decimal string). Default "100". */
  DRIP_AMOUNT?: string;
  /** Refuse to drip when the faucet balance (tXDS) is below this. Default "1000". */
  MIN_FAUCET_BALANCE?: string;
  /** Comma-separated list of origins allowed to call /api/* cross-origin (explorer, wallet). */
  CORS_ORIGINS?: string;
  /** Explorer tx URL template, "{hash}" is replaced, e.g. "https://explorer.testnet.example/tx/{hash}". */
  EXPLORER_TX_URL?: string;
  /** Optional: if set, Turnstile responses must report this hostname. */
  TURNSTILE_EXPECTED_HOSTNAME?: string;
}

export const DEFAULT_DRIP_TXDS = "100";
export const DEFAULT_MIN_BALANCE_TXDS = "1000";
export const RATE_LIMIT_WINDOW_SECONDS = 24 * 60 * 60;

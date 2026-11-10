// tXDS faucet for xDEVS Chain — Cloudflare Worker entry point (module syntax).
//
//   GET  /            HTML form (address / .xdevs name + Turnstile)
//   POST /api/claim   {address, turnstileToken} -> {ok, txHash, ...}
//   GET  /api/status  faucet address, balance, drip amount, chain id (no secrets)
//
// Policy (plan §20.3): DRIP_AMOUNT (default 100 tXDS) once per 24h per address AND per IP,
// Turnstile captcha, small hot wallet with a balance floor (plan §25.1 "Faucet draining").

import { assertAllowedRecipient, parseRecipient, resolveName } from "./address.ts";
import { formatTxds, loadAccount, loadConfig, makeClient, rpcUnavailable, SYMBOL } from "./chain.ts";
import type { Env } from "./env.ts";
import { corsHeaders, errorResponse, HttpError, json, withHeaders } from "./http.ts";
import { renderPage } from "./page.ts";
import { ipKey } from "./ratelimit.ts";
import type { DripRequest, DripResult } from "./signer.ts";
import { verifyTurnstile } from "./turnstile.ts";

export { FaucetSigner } from "./signer.ts";
export type { Env } from "./env.ts";

/** Single global signer instance: one nonce sequence for the one hot wallet. */
export const SIGNER_NAME = "faucet-hot-wallet";
const MAX_BODY_BYTES = 4096;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const isApi = url.pathname.startsWith("/api/");
    const cors = isApi ? corsHeaders(request, env.CORS_ORIGINS) : {};
    try {
      const res = await route(request, env, url);
      return withHeaders(res, cors);
    } catch (err) {
      if (err instanceof HttpError) return withHeaders(errorResponse(err), cors);
      console.error(`unhandled error: ${(err as Error)?.name ?? "Error"}`);
      return withHeaders(errorResponse(new HttpError(500, "internal", "Unexpected error.")), cors);
    }
  },
} satisfies ExportedHandler<Env>;

async function route(request: Request, env: Env, url: URL): Promise<Response> {
  const { pathname } = url;
  const method = request.method;

  if (method === "OPTIONS" && pathname.startsWith("/api/")) {
    return new Response(null, { status: 204 });
  }
  if (pathname === "/" || pathname === "/index.html") {
    if (method !== "GET" && method !== "HEAD") throw methodNotAllowed("GET");
    let drip = "100";
    try {
      drip = formatTxds(loadConfig(env).dripWei);
    } catch {
      /* page still renders; claim will report the misconfiguration */
    }
    return renderPage({ siteKey: env.TURNSTILE_SITE_KEY ?? "", dripAmount: drip });
  }
  if (pathname === "/api/claim") {
    if (method !== "POST") throw methodNotAllowed("POST");
    return handleClaim(request, env);
  }
  if (pathname === "/api/status") {
    if (method !== "GET") throw methodNotAllowed("GET");
    return handleStatus(env);
  }
  throw new HttpError(404, "not_found", "Not found.");
}

function methodNotAllowed(allow: string): HttpError {
  return new HttpError(405, "method_not_allowed", "Method not allowed.", { Allow: allow });
}

async function readJsonBody(request: Request): Promise<Record<string, unknown>> {
  const len = Number(request.headers.get("Content-Length") ?? "0");
  if (len > MAX_BODY_BYTES) throw new HttpError(400, "body_too_large", "Request body too large.");
  const text = await request.text();
  if (text.length > MAX_BODY_BYTES) throw new HttpError(400, "body_too_large", "Request body too large.");
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    throw new HttpError(400, "invalid_json", "Request body must be JSON: {\"address\": \"0x…\", \"turnstileToken\": \"…\"}.");
  }
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    throw new HttpError(400, "invalid_json", "Request body must be a JSON object.");
  }
  return body as Record<string, unknown>;
}

async function handleClaim(request: Request, env: Env): Promise<Response> {
  const ip = request.headers.get("CF-Connecting-IP");
  if (!ip) throw new HttpError(400, "missing_client_ip", "Could not determine your IP address.");

  const body = await readJsonBody(request);

  // 1. Validate the recipient (cheap, before spending a captcha verification).
  const recipient = parseRecipient(body.address);
  const address = recipient.kind === "address" ? recipient.address : await resolveName(recipient.name);
  assertAllowedRecipient(address);

  // 2. Captcha (server-side; the token is single use).
  await verifyTurnstile({
    token: body.turnstileToken,
    secret: env.TURNSTILE_SECRET_KEY,
    remoteIp: ip,
    expectedHostname: env.TURNSTILE_EXPECTED_HOSTNAME,
  });

  // 3. Rate limit + balance check + sign + send, serialized in the signer Durable Object.
  const payload: DripRequest = { address, ipKey: ipKey(ip) };
  const stub = env.FAUCET_SIGNER.get(env.FAUCET_SIGNER.idFromName(SIGNER_NAME));
  const res = await stub.fetch(
    new Request("https://faucet-signer.internal/drip", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  );
  const data = (await res.json()) as (DripResult & { explorerUrl?: string | null }) | { ok: false };
  if (data.ok) data.explorerUrl = explorerUrl(env.EXPLORER_TX_URL, data.txHash);
  const passHeaders: Record<string, string> = {};
  const retryAfter = res.headers.get("Retry-After");
  if (retryAfter) passHeaders["Retry-After"] = retryAfter;
  return json(data, res.status, passHeaders);
}

export function explorerUrl(template: string | undefined, hash: string): string | null {
  if (!template || !template.includes("{hash}")) return null;
  const url = template.replace("{hash}", hash);
  return url.startsWith("https://") || url.startsWith("http://") ? url : null;
}

async function handleStatus(env: Env): Promise<Response> {
  const cfg = loadConfig(env);
  const account = loadAccount(env);
  const base = {
    address: account.address,
    chainId: cfg.chainId,
    symbol: SYMBOL,
    dripAmount: formatTxds(cfg.dripWei),
    minBalance: formatTxds(cfg.minBalanceWei),
    rateLimit: "1 claim per 24h per address and per IP",
  };
  let balanceWei: bigint;
  try {
    balanceWei = await makeClient(cfg).getBalance({ address: account.address });
  } catch (err) {
    rpcUnavailable(err); // logs a short reason
    return json({ ok: false, ...base, balance: null, healthy: false, error: "rpc_unavailable" }, 503);
  }
  return json({
    ok: true,
    ...base,
    balance: formatTxds(balanceWei),
    balanceWei: balanceWei.toString(),
    healthy: balanceWei >= cfg.minBalanceWei && balanceWei >= cfg.dripWei,
  });
}

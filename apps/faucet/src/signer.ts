// FaucetSigner Durable Object: the ONLY place that signs and sends faucet transactions.
//
// Why a Durable Object: Workers isolates run concurrently all over the world, so two
// claims could read the same pending nonce and one would be rejected (or replace the
// other). Workers KV cannot act as a lock (no compare-and-set, eventually consistent).
// A single, named Durable Object instance is single-threaded and lives in one location,
// and we additionally chain every drip through an in-memory promise mutex because a
// DO *can* interleave requests while it awaits outbound fetch() calls (RPC, KV).
//
// Nonce strategy: nonce = max(eth_getTransactionCount(faucet, "pending"), lastUsed + 1).
// The in-memory `nextNonce` covers nodes whose "pending" count lags the mempool; the RPC
// value covers DO restarts/evictions and txs sent from elsewhere (e.g. a manual refill test).
// Limitation: if a broadcast tx is dropped from the mempool, later txs queue behind the
// nonce gap until it is re-sent or the DO restarts and the node's pending count is used
// again. See docs/runbooks/faucet.md "Stuck nonce".
//
// Old-style DO class (plain class with fetch) so the module also loads under Node for tests.

import { assertAllowedRecipient } from "./address.ts";
import { formatTxds, loadAccount, loadConfig, makeClient, rpcUnavailable, SYMBOL } from "./chain.ts";
import type { Env } from "./env.ts";
import { errorResponse, HttpError, json } from "./http.ts";
import { addressKey, assertNotLimited, release, reserve } from "./ratelimit.ts";

export interface DripRequest {
  address: `0x${string}`;
  ipKey: string;
}

export interface DripResult {
  ok: true;
  txHash: `0x${string}`;
  to: `0x${string}`;
  amount: string;
  amountWei: string;
  symbol: string;
  nonce: number;
}

export class FaucetSigner {
  private readonly env: Env;
  private queue: Promise<unknown> = Promise.resolve();
  private nextNonce: number | null = null;
  private chainVerified = false;

  constructor(_state: DurableObjectState, env: Env) {
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    if (request.method !== "POST") return errorResponse(new HttpError(405, "method_not_allowed", "POST only."));
    let body: DripRequest;
    try {
      body = (await request.json()) as DripRequest;
    } catch {
      return errorResponse(new HttpError(400, "invalid_json", "Bad request."));
    }
    try {
      const result = await this.exclusive(() => this.drip(body));
      return json(result);
    } catch (err) {
      if (err instanceof HttpError) return errorResponse(err);
      console.error(`signer error: ${(err as Error)?.name ?? "Error"}`);
      return errorResponse(new HttpError(503, "internal", "Faucet is temporarily unavailable."));
    }
  }

  /** Serialize `fn` behind every previously queued call. */
  private exclusive<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.queue.then(fn, fn);
    this.queue = run.catch(() => undefined);
    return run;
  }

  private async drip(req: DripRequest): Promise<DripResult> {
    const env = this.env;
    const cfg = loadConfig(env);
    const account = loadAccount(env);
    const to = req.address.toLowerCase() as `0x${string}`;
    assertAllowedRecipient(to, account.address);

    const now = Date.now();
    const keys = [addressKey(to), req.ipKey];
    await assertNotLimited(
      env.RATE_LIMIT,
      [
        { key: keys[0]!, label: "address" },
        { key: keys[1]!, label: "IP address" },
      ],
      now,
    );

    const client = makeClient(cfg);

    let balance: bigint;
    let pendingNonce: number;
    let fees: { maxFeePerGas: bigint; maxPriorityFeePerGas: bigint };
    let gas: bigint;
    try {
      if (!this.chainVerified) {
        const remoteId = await client.getChainId();
        if (remoteId !== cfg.chainId) {
          console.error(`chain id mismatch: RPC reports ${remoteId}, CHAIN_ID is ${cfg.chainId}`);
          throw new HttpError(503, "misconfigured", "Faucet is temporarily unavailable.");
        }
        this.chainVerified = true;
      }
      balance = await client.getBalance({ address: account.address, blockTag: "latest" });
      pendingNonce = await client.getTransactionCount({ address: account.address, blockTag: "pending" });
      fees = await client.estimateFeesPerGas();
      gas = await client.estimateGas({ account: account.address, to, value: cfg.dripWei });
    } catch (err) {
      if (err instanceof HttpError) throw err;
      throw rpcUnavailable(err);
    }

    const maxCost = cfg.dripWei + gas * fees.maxFeePerGas;
    if (balance < cfg.minBalanceWei || balance < maxCost) {
      console.warn(`faucet balance low: ${formatTxds(balance)} ${SYMBOL}`);
      throw new HttpError(
        503,
        "faucet_empty",
        "The faucet is running low and is paused until it is refilled. Please try again later or ask on Discord.",
      );
    }

    const nonce = Math.max(pendingNonce, this.nextNonce ?? 0);

    // Reserve the rate-limit slots BEFORE broadcasting: if we crash after broadcast the
    // user cannot immediately claim again. Released below if the send definitely failed.
    await reserve(env.RATE_LIMIT, keys, now);

    let txHash: `0x${string}`;
    try {
      const serializedTransaction = await account.signTransaction({
        type: "eip1559",
        chainId: cfg.chainId,
        nonce,
        to,
        value: cfg.dripWei,
        gas,
        maxFeePerGas: fees.maxFeePerGas,
        maxPriorityFeePerGas: fees.maxPriorityFeePerGas,
      });
      txHash = await client.sendRawTransaction({ serializedTransaction });
    } catch (err) {
      // Unknown outcome (e.g. timeout) or rejection: force a fresh nonce read next time.
      this.nextNonce = null;
      await release(env.RATE_LIMIT, keys);
      throw rpcUnavailable(err);
    }

    this.nextNonce = nonce + 1;
    return {
      ok: true,
      txHash,
      to,
      amount: formatTxds(cfg.dripWei),
      amountWei: cfg.dripWei.toString(),
      symbol: SYMBOL,
      nonce,
    };
  }
}

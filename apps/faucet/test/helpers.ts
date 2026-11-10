// Test doubles: in-memory KV, fake Durable Object namespace, fetch mock for Turnstile + JSON-RPC.
// No network access: any unexpected outbound fetch throws.

import { keccak256 } from "viem";
import { FaucetSigner } from "../src/signer.ts";
import { SITEVERIFY_URL } from "../src/turnstile.ts";
import type { Env } from "../src/env.ts";

/** Well-known Anvil/Hardhat dev key #0 (public, test-only). */
export const TEST_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
export const TEST_FAUCET_ADDRESS = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
export const RPC_URL = "https://rpc.testnet.example/";
export const CHAIN_ID = 717171;
export const TURNSTILE_SECRET = "turnstile-secret-test";

export class MockKV {
  store = new Map<string, string>();
  puts: { key: string; value: string; ttl?: number }[] = [];
  async get(key: string): Promise<string | null> {
    return this.store.get(key) ?? null;
  }
  async put(key: string, value: string, opts?: { expirationTtl?: number }): Promise<void> {
    this.store.set(key, value);
    this.puts.push({ key, value, ttl: opts?.expirationTtl });
  }
  async delete(key: string): Promise<void> {
    this.store.delete(key);
  }
}

export interface RpcState {
  chainId: number;
  balance: bigint;
  pendingNonce: number;
  baseFee: bigint;
  priorityFee: bigint;
  gas: bigint;
  failSend: boolean;
  failAll: boolean;
  sent: `0x${string}`[];
  calls: string[];
}

export interface TurnstileState {
  success: boolean;
  action?: string;
  hostname?: string;
  calls: { secret: string; response: string; remoteip: string }[];
}

export function defaultRpc(): RpcState {
  return {
    chainId: CHAIN_ID,
    balance: 1_000_000n * 10n ** 18n,
    pendingNonce: 7,
    baseFee: 1_000_000_000n,
    priorityFee: 1_000_000_000n,
    gas: 21_000n,
    failSend: false,
    failAll: false,
    sent: [],
    calls: [],
  };
}

const hex = (n: bigint | number) => `0x${n.toString(16)}`;

export function installFetchMock(rpc: RpcState, turnstile: TurnstileState): () => void {
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input as RequestInfo, init as RequestInit);
    if (req.url === SITEVERIFY_URL) {
      const form = await req.formData();
      turnstile.calls.push({
        secret: String(form.get("secret")),
        response: String(form.get("response")),
        remoteip: String(form.get("remoteip")),
      });
      return Response.json({
        success: turnstile.success,
        "error-codes": turnstile.success ? [] : ["invalid-input-response"],
        hostname: turnstile.hostname ?? "faucet.testnet.example",
        action: turnstile.action ?? "claim",
      });
    }
    if (req.url === RPC_URL) {
      const body = (await req.json()) as unknown;
      const handle = (msg: { id: number; method: string; params: unknown[] }) => {
        rpc.calls.push(msg.method);
        const ok = (result: unknown) => ({ jsonrpc: "2.0", id: msg.id, result });
        const err = (message: string) => ({ jsonrpc: "2.0", id: msg.id, error: { code: -32000, message } });
        if (rpc.failAll) return err("node down");
        switch (msg.method) {
          case "eth_chainId":
            return ok(hex(rpc.chainId));
          case "eth_getBalance":
            return ok(hex(rpc.balance));
          case "eth_getTransactionCount":
            return ok(hex(rpc.pendingNonce));
          case "eth_getBlockByNumber":
            return ok({
              number: "0x10",
              hash: `0x${"ab".repeat(32)}`,
              parentHash: `0x${"cd".repeat(32)}`,
              timestamp: "0x6553f100",
              gasLimit: "0x1c9c380",
              gasUsed: "0x0",
              baseFeePerGas: hex(rpc.baseFee),
              miner: `0x${"00".repeat(20)}`,
              transactions: [],
            });
          case "eth_maxPriorityFeePerGas":
            return ok(hex(rpc.priorityFee));
          case "eth_gasPrice":
            return ok(hex(rpc.baseFee + rpc.priorityFee));
          case "eth_estimateGas":
            return ok(hex(rpc.gas));
          case "eth_sendRawTransaction": {
            if (rpc.failSend) return err("txpool is full");
            const raw = msg.params[0] as `0x${string}`;
            rpc.sent.push(raw);
            return ok(keccak256(raw));
          }
          default:
            return err(`unexpected method ${msg.method}`);
        }
      };
      const result = Array.isArray(body) ? body.map(handle) : handle(body as never);
      return Response.json(result);
    }
    throw new Error(`unexpected network access in test: ${req.url}`);
  }) as typeof fetch;
  return () => {
    globalThis.fetch = original;
  };
}

export function makeEnv(overrides: Partial<Env> = {}): { env: Env; kv: MockKV } {
  const kv = new MockKV();
  const env = {
    RATE_LIMIT: kv as unknown as KVNamespace,
    FAUCET_PRIVATE_KEY: TEST_PRIVATE_KEY,
    TURNSTILE_SECRET_KEY: TURNSTILE_SECRET,
    TURNSTILE_SITE_KEY: "1x00000000000000000000AA",
    RPC_URL,
    CHAIN_ID: String(CHAIN_ID),
    DRIP_AMOUNT: "100",
    MIN_FAUCET_BALANCE: "1000",
    CORS_ORIGINS: "https://explorer.testnet.example, https://wallet.testnet.example",
    EXPLORER_TX_URL: "https://explorer.testnet.example/tx/{hash}",
    ...overrides,
  } as Env;
  // One signer instance per env, like the single named DO in production.
  const signer = new FaucetSigner({} as DurableObjectState, env);
  env.FAUCET_SIGNER = {
    idFromName: () => ({}) as DurableObjectId,
    get: () => signer,
  };
  return { env, kv };
}

export function claimRequest(body: unknown, ip = "203.0.113.7", headers: Record<string, string> = {}): Request {
  return new Request("https://faucet.testnet.example/api/claim", {
    method: "POST",
    headers: { "Content-Type": "application/json", "CF-Connecting-IP": ip, ...headers },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
}

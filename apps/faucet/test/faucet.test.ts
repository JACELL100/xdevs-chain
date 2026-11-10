import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, it } from "node:test";
import { parseTransaction, recoverTransactionAddress, type TransactionSerializedEIP1559 } from "viem";
import worker from "../src/index.ts";
import { ipKey } from "../src/ratelimit.ts";
import {
  CHAIN_ID,
  claimRequest,
  defaultRpc,
  installFetchMock,
  makeEnv,
  TEST_FAUCET_ADDRESS,
  TEST_PRIVATE_KEY,
  TURNSTILE_SECRET,
  type RpcState,
  type TurnstileState,
} from "./helpers.ts";

const ALICE = "0x1111111111111111111111111111111111111111";
const BOB = "0x2222222222222222222222222222222222222222";
const DRIP_WEI = 100n * 10n ** 18n;

let rpc: RpcState;
let turnstile: TurnstileState;
let restoreFetch: () => void;
let logs: string[];
const consoleBackup = { log: console.log, info: console.info, warn: console.warn, error: console.error };

beforeEach(() => {
  rpc = defaultRpc();
  turnstile = { success: true, calls: [] };
  restoreFetch = installFetchMock(rpc, turnstile);
  logs = [];
  for (const k of ["log", "info", "warn", "error"] as const) {
    console[k] = (...args: unknown[]) => {
      logs.push(args.map(String).join(" "));
    };
  }
});

afterEach(() => {
  restoreFetch();
  Object.assign(console, consoleBackup);
  // The private key must never appear in logs, in any test.
  const keyHex = TEST_PRIVATE_KEY.slice(2);
  for (const line of logs) assert.ok(!line.toLowerCase().includes(keyHex), `private key leaked to logs: ${line}`);
});

async function call(env: Parameters<typeof worker.fetch>[1], req: Request) {
  const res = await worker.fetch(req, env);
  const text = await res.text();
  assert.ok(!text.includes(TEST_PRIVATE_KEY.slice(2)), "private key leaked in response");
  let body: any = text;
  try {
    body = JSON.parse(text);
  } catch {
    /* html */
  }
  return { res, body };
}

describe("GET /", () => {
  it("serves the accessible form with Turnstile and the no-value notice", async () => {
    const { env } = makeEnv();
    const { res, body } = await call(env, new Request("https://faucet.testnet.example/"));
    assert.equal(res.status, 200);
    assert.match(res.headers.get("Content-Type") ?? "", /text\/html/);
    assert.match(body, /Testnet tokens have no value/);
    assert.match(body, /class="cf-turnstile" data-sitekey="1x00000000000000000000AA"/);
    assert.match(body, /<label for="address">/);
    assert.match(body, /aria-live="polite"/);
    assert.match(res.headers.get("Content-Security-Policy") ?? "", /challenges\.cloudflare\.com/);
  });
});

describe("POST /api/claim", () => {
  it("happy path: verifies captcha, sends an EIP-1559 tx of DRIP_AMOUNT, records both limits", async () => {
    const { env, kv } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 200, JSON.stringify(body));
    assert.equal(body.ok, true);
    assert.equal(body.amount, "100");
    assert.equal(body.symbol, "tXDS");
    assert.match(body.txHash, /^0x[0-9a-f]{64}$/);
    assert.equal(body.explorerUrl, `https://explorer.testnet.example/tx/${body.txHash}`);

    // Turnstile verified server-side with the secret and client IP.
    assert.deepEqual(turnstile.calls, [{ secret: TURNSTILE_SECRET, response: "tok", remoteip: "203.0.113.7" }]);

    // Exactly one signed tx, correct contents and signer.
    assert.equal(rpc.sent.length, 1);
    const raw = rpc.sent[0] as TransactionSerializedEIP1559;
    const tx = parseTransaction(raw);
    assert.equal(tx.type, "eip1559");
    assert.equal(tx.chainId, CHAIN_ID);
    assert.equal(tx.to, ALICE);
    assert.equal(tx.value, DRIP_WEI);
    assert.equal(tx.nonce, 7);
    assert.equal(tx.gas, 21_000n);
    assert.ok(tx.maxFeePerGas! >= rpc.baseFee + rpc.priorityFee);
    const from = await recoverTransactionAddress({ serializedTransaction: raw });
    assert.equal(from.toLowerCase(), TEST_FAUCET_ADDRESS);

    // Both rate-limit markers written with a 24h TTL.
    assert.ok(kv.store.has(`addr:${ALICE}`));
    assert.ok(kv.store.has("ip:203.0.113.7"));
    assert.ok(kv.puts.every((p) => p.ttl === 86_400));
  });

  it("accepts addresses regardless of case/checksum and normalizes them", async () => {
    const { env, kv } = makeEnv();
    const mixed = "0xAbCdEf0123456789aBcDeF0123456789AbCdEf01"; // not a valid EIP-55 checksum
    const { res } = await call(env, claimRequest({ address: `  ${mixed}  `, turnstileToken: "tok" }));
    assert.equal(res.status, 200);
    assert.ok(kv.store.has(`addr:${mixed.toLowerCase()}`));
  });

  for (const bad of ["", "0x123", "1111111111111111111111111111111111111111", "0xZZ11111111111111111111111111111111111111", 42, null]) {
    it(`rejects invalid address ${JSON.stringify(bad)} with 400 before captcha/RPC`, async () => {
      const { env } = makeEnv();
      const { res, body } = await call(env, claimRequest({ address: bad, turnstileToken: "tok" }));
      assert.equal(res.status, 400);
      assert.equal(body.ok, false);
      assert.equal(turnstile.calls.length, 0);
      assert.equal(rpc.calls.length, 0);
    });
  }

  for (const sys of [
    "0x0000000000000000000000000000000000000000",
    "0x0000000000000000000000000000000000001000",
    "0x0000000000000000000000000000000000001004",
    "0x0000000000000000000000000000000000001009",
    "0x0000000000000000000000000000000000000001",
    "0xfffffffffffffffffffffffffffffffffffffffe",
    "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE",
  ]) {
    it(`rejects system/reserved address ${sys} with 400`, async () => {
      const { env } = makeEnv();
      const { res, body } = await call(env, claimRequest({ address: sys, turnstileToken: "tok" }));
      assert.equal(res.status, 400);
      assert.equal(body.error, "reserved_address");
      assert.equal(rpc.sent.length, 0);
    });
  }

  it("rejects the faucet's own address", async () => {
    const { env } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: TEST_FAUCET_ADDRESS, turnstileToken: "tok" }));
    assert.equal(res.status, 400);
    assert.equal(body.error, "reserved_address");
    assert.equal(rpc.sent.length, 0);
  });

  it("rejects .xdevs names with a clear 400 until XNS exists", async () => {
    const { env } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: "alice.xdevs", turnstileToken: "tok" }));
    assert.equal(res.status, 400);
    assert.equal(body.error, "names_not_supported");
  });

  it("captcha failure -> 403, nothing sent, no limit consumed", async () => {
    turnstile.success = false;
    const { env, kv } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: ALICE, turnstileToken: "bad" }));
    assert.equal(res.status, 403);
    assert.equal(body.error, "captcha_failed");
    assert.equal(rpc.sent.length, 0);
    assert.equal(kv.store.size, 0);
  });

  it("missing captcha token -> 403 without calling siteverify", async () => {
    const { env } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: ALICE }));
    assert.equal(res.status, 403);
    assert.equal(body.error, "captcha_required");
    assert.equal(turnstile.calls.length, 0);
  });

  it("captcha token minted for a different action -> 403", async () => {
    turnstile.action = "login";
    const { env } = makeEnv();
    const { res } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 403);
  });

  it("per-address limit: same address from another IP within 24h -> 429 with Retry-After", async () => {
    const { env } = makeEnv();
    const first = await call(env, claimRequest({ address: ALICE, turnstileToken: "t1" }, "198.51.100.1"));
    assert.equal(first.res.status, 200);
    const second = await call(env, claimRequest({ address: ALICE.toUpperCase().replace("0X", "0x"), turnstileToken: "t2" }, "198.51.100.2"));
    assert.equal(second.res.status, 429);
    assert.equal(second.body.error, "rate_limited");
    assert.match(second.body.message, /address/);
    const retry = Number(second.res.headers.get("Retry-After"));
    assert.ok(retry > 86_000 && retry <= 86_400, `Retry-After=${retry}`);
    assert.equal(rpc.sent.length, 1);
  });

  it("per-IP limit: different address from the same IP within 24h -> 429", async () => {
    const { env } = makeEnv();
    assert.equal((await call(env, claimRequest({ address: ALICE, turnstileToken: "t1" }, "198.51.100.9"))).res.status, 200);
    const second = await call(env, claimRequest({ address: BOB, turnstileToken: "t2" }, "198.51.100.9"));
    assert.equal(second.res.status, 429);
    assert.match(second.body.message, /IP address/);
    assert.ok(second.res.headers.get("Retry-After"));
    assert.equal(rpc.sent.length, 1);
  });

  it("per-IP limit applies to the whole IPv6 /64", async () => {
    const { env } = makeEnv();
    assert.equal((await call(env, claimRequest({ address: ALICE, turnstileToken: "t1" }, "2001:db8:1:2::1"))).res.status, 200);
    const second = await call(env, claimRequest({ address: BOB, turnstileToken: "t2" }, "2001:0db8:0001:0002:ffff:eeee:dddd:cccc"));
    assert.equal(second.res.status, 429);
  });

  it("limit expires after 24h", async () => {
    const { env, kv } = makeEnv();
    kv.store.set(`addr:${ALICE}`, JSON.stringify({ at: Date.now() - 86_400_000 - 1000 }));
    const { res } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 200);
  });

  it("low faucet balance -> 503 friendly error, nothing sent, no limit consumed", async () => {
    rpc.balance = 999n * 10n ** 18n; // below MIN_FAUCET_BALANCE=1000
    const { env, kv } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 503);
    assert.equal(body.error, "faucet_empty");
    assert.match(body.message, /refilled/);
    assert.equal(rpc.sent.length, 0);
    assert.equal(kv.store.size, 0);
  });

  it("RPC send failure -> 503 and the rate-limit reservation is released", async () => {
    rpc.failSend = true;
    const { env, kv } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 503);
    assert.equal(body.error, "rpc_unavailable");
    assert.equal(kv.store.size, 0);
  });

  it("RPC chain id mismatch -> 503 misconfigured, nothing signed", async () => {
    rpc.chainId = 1;
    const { env } = makeEnv();
    const { res, body } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 503);
    assert.equal(body.error, "misconfigured");
    assert.equal(rpc.sent.length, 0);
  });

  it("concurrent claims are serialized with distinct sequential nonces", async () => {
    const { env } = makeEnv();
    // The mock node's "pending" nonce never advances, simulating a lagging pending view.
    const addrs = Array.from({ length: 5 }, (_, i) => `0x${String(i + 3).repeat(40)}`);
    const results = await Promise.all(
      addrs.map((a, i) => call(env, claimRequest({ address: a, turnstileToken: `t${i}` }, `192.0.2.${i + 1}`))),
    );
    for (const r of results) assert.equal(r.res.status, 200, JSON.stringify(r.body));
    const nonces = rpc.sent.map((raw) => parseTransaction(raw as TransactionSerializedEIP1559).nonce).sort((a, b) => a - b);
    assert.deepEqual(nonces, [7, 8, 9, 10, 11]);
  });

  it("invalid JSON -> 400", async () => {
    const { env } = makeEnv();
    const { res, body } = await call(env, claimRequest("{not json"));
    assert.equal(res.status, 400);
    assert.equal(body.error, "invalid_json");
  });

  it("malformed private key -> 503 without leaking it", async () => {
    const { env } = makeEnv({ FAUCET_PRIVATE_KEY: TEST_PRIVATE_KEY.slice(0, 40) });
    const { res, body } = await call(env, claimRequest({ address: ALICE, turnstileToken: "tok" }));
    assert.equal(res.status, 503);
    assert.equal(body.error, "misconfigured");
    for (const line of logs) assert.ok(!line.includes(TEST_PRIVATE_KEY.slice(2, 40)));
  });

  it("GET on /api/claim -> 405", async () => {
    const { env } = makeEnv();
    const { res } = await call(env, new Request("https://faucet.testnet.example/api/claim"));
    assert.equal(res.status, 405);
  });
});

describe("GET /api/status", () => {
  it("returns faucet address, balance, drip amount and chain id — no secrets", async () => {
    const { env } = makeEnv();
    const { res, body } = await call(env, new Request("https://faucet.testnet.example/api/status"));
    assert.equal(res.status, 200);
    assert.equal(body.address.toLowerCase(), TEST_FAUCET_ADDRESS);
    assert.equal(body.balance, "1000000");
    assert.equal(body.dripAmount, "100");
    assert.equal(body.chainId, CHAIN_ID);
    assert.equal(body.healthy, true);
    const text = JSON.stringify(body);
    assert.ok(!text.includes(TURNSTILE_SECRET));
  });

  it("reports 503 when the RPC is down", async () => {
    rpc.failAll = true;
    const { env } = makeEnv();
    const { res, body } = await call(env, new Request("https://faucet.testnet.example/api/status"));
    assert.equal(res.status, 503);
    assert.equal(body.balance, null);
  });
});

describe("CORS", () => {
  it("echoes an allowed origin and answers preflight", async () => {
    const { env } = makeEnv();
    const pre = await worker.fetch(
      new Request("https://faucet.testnet.example/api/claim", {
        method: "OPTIONS",
        headers: { Origin: "https://wallet.testnet.example", "Access-Control-Request-Method": "POST" },
      }),
      env,
    );
    assert.equal(pre.status, 204);
    assert.equal(pre.headers.get("Access-Control-Allow-Origin"), "https://wallet.testnet.example");
    assert.match(pre.headers.get("Access-Control-Allow-Methods") ?? "", /POST/);

    const { res } = await call(
      env,
      claimRequest({ address: ALICE, turnstileToken: "tok" }, "203.0.113.50", { Origin: "https://explorer.testnet.example" }),
    );
    assert.equal(res.status, 200);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), "https://explorer.testnet.example");
  });

  it("does not allow unknown origins", async () => {
    const { env } = makeEnv();
    const res = await worker.fetch(
      new Request("https://faucet.testnet.example/api/status", { headers: { Origin: "https://evil.example" } }),
      env,
    );
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
  });
});

describe("ipKey", () => {
  it("normalizes IPv4 and truncates IPv6 to /64", () => {
    assert.equal(ipKey("203.0.113.7"), "ip:203.0.113.7");
    assert.equal(ipKey("2001:db8:1:2::1"), "ip6:2001:db8:1:2::/64");
    assert.equal(ipKey("2001:0DB8:0001:0002:aaaa:bbbb:cccc:dddd"), "ip6:2001:db8:1:2::/64");
    assert.equal(ipKey("::1"), "ip6:0:0:0:0::/64");
  });
});

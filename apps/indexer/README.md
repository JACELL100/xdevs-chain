# xDEVS indexer, explorer API and vault alerts

`apps/indexer` follows an xDEVS Chain node over standard JSON-RPC, stores blocks, transactions, logs, transfers,
approvals, `.xdevs` names, Guardian Vault queues and account-security events in Postgres, serves the read-only
[explorer API](../../docs/specs/explorer-api.md), and sends Guardian Vault alerts (plan §16.2) to webhooks and Telegram.

Node.js 24, TypeScript run directly by Node (type stripping, no build step), three runtime dependencies: `viem`, `pg`
and `@xdevs/sdk` (linked from `sdk/ts`).

## What it does

- **Follows the chain**: `eth_blockNumber` polling plus `eth_subscribe("newHeads")` over WebSocket when the node offers
  it (polling stays the source of truth), `eth_getBlockByNumber(n, true)`, `eth_getBlockReceipts` (falls back to
  per-tx receipts) and an `eth_getLogs` cross-check, in JSON-RPC batches of at most 50 (the node's limit, plan §19.3).
- **Verifies** parent hashes inside each batch and against the stored cursor. BFT finality means no reorgs, but a
  restarted dev chain does diverge: the indexer rolls back to the common ancestor (at most `MAX_REORG_DEPTH` blocks)
  and rebuilds the derived tables from stored logs; deeper divergence stops it with a clear error.
- **Idempotent and resumable**: each batch (rows, derived state, alert outbox, cursor) is one database transaction.
- **Fees**: per transaction and block, base fee split burned / Treasury (80/20 on an xDEVS node) and the tip.
- **Spam and address poisoning** (plan §16.3): zero-value transfers, dust, look-alike counterparties (same first 4 and
  last 4 hex chars as an address the other side already dealt with), unknown and impersonating tokens. Flags are
  stored, nothing is deleted; the API hides spam not sent by the viewed address unless `includeSpam=true`.
- **Approvals**: ERC-20 (never expire), XDRC-20 `ApprovalWithExpiry` / `AllApprovalsRevoked`, Permit2Expiring
  `Approval` / `Permit` / `Lockdown` / `AllRevoked`; the API adds the live remaining allowance and a ready-to-sign
  revoke call.
- **XNS**: registrations, renewals, transfers, address records and verified primary names.
- **Guardian Vault and account security**: queued / cancelled / executed / invalidated actions, freeze, recovery,
  guardians, XIP-0001 EOA lock; live `xdevs_getEoaLock` and `xdevs_getCommitCertificate` from the node when available.
- **Alerts**: `vault.queued`, `recovery.approved`, `recovery.ready`, `recovery.executed`, `account.frozen`,
  `account.unfrozen`, `eoaLock.changed`, delivered at least once per destination with retries.

## Run locally

```bash
cd apps/indexer
npm ci

# 1) A node. Either the xDEVS dev node with every predeploy (XNS, Permit2Expiring, WXDS, smart accounts) ...
xdevs run --dev --genesis ../../deploy/configs/dev-genesis-with-predeploys.json --data-dir ./.devnode --block-time 1s
# ... or anvil (chain id 7117, no xDEVS predeploys)
anvil --chain-id 7117

# 2) Postgres. Any local Postgres, or no server at all with in-process PGlite (needs the dev dependencies):
DATABASE_URL=pglite://./data RPC_URL=http://127.0.0.1:8545 npm start
#    or
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/xdevs_explorer npm start

curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/api/v1/stats
```

`npm start` applies `migrations/*.sql` (once each, recorded in `schema_migrations`), then runs the indexer and the API
in one process. `MODE=indexer` / `MODE=api` split them (for example several API processes in front of one indexer).

Operator commands (they write to the database; the HTTP API never does):

```bash
npm run cli -- subscribe 0xYourAccount 123456789   # Telegram chat id for alerts about that account
npm run cli -- unsubscribe 0xYourAccount 123456789
npm run cli -- label 0xAddress "Faucet"
npm run cli -- status                               # cursor, totals, pending/failed alerts
npm run cli -- reset --yes                          # drop everything (e.g. after resetting a dev chain)
```

## Configuration (environment only)

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | `postgres://postgres:postgres@127.0.0.1:5432/xdevs_explorer` | Postgres URL (`?sslmode=require` for Neon/Supabase), or `pglite://<dir>` / `pglite://` (memory). **Secret.** |
| `RPC_URL` | `http://127.0.0.1:8545` | Node JSON-RPC (HTTP). |
| `WS_URL` | `RPC_URL` with `ws://` | WebSocket for `newHeads`; `off` to poll only. |
| `MODE` | `all` | `all`, `indexer` or `api`. |
| `PORT`, `HOST` | `8080`, `0.0.0.0` | API listen address. |
| `START_BLOCK` | `0` | First block to index on an empty database. |
| `BATCH_SIZE` | `20` | Blocks per batch / transaction. |
| `POLL_INTERVAL_MS` | `1000` | Head polling interval. |
| `CONFIRMATIONS` | `0` | Stay this many blocks behind the head (0 is right under BFT finality). |
| `MAX_REORG_DEPTH` | `64` | Deepest automatic rollback. |
| `VERIFY_LOGS` | `true` | `eth_getLogs` cross-check per batch. |
| `RPC_BATCH` | `true` | Use JSON-RPC batches (auto-disabled if the server rejects them). |
| `RPC_TIMEOUT_MS` | `15000` | Per request. |
| `BURN_BPS` | auto | Base-fee share burned: 8000 on an xDEVS node, 10000 elsewhere. |
| `CORS_ORIGINS` | `*` | Comma list of allowed origins. |
| `RATE_LIMIT_RPS`, `RATE_LIMIT_BURST` | `10`, `40` | Token bucket per client IP. |
| `TRUST_PROXY` | `none` | `cloudflare` (use `CF-Connecting-IP`) or `xff` (first `X-Forwarded-For`). Only behind a proxy you control. |
| `HEALTH_MAX_LAG` | `30` | `/health` reports `ok: false` beyond this lag. |
| `DUST_NATIVE_WEI` | `10^15` | Native dust threshold (0.001 tXDS). |
| `KNOWN_TOKENS` | — | Comma list of verified token addresses (WXDS is always known). |
| `CANONICAL_TOKEN_FACTORY` | — | `CanonicalTokenFactory` address; its tokens count as known. |
| `XNS_REGISTRY`, `PERMIT2_EXPIRING`, `WXDS` | predeploys | Override only for non-xDEVS test chains. |
| `ALERT_WEBHOOK_URLS` | — | Comma list; each gets a JSON `POST`. **Secret** (URLs often embed tokens; logs show only the index). |
| `ALERT_WEBHOOK_SECRET` | — | HMAC-SHA256 key for the `X-Xdevs-Signature: sha256=…` header. **Secret.** |
| `TELEGRAM_BOT_TOKEN` | — | Bot token for `sendMessage`. **Secret.** |
| `ALERT_EVENTS` | all | Comma list of alert types to send. |
| `ALERT_MAX_AGE_SECONDS` | `3600` | Blocks older than this never alert (no alert storm during a backfill). |
| `ALERT_UNVERIFIED` | `false` | Also alert for emitters whose code is not an xDEVS smart account. |
| `EXPLORER_URL` | — | Adds an explorer link to alerts. |

## Tests

```bash
npm test             # everything below
npm run test:unit    # decoder, spam rules, indexer, API, alerts: PGlite (in-process Postgres) + a mocked RPC
npm run test:integration
npm run typecheck
```

The integration test starts a real node, sends real transactions with viem (deploys the XDRC-20 from
`test/fixtures/xdrc20.json`, transfers, an expiring approval, a native transfer, and on the xDEVS node an XNS
registration with a primary name and a WXDS deposit), indexes them into PGlite over HTTP and checks the API. It uses
`XDEVS_BIN` (default `~/.xdevs-target/debug/xdevs[.exe]`) with `deploy/configs/dev-genesis-with-predeploys.json`, or
anvil (`ANVIL_BIN`, `~/.foundry/bin`, `PATH`; force with `INTEGRATION_NODE=anvil`), and is skipped when neither exists.
`test/fixtures/xdrc20.json` comes from `contracts/out` (`forge build`); regenerate it if XDRC20 changes, and
`src/abi.generated.ts` with `node scripts/gen-abi.mjs`.

## Deploying for free (plan §24.1)

The indexer is a **long-running process**: it has to poll the node every second and keep the alert outbox moving. It
does not fit serverless free tiers (Cloudflare Workers, Vercel functions) and free "web service" hosts that sleep
when idle would stop indexing and alerting. Honest options:

1. **On the machine that runs the public RPC node** (the home PC behind the Cloudflare Tunnel, plan §24.2). The
   indexer then reads the node on `127.0.0.1` with no public RPC limits, and the API is published through the same
   tunnel (for example `xdevschain-explorer-api.xdevs.in` → `127.0.0.1:8080`). Uptime is the home machine's uptime.
2. **An Oracle Cloud Always Free VM** (the Ampere A1 shape or an AMD micro; see
   [docs/runbooks/oracle-always-free.md](../../docs/runbooks/oracle-always-free.md)). Point `RPC_URL` at a validator or
   full node over the private network. Keep `MODE=all` on a 1 GB micro VM; Node uses about 100–150 MB.

Either way, run it under systemd (sketch):

```ini
[Unit]
Description=xDEVS explorer indexer + API
After=network-online.target

[Service]
WorkingDirectory=/opt/xdevs/apps/indexer
EnvironmentFile=/etc/xdevs/indexer.env
ExecStart=/usr/bin/node --disable-warning=ExperimentalWarning src/main.ts
Restart=always
RestartSec=5
User=xdevs
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

**Database**: a free Neon or Supabase Postgres (about 0.5 GB). Use the connection string with `sslmode=require`, and on
Neon prefer the direct (non-pooled) endpoint for the indexer: it holds one transaction per batch. Supabase pauses
inactive free projects; the indexer writes every block, so an active testnet keeps it awake, but check the current
terms. Size estimate: roughly 1–2 KB per transaction plus logs, so 0.5 GB holds a few hundred thousand transactions;
set `START_BLOCK` to skip history if space runs out.

**API exposure**: put Cloudflare in front (tunnel or proxied DNS), set `TRUST_PROXY=cloudflare`, set `CORS_ORIGINS` to the
explorer origin, and add a Cloudflare rate-limiting rule as a second layer. `/health` is the UptimeRobot target.

## Limitations

See [explorer-api.md §6](../../docs/specs/explorer-api.md). In short: no internal (trace-based) native transfers, no
NFTs, primary names only through `ReverseClaimed` + `NameChanged`, alerts already sent are never retracted, and the
look-alike check only knows the history this indexer has seen (start from block 0 for full coverage).

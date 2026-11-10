# Explorer API (`apps/indexer`) — v1

Status: testnet reference implementation. Plan refs: §16.2 (vault alerts), §16.3 (address poisoning), §16.5
(expiring approvals), §19 (JSON-RPC), §23 (explorer), §24.1 (free hosting).

The indexer (`apps/indexer`) follows an xDEVS node over standard `eth_` JSON-RPC, writes Postgres, and serves this
read-only HTTP API. The explorer UI (`apps/explorer`) uses only this API plus the public RPC. Everything here is
**advisory** display data: the chain is the source of truth.

## 1. Conventions

| Topic | Rule |
|---|---|
| Base path | `/api/v1`. Also `GET /health` (no prefix) for uptime checks. |
| Methods | `GET`, `HEAD` and `OPTIONS` (CORS preflight). Anything else: `405`. The API never writes chain data. |
| JSON | `application/json; charset=utf-8`, camelCase keys. |
| Addresses | Inputs are case-insensitive. Outputs are EIP-55 checksummed. |
| Hashes, calldata | Lowercase `0x` hex. |
| Big numbers | `0x`-prefixed lowercase hex strings ("quantities"): values, balances, gas, fees, token amounts, vault action ids. |
| Small numbers | JSON numbers: block numbers, timestamps (Unix seconds), indexes, counts, decimals, nonces. |
| Absent | `null` (never omitted for documented keys). |
| Pagination | `?limit=` 1..100 (default 25) and `?cursor=` (opaque string from the previous page). Response `{ "items": [...], "nextCursor": string \| null }`. Newest first. A cursor is stable: rows indexed after the first page never shift later pages. |
| Booleans in query | `true` / `false` / `1` / `0`. |
| Errors | `{ "error": { "code": string, "message": string } }`. Codes: `BAD_REQUEST` (400), `NOT_FOUND` (404), `METHOD_NOT_ALLOWED` (405), `RATE_LIMITED` (429), `INTERNAL` (500), `UNAVAILABLE` (503, database down). |
| CORS | `Access-Control-Allow-Origin` from `CORS_ORIGINS` (default `*`), methods `GET, HEAD, OPTIONS`. |
| Rate limit | Token bucket per client IP (`RATE_LIMIT_RPS`, `RATE_LIMIT_BURST`). Headers `RateLimit-Limit`, `RateLimit-Remaining`; on `429` also `Retry-After` (seconds). Behind Cloudflare set `TRUST_PROXY=cloudflare` so `CF-Connecting-IP` is used. |
| Caching | `Cache-Control: public, max-age=5` for lists and summaries, `max-age=60` for blocks and transactions (final under BFT). |
| Live data | Fields marked *live* are read from the node at request time (`eth_getBalance`, `eth_call`, ...). If the node is unreachable they are `null` and the object carries `"live": false`. |

### 1.1 Shared objects

`TokenRef`
```json
{ "address": "0x…" | null, "name": "Testnet XDS", "symbol": "tXDS", "decimals": 18, "known": true }
```
`address: null` means the native coin. `known` is true for native tXDS, the WXDS predeploy, canonical bridged
tokens (`CanonicalTokenCreated` from `CANONICAL_TOKEN_FACTORY`) and the operator allow-list `KNOWN_TOKENS`.
`name`/`symbol`/`decimals` are `null` when the token's metadata calls failed.

`BlockSummary`
```json
{
  "number": 120, "hash": "0x…", "parentHash": "0x…", "timestamp": 1793650289, "miner": "0x…",
  "txCount": 3, "gasUsed": "0x…", "gasLimit": "0x…", "baseFeePerGas": "0x…" | null, "size": 1234 | null,
  "fees": { "burned": "0x…", "treasury": "0x…", "priority": "0x…" }
}
```

`TxSummary`
```json
{
  "hash": "0x…", "blockNumber": 120, "timestamp": 1793650289, "txIndex": 0, "type": 2,
  "from": "0x…", "to": "0x…" | null, "contractCreated": "0x…" | null,
  "value": "0x…", "status": "success" | "reverted", "methodId": "0xa9059cbb" | null, "fee": "0x…",
  "spam": false, "spamReasons": []
}
```

`FeeBreakdown` (xDEVS fee model, plan §15.1: base fee split between burn and Treasury, tip to the proposer)
```json
{
  "total": "0x…",            // gasUsed × effectiveGasPrice
  "baseFee": "0x…",          // gasUsed × block.baseFeePerGas
  "burned": "0x…",           // baseFee × burnBps / 10000
  "treasury": "0x…",         // baseFee − burned
  "priorityFee": "0x…",      // total − baseFee (to block.miner)
  "burnBps": 8000            // chain config burnBps (the node reads it from genesis, not FeeConfig): 8000 on an xDEVS
                             // node (web3_clientVersion contains "xdevs"), 10000 elsewhere (anvil); override with BURN_BPS
}
```

`TokenTransfer`
```json
{
  "blockNumber": 120, "timestamp": 1793650289, "txHash": "0x…", "logIndex": 4 | null,
  "kind": "native" | "erc20", "token": TokenRef,
  "from": "0x…", "to": "0x…", "value": "0x…",
  "spam": false, "spamReasons": [], "lookAlikeOf": "0x…" | null
}
```
`kind: "native"` rows come from the transaction's own `value` (top-level only: internal native transfers need
`debug_trace*`, which the node does not expose publicly) and from WXDS `Deposit` / `Withdrawal` (shown as
`erc20` mint/burn of WXDS). `logIndex` is `null` for native rows.

`DecodedLog`
```json
{
  "logIndex": 4, "address": "0x…", "topics": ["0x…"], "data": "0x…",
  "event": "ApprovalWithExpiry" | null, "signature": "ApprovalWithExpiry(address,address,uint256,uint64)" | null,
  "standard": "erc20" | "xdrc20" | "permit2" | "xns" | "vault" | "account" | "factory" | "wxds" | null,
  "args": { "owner": "0x…", "spender": "0x…", "value": "0x…", "expiry": "0x…" } | null
}
```
Integers in `args` are hex strings, `bytes` are hex, tuples are objects, arrays are arrays. Unknown events have
`event`, `signature`, `standard` and `args` set to `null`.

`ApprovalChange` (an approval written by one transaction)
```json
{
  "kind": "erc20" | "xdrc20" | "permit2", "event": "Approval" | "ApprovalWithExpiry" | "Permit" | "Lockdown" | "AllApprovalsRevoked" | "AllRevoked",
  "token": TokenRef | null, "owner": "0x…", "spender": "0x…" | null,
  "amount": "0x…" | null, "unlimited": false, "expiry": 1796242289 | null, "revokesAll": false
}
```
`unlimited` is true for `2^256−1` (ERC-20/XDRC-20) or `2^160−1` (Permit2). `expiry: null` on a plain ERC-20
approval means **it never expires**. `revokesAll` marks `AllApprovalsRevoked` (XDRC-20, per token) and
`AllRevoked` (Permit2Expiring, every token).

## 2. Endpoints

### `GET /health`
`200 { "ok": true, "chainId": 7117, "indexedBlock": 120, "headBlock": 121 | null, "lagBlocks": 1 | null }`.
`ok` is false (still `200`) when the lag exceeds `HEALTH_MAX_LAG` blocks (default 30) or the node is unreachable.

### `GET /api/v1/stats`
```json
{
  "chainId": 7117, "latestBlock": 121 | null, "indexedBlock": 120, "lagBlocks": 1 | null,
  "avgBlockTimeMs": 2000 | null,        // over the last 100 indexed blocks
  "totalTransactions": 1234, "totalAddresses": 56, "totalTokenTransfers": 789,
  "burnedFees": "0x…", "treasuryFees": "0x…", "latestBaseFee": "0x…" | null,
  "nativeCurrency": { "name": "Testnet XDS", "symbol": "tXDS", "decimals": 18 }
}
```

### `GET /api/v1/blocks?limit&cursor`
Page of `BlockSummary`, newest first.

### `GET /api/v1/blocks/:numberOrHash`
`:numberOrHash` is a decimal number, a `0x` hex number (≤ 16 hex digits), a 32-byte hash, or `latest`.
`200 { ...BlockSummary, "transactions": TxSummary[], "commit": { "round": 0, "signatures": 4, "validators": ["0x…"] } | null }`
(transactions in block order, at most 1000), `404` if not indexed. `commit` is *live*: `xdevs_getCommitCertificate(number)`
from the node, shown as "finalized by N validators"; `null` for dev-mode blocks, genesis, a hash mismatch, or a node
without the method.

### `GET /api/v1/txs?limit&cursor`
Page of `TxSummary`, newest first.

### `GET /api/v1/txs/:hash`
```json
{
  ...TxSummary,
  "blockHash": "0x…", "nonce": 7, "gas": "0x…", "gasUsed": "0x…",
  "gasPrice": "0x…" | null, "maxFeePerGas": "0x…" | null, "maxPriorityFeePerGas": "0x…" | null,
  "effectiveGasPrice": "0x…", "input": "0x…",
  "authorizationList": [{ "chainId": "0x…", "address": "0x…", "nonce": "0x…", "authority": null }] | null,
  "feeBreakdown": FeeBreakdown,
  "logs": DecodedLog[],
  "tokenTransfers": TokenTransfer[],
  "approvals": ApprovalChange[]
}
```
`404` if the transaction is not indexed (pending transactions are not served).

### `GET /api/v1/addresses/:address`
```json
{
  "address": "0x…",
  "live": true,
  "balance": "0x…" | null,                // live: eth_getBalance
  "nonce": 7 | null,                      // live: eth_getTransactionCount
  "kind": "eoa" | "contract" | "delegated" | null,   // live: eth_getCode; "delegated" = EIP-7702 designator
  "delegatedTo": "0x…" | null,
  "isContract": false,                    // live when available, else indexed
  "firstSeenBlock": 10 | null, "lastSeenBlock": 120 | null, "txCount": 12,
  "xns": { "primaryName": "alice.xdevs" | null, "names": [{ "name": "alice.xdevs", "expires": 1796242289, "status": "active" }] },
  "labels": ["Treasury (predeploy)"],
  "token": TokenRef | null,               // when the address is an indexed token contract
  "security": {
    "accountKind": "xdevs-7702" | "xdevs-proxy" | "unknown" | null,  // null: no account events seen
    "vaultEnabled": true | null,
    "frozen": false, "frozenUntil": 1796242289 | null,
    "eoaLock": { "status": "unlocked" | "locked" | "sealed" | "unknown", "active": true | null, "source": "rpc" | "indexer" },
    "eoaKeyRevoked": false,
    "recovery": { "id": "0x…", "approvals": 2, "readyAt": 1796242289 | null } | null,
    "guardians": { "count": 3, "threshold": 2 } | null,
    "openVaultActions": 1
  }
}
```
`eoaLock.source` is `rpc` when the node answered `xdevs_getEoaLock` (xDEVS node), otherwise the last indexed
`EoaLockUpdated` (`active: null`). `primaryName` is verified: the reverse record's name must currently be active and
resolve back to this address. `labels` combine predeploy names (`contracts/src/common/Predeploys.sol`) and the
`labels` table.

### `GET /api/v1/addresses/:address/txs?limit&cursor&includeSpam=false`
Page of `TxSummary` where the address is `from`, `to` or the created contract. With `includeSpam=false` (default),
transactions flagged `spam` **and not sent by this address** are hidden. Response adds `"hiddenSpam"`: on the first
page with `includeSpam=false`, the number of hidden rows (capped at 1000); `null` otherwise.

### `GET /api/v1/addresses/:address/token-transfers?limit&cursor&includeSpam=false&token=0x…`
Page of `TokenTransfer` where the address is `from` or `to` (native and token). Same spam rule, judged against the
**transaction sender** (a poisoning `transferFrom(victim, lookalike, 0)` is sent by the attacker, so it is hidden from
the victim's view even though `from` is the victim). Response adds `"hiddenSpam"` (same rule as above, respecting `token`).

### `GET /api/v1/addresses/:address/approvals?includeExpired=false`
Live approvals granted **by** the address, newest first (not paginated, at most 500).
```json
{
  "items": [{
    "kind": "erc20" | "xdrc20" | "permit2", "token": TokenRef, "spender": "0x…",
    "amount": "0x…", "unlimited": true, "expiry": 1796242289 | null, "expired": false,
    "remaining": "0x…" | null,     // live: token.allowance() / Permit2Expiring.allowance() (spending emits no event)
    "blockNumber": 100, "txHash": "0x…",
    "revoke": { "to": "0x…", "data": "0x…", "description": "approve(spender, 0) on the token" }
  }],
  "live": true
}
```
Live set (xns-and-approvals.md §3.2, §4): latest write per `(kind, token, owner, spender)` after the owner's latest
revoke-all, with `amount > 0` and (`expiry` null or `> now`). `includeExpired=true` also returns expired (not
revoked) rows with `expired: true`. `revoke` is a ready-to-sign call (the UI shows it as a hint; the API never
signs).

### `GET /api/v1/addresses/:address/vault-queue?status=open|all&limit&cursor`
Guardian Vault actions of the account (`log.address`), newest first.
```json
{
  "account": "0x…", "id": "0x3", "target": "0x…", "value": "0x…", "data": "0x…", "executeAfter": 1796242289,
  "status": "queued" | "ready" | "cancelled" | "executed" | "expired" | "invalidated",
  "queuedBlock": 100, "queuedTx": "0x…", "cancelledBy": "0x…" | null, "finalBlock": 110 | null, "finalTx": "0x…" | null
}
```
`ready`: `executeAfter ≤ now ≤ executeAfter + 14 days`. `expired`: past the 14-day execution window.
`invalidated`: queued before a `VaultEpochBumped` / `RecoveryExecuted` (smart-accounts.md §7.3). `status=open`
(default) returns `queued` and `ready`.

### `GET /api/v1/names/:name`
`:name` is `alice` or `alice.xdevs` (normalized with `@xdevs/sdk` `normalizeXnsName`; invalid → `400`).
```json
{
  "name": "alice.xdevs", "label": "alice", "node": "0x…", "labelhash": "0x…",
  "owner": "0x…", "address": "0x…" | null, "expires": 1796242289, "graceEndsAt": 1804018289,
  "status": "active" | "grace" | "released",
  "registeredBlock": 100, "registeredTx": "0x…", "isPrimaryFor": "0x…" | null
}
```
`address` is `null` unless `status` is `active` (expired names stop resolving, xns-and-approvals.md §2.2).
`404` if never registered.

### `GET /api/v1/search?q=`
`200 { "query": "…", "results": [{ "type": "block" | "tx" | "address" | "name", "id": "…", "label": "…" | null }] }`.
Rules: decimal → block; 64-hex hash → transaction, else block hash; 40-hex → address (always returned, even if never
seen); anything else → XNS name (`.xdevs` optional) and, for a valid label prefix, up to 10 indexed names starting
with it. Empty `results` when nothing matches.

## 3. Indexing

- Follows the node with `eth_blockNumber` polling (`POLL_INTERVAL_MS`) and, when the node accepts it,
  `eth_subscribe("newHeads")` over WebSocket on the same port (`WS_URL`, default derived from `RPC_URL`) to wake up early.
  Polling stays the source of truth.
- Per batch: `eth_getBlockByNumber(n, true)` and `eth_getBlockReceipts(n)` (fallback: `eth_getTransactionReceipt` per tx)
  in JSON-RPC batches of at most 50, an `eth_getLogs` cross-check of the log count and block hashes (`VERIFY_LOGS`),
  parent-hash linkage inside the batch and against the stored cursor, then one database transaction for the rows, the
  derived state, the alert outbox and the cursor. A crash never leaves half a batch; re-running a batch is a no-op.
- Event sources are pinned where spoofing matters: XNS events only from `XNS_REGISTRY` (`0x…1004`), Permit2 events only
  from `PERMIT2_EXPIRING` (`0x…1008`), WXDS mint/burn only from `WXDS` (`0x…1009`), `AccountCreated` only from the
  factory `0x…1006`. Vault and account events describe their emitter, which is classified by code.

## 4. Spam and poisoning flags

Flags are stored, never used to delete data. `spamReasons` values:

| Reason | Rule | Sets `spam` |
|---|---|---|
| `zero-value` | value is 0 (token transfer, or a plain native transfer: no calldata) | yes |
| `look-alike` | a zero-value or dust transfer whose counterparty shares the first 4 and last 4 hex chars with a *different* address the other side previously transacted with (non-spam history only); `lookAlikeOf` names it | yes |
| `impersonating-token` | an unknown token whose symbol equals `tXDS`/`XDS` or a known token's symbol | yes |
| `dust` | `0 < value < DUST_NATIVE_WEI` (native, default 10¹⁵ = 0.001 tXDS) or `< 10^(decimals−3)` token units | only with `unknown-token` |
| `unknown-token` | token is not `known` | only with `dust` |

A transaction's `spam`/`spamReasons` are those of its native transfer. Hiding is relative to the viewer: rows sent by
the viewed address are never hidden. The look-alike check uses `isLookAlikeAddress(a, b, 4)` from `@xdevs/sdk`
(4 chars matches every 5- and 6-char look-alike too).

## 5. Vault alerts (not HTTP)

The indexer writes an outbox row per (event, destination) in the same database transaction as the block, then
delivers with retries (exponential backoff, at most 8 attempts). Events: `vault.queued` (`VaultActionQueued`),
`recovery.approved` (`RecoveryApproved`, i.e. a recovery started or gained a vote), `recovery.ready`,
`recovery.executed`, `account.frozen`, `account.unfrozen`, `eoaLock.changed`. Only accounts whose code is an xDEVS
smart account (7702 designator or ERC-1967 proxy to `0x…1005`) alert, unless `ALERT_UNVERIFIED=true`; blocks older
than `ALERT_MAX_AGE_SECONDS` (default 3600) never alert, so a backfill does not replay history.

Webhook: `POST` to each `ALERT_WEBHOOK_URLS` entry with body
`{ "type": "vault.queued", "chainId": 7117, "account": "0x…", "blockNumber": 1, "txHash": "0x…", "timestamp": 1, "verified": true, "details": {...}, "explorerUrl": "…" | null }`
and header `X-Xdevs-Signature: sha256=<hex HMAC-SHA256 of the body>` when `ALERT_WEBHOOK_SECRET` is set.
Telegram: `sendMessage` with `TELEGRAM_BOT_TOKEN` to every chat id subscribed to the account in
`alert_subscriptions` (managed with `npm run cli -- subscribe <account> <chatId>`; never exposed by the API).

## 6. Limitations

- Internal (contract-to-contract) native transfers are not indexed: the public node has no `debug_trace*` (plan §19.1).
- ERC-721/1155 transfers are not decoded (Transfer with 4 topics is kept as a raw log).
- Approval `remaining` is live; `amount` is the amount written by the last approval event.
- Primary names follow `ReverseClaimed` + `NameChanged`; a reverse record written by an external resolver is not seen.
- On a parent-hash mismatch (impossible under BFT finality, possible with a restarted dev chain) the indexer rolls
  back to the common ancestor (at most `MAX_REORG_DEPTH` blocks) and rebuilds derived tables from stored logs; deeper
  divergence stops the indexer with a "reset the database" error. Alerts already sent are not retracted.

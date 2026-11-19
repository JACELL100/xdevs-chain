# JSON-RPC reference

The xDEVS node speaks the standard Ethereum JSON-RPC API, so MetaMask, Foundry, Hardhat, viem and ethers work unchanged. It also has an `xdevs_` namespace for chain-specific features.

This page describes the node as built today (Phases 1–2), whether it runs as a dev chain, a validator or a full node. The public endpoints are [not live yet](/network/endpoints).

## Transport

- **HTTP** on `127.0.0.1:8545` by default (`--http` or `[rpc] http`).
- **WebSocket** on the same port (`ws://127.0.0.1:8545`), with [subscriptions](#subscriptions-websocket); an optional separate listener can be set with `[rpc] ws`.
- CORS: any origin by default (`[rpc] cors`).

## Supported methods

### `eth_`

| Method | Notes |
|---|---|
| `eth_chainId` | `0x1bcd` (7117) on the dev chain |
| `eth_blockNumber` | |
| `eth_syncing` | Sync status of the node |
| `eth_accounts` | Empty list: the node holds no user keys |
| `eth_gasPrice`, `eth_maxPriorityFeePerGas`, `eth_feeHistory` | EIP-1559 fee data |
| `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt` | Accept a block number, tag or hash |
| `eth_call` | Gas cap 50,000,000 (`[rpc] gas_cap`) |
| `eth_estimateGas` | Returns XIP-0001 errors for EOA-locked senders |
| `eth_sendRawTransaction` | Legacy, EIP-2930, EIP-1559 and EIP-7702 (type 4). Blob (type 3) is rejected |
| `eth_getBlockByNumber`, `eth_getBlockByHash` | |
| `eth_getBlockTransactionCountByNumber`, `eth_getBlockTransactionCountByHash` | |
| `eth_getTransactionByHash`, `eth_getTransactionReceipt`, `eth_getBlockReceipts` | |
| `eth_getLogs` | At most 10,000 blocks per query and 10,000 results |

### `net_` and `web3_`

`net_version`, `net_listening`, `net_peerCount`, `web3_clientVersion`, `web3_sha3`.

### Subscriptions (WebSocket)

`eth_subscribe` / `eth_unsubscribe`, notifications as `eth_subscription`:

| Kind | Notifications |
|---|---|
| `newHeads` | One per committed block: the header as `eth_getBlockByNumber(n, false)` returns it |
| `logs` | Every log matching an optional `{ address, topics }` filter, in block order. Always `removed: false` (BFT finality, no reorgs) |
| `newPendingTransactions` | The hash of every transaction admitted to the mempool (full objects are not supported) |
| `vaultEvents` | Guardian Vault alerts (`queued`, `cancelled`, `executed`, `invalidated`, `frozen`, `unfrozen`) of an optional `{ account }` ([details](#eth_subscribevaultevents)) |

Limits: 20 subscriptions per connection, 1,000 per server, up to 256 addresses and 256 topics per log filter. A subscriber that falls too far behind gets an error notification and the subscription is closed. Over plain HTTP, `eth_subscribe` fails with `-32601`.

```bash
# with websocat or any WS client
websocat ws://127.0.0.1:8545
{"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newHeads"]}
```

### Block tags

There is no pending block, and every committed block is final:

- `latest`, `safe`, `finalized` and `pending` all mean the latest block.
- Exception: `eth_getTransactionCount` with `pending` also counts transactions waiting in the pool.
- `earliest` means block 0.

### State at past blocks

The node keeps reverse state diffs for the last **10,000 blocks** by default, so `cast ... --block N`, `anvil --fork-url` and `forge script` can pin a block while new blocks keep coming. The window is set by `[chain] pruning` (`full` = 10,000, `minimal` = 128, `archive` = all) or `state_history_blocks`. Older blocks return `-32000 historical state not available`.

## Limits

| Limit | Default | Config key |
|---|---|---|
| `eth_getLogs` block range | 10,000 blocks | `[rpc] max_logs_range` |
| `eth_getLogs` results | 10,000 | |
| `eth_call` / `eth_estimateGas` gas cap | 50,000,000 | `[rpc] gas_cap` |
| Batch size | 50 requests | `[rpc] max_batch` |
| Safety methods (`xdevs_checkRecipient`, `xdevs_getApprovals`, `xdevs_getVaultQueue`) | 300,000 gas per view call, log scans within the `eth_getLogs` limits, 256 token/spender pairs, 256 vault action ids, 20 s per request | |

Requests are limited to 5 MiB and responses to 32 MiB.

## Error codes

| Code | Meaning |
|---|---|
| `3` | Execution reverted (`eth_call` / `eth_estimateGas`); `data` holds the revert bytes |
| `-32000` | Unknown block, transaction rejected by the pool, or `historical state not available` |
| `-32003` | Transaction rejected by a validity rule. For XIP-0001, `data.reason` is `XIP0001_SEALED`, `XIP0001_NOT_SELF_CALL`, `XIP0001_VALUE_NOT_ZERO`, `XIP0001_GAS_LIMIT` or `XIP0001_FEE_CAP` |
| `-32005` | Limit exceeded (for example too many `eth_getLogs` results, or too many subscriptions on the server) |
| `-32006` | Too many subscriptions on one connection |
| `-32601` | Method not found or not supported |
| `-32602` | Invalid parameters |
| `-32603` | Internal error |

## The `xdevs_` namespace

| Method | What it does | Status |
|---|---|---|
| `xdevs_getEoaLock(address, block?)` | [XIP-0001](/concepts/eoa-lock) lock state of an address | <span class="status ok">Implemented</span> |
| `xdevs_getCommitCertificate(block)` | The validator signatures that finalized a block | <span class="status ok">Implemented</span> |
| `xdevs_submitEvidence(raw)` | Submit double-sign evidence (two conflicting signed votes) for slashing | <span class="status ok">Implemented</span> |
| `xdevs_validators(block?)` | Active validator set, power, uptime | <span class="status planned">Phase 3</span> |
| `xdevs_epochInfo(block?)` | Current epoch and next transition | <span class="status planned">Phase 3</span> |
| `xdevs_simulateTransaction(tx, block?)` | Asset changes, approvals, Guardian Vault outcome and plain-language warnings before signing | <span class="status ok">Implemented</span> |
| `xdevs_checkRecipient(to)` / `(from, to, block?)` | Facts (contract, smart account, EOA lock, XNS name) and advice (first-time, look-alike, poisoning) about a recipient | <span class="status ok">Implemented</span> |
| `xdevs_getApprovals(owner, options?, block?)` | Live approvals found in the recent log window, for a revoke screen | <span class="status ok">Implemented</span> |
| `xdevs_getVaultQueue(account, block?)` | Pending Guardian Vault actions | <span class="status ok">Implemented</span> |
| `xdevs_resolveName(name, block?)` / `xdevs_reverseName(address, block?)` | XNS lookups | <span class="status ok">Implemented</span> |
| `eth_subscribe("vaultEvents", {account?})` | Real-time vault alerts (WebSocket) | <span class="status ok">Implemented</span> |
| `xdevs_getMempoolEncryptionKey()` | Committee key of the encrypted mempool (prototype, off unless the genesis sets `encryptedMempoolBlock`) | <span class="status ok">Prototype</span> |
| `xdevs_sendEncryptedTransaction(raw)` | Submit an envelope whose payload is encrypted to the committee | <span class="status ok">Prototype</span> |
| `xdevs_localFee(contract, block?)` | Local fee-market price for a contract | <span class="status planned">Phase 5</span> |

The plan originally named this method `xdevs_getCommit`; the node and the SDK's `getCommit` use `xdevs_getCommitCertificate`.

Planned methods currently answer "method not found" (`-32601`); the [SDK](/developers/sdk) turns that into `XdevsMethodNotAvailableError` with the phase number. The plan's `xdevs_subscribe("vaultEvents")` is served as `eth_subscribe("vaultEvents")`, so standard WebSocket clients work unchanged.

### `xdevs_getEoaLock`

```bash
curl -s http://127.0.0.1:8545 -H 'content-type: application/json' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "xdevs_getEoaLock",
  "params": ["0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "latest"]
}'
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "active": true,
    "status": "unlocked",
    "word": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "delegatedTo": null
  }
}
```

| Field | Meaning |
|---|---|
| `active` | Whether XIP-0001 is enforced for the next block |
| `status` | `"unlocked"`, `"locked"` or `"sealed"` at the requested block |
| `word` | Raw lock word (32 bytes) |
| `delegatedTo` | EIP-7702 delegation target, or `null` if not delegated |

### `xdevs_submitEvidence`

Takes one parameter: the RLP of two conflicting votes signed by the same validator for the same height, round and vote type (`xdevs_consensus::ConflictingVotes`: `[[type, height, round, blockHash or empty for nil, validator, signature], [...]]`). The node verifies both Ed25519 signatures against the validator set of that height and that the evidence is at most 10,000 blocks old, then queues it: a block it proposes includes it and `Staking.slash` slashes and tombstones the validator (see [Validators](/operators/validators)). Returns the evidence hash (`keccak256` of the canonical RLP). Invalid evidence is refused with `-32602`. Nodes also collect the equivocations their consensus engine observes, without this call.

### `xdevs_getCommitCertificate`

Takes a block number, tag or hash. Returns `null` for unknown blocks and for blocks without a certificate (genesis, dev-mode blocks).

```json
{
  "height": "0x2a",
  "round": "0x0",
  "blockHash": "0x…",
  "signatures": [
    { "validator": "0x…(20 bytes)", "signature": "0x…(64-byte Ed25519)" }
  ],
  "raw": "0x…(canonical RLP of the certificate)"
}
```

Signatures are sorted by validator address (`sha256(ed25519 public key)[..20]`). A client can verify `raw` against the validator set.

### Safety methods (Phase 4)

These methods follow principle 7 of the plan, **facts vs opinions**. On-chain data is reported as fact. Every warning, flag and look-alike match is **advice, not fact**, and each result says so in a `disclaimer` field. Wallets should show these results as advice, never as a guarantee.

All of them run under the normal RPC limits:

- Each view call made on your behalf (`allowance`, `getAction`, `resolveAddr`, ...) is an `eth_call` with at most 300,000 gas. It also runs under the node's EVM time limit and the per-connection concurrency limit.
- Log scans use the `eth_getLogs` limits: at most `max_logs_range` blocks (default 10,000) and `max_logs_results` logs (default 10,000).
- Every step of a request reads the same block. A request that takes longer than 20 s fails with `-32005`.

The node only looks at recent history. The indexer API behind the explorer is the full-history source.

#### `xdevs_simulateTransaction`

`xdevs_simulateTransaction(tx, block?)` runs `tx` in revm against the state of `block` (default `latest`). It uses the same gas cap, time limit and cancellation as `eth_call`, and nothing is committed.

- `tx` is an `eth_call` transaction object, and `from` is required.
- Fees are not simulated: `gasPrice` and `maxFeePerGas` are ignored for execution. A sender that could not pay `gas × maxFee + value` gets an `INSUFFICIENT_FUNDS` warning.
- A transaction that would revert is not an RPC error. It returns `success: false`, the reason, and no asset changes.

```bash
curl -s http://127.0.0.1:8545 -H 'content-type: application/json' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "xdevs_simulateTransaction",
  "params": [{
    "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
    "to": "0x0000000000000000000000000000000000001009",
    "data": "0x095ea7b3000000000000000000000000000000000000000000000000000000000000beefffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
  }, "latest"]
}'
```

```json
{
  "success": true,
  "gasUsed": "0xb4f4",
  "assetChanges": [],
  "balanceChanges": [],
  "approvals": [
    {
      "kind": "erc20",
      "token": "0x0000000000000000000000000000000000001009",
      "symbol": "WXDS",
      "owner": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
      "spender": "0x000000000000000000000000000000000000bEEF",
      "amount": "unlimited",
      "rawAmount": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
      "unlimited": true,
      "expires": null
    }
  ],
  "contractsCreated": [],
  "warnings": [
    {
      "code": "UNLIMITED_APPROVAL",
      "severity": "high",
      "message": "This gives 0x0000…bEEF permission to spend ALL your WXDS, with no expiry."
    }
  ],
  "disclaimer": "advice, not fact: warnings and flags are heuristics; simulated effects may differ if state changes before the transaction is included",
  "blockNumber": "0x2a"
}
```

| Field | Meaning |
|---|---|
| `success`, `gasUsed` | Outcome and gas used. When `success` is false, `revertReason` (a decoded `Error(string)` or `Panic`, a halt reason, or `"execution reverted"`) and `revertData` are also returned |
| `assetChanges[]` | Every movement in execution order. Fields: `{token: "native" or address, standard: native/erc20/erc721/erc1155, symbol, decimals, from, to, amount, rawAmount, tokenId?}`. `amount` is scaled by `decimals`, or raw when decimals are unknown. WXDS wraps and unwraps appear as mints and burns |
| `balanceChanges[]` | Net change per address and asset: `{address, token, symbol, delta, rawDelta, tokenId?}`, signed (`"-12.5"`) |
| `approvals[]` | Approvals set or changed: `kind` (`erc20`, `xdrc20`, `permit2`, `erc721`, `erc721-all`, `erc1155-all`), `owner`, `spender`, `amount` (`"unlimited"` for at least 2^160−1, `"all"` for operator approvals, `"0"` for a revocation, otherwise scaled), `expires` (unix seconds or `null` for never) |
| `contractsCreated[]` | `{address, creator}` |
| `vault` | Present when the transaction goes through an xDEVS Smart Account: `{account, queued, actionId?, releaseAt?, executed?, cancelled?}`. When `queued` is true, the Guardian Vault held the action back, so nothing moves before `releaseAt` and the action can still be cancelled |
| `warnings[]` | `{code, severity: info/low/medium/high, message}`, highest severity first |

Simulation warning codes: `REVERT`, `UNLIMITED_APPROVAL` (high with no expiry, medium with one), `NEVER_EXPIRING_APPROVAL`, `SET_APPROVAL_FOR_ALL`, `OWNERSHIP_TRANSFER`, `HIGH_VALUE_TRANSFER` (more than half of the sender's balance), `SELF_DESTRUCT`, `CONTRACT_CREATION`, `BURN`, `VAULT_QUEUED`, `INSUFFICIENT_FUNDS` and `TRACE_TRUNCATED`. Token symbols and decimals are whatever the token contract reports, so they are not verified.

#### `xdevs_checkRecipient`

`xdevs_checkRecipient(to)` or `xdevs_checkRecipient(from, to, block?)`. `to` is an address or an XNS name (`alice` or `alice.xdevs`). Pass `null` as `from` to give a block without a sender.

On-chain **facts** and heuristic **advice** are kept in separate objects. The top-level `firstTime`, `lookAlikeOf`, `name`, `isContract` and `warnings` summarize them for older clients.

```jsonc
{
  "address": "0x1234…5679",
  "firstTime": true,
  "lookAlikeOf": ["0x1234…5678"],
  "name": null,
  "isContract": false,
  "warnings": [ /* same as advice.flags */ ],
  "facts": {
    "address": "0x1234…5679",
    "resolvedFrom": null,
    "isZeroAddress": false,
    "isPrecompile": false,
    "isSystemContract": false,
    "isContract": false,
    "codeSize": 0,
    "isSmartAccount": false,
    "delegatedTo": null,
    "eoaLock": "unlocked",
    "nonce": 0,
    "balance": "0x0",
    "primaryName": null
  },
  "advice": {
    "firstTime": true,
    "lookAlikeOf": ["0x1234…5678"],
    "flags": [
      { "code": "LOOKALIKE_RECIPIENT", "severity": "high", "message": "This address looks like 0x1234…5678 you sent to before (same first and last characters) — possible address poisoning. Compare every character." },
      { "code": "POISONING_PATTERN", "severity": "high", "message": "This address sent you a zero-value token transfer — a common address-poisoning trick. Do not copy addresses from your history." },
      { "code": "NEW_ADDRESS", "severity": "low", "message": "This address has never been used on chain (no transactions, no balance)." }
    ],
    "history": { "fromBlock": 90001, "toBlock": 100000, "complete": true },
    "disclaimer": "advice, not fact: …"
  },
  "blockNumber": "0x186a0"
}
```

- **Facts** cover code size and contract status, smart account status (an EIP-7702 delegation to, or an ERC-1967 proxy of, `0x…1005`), the XIP-0001 lock, nonce, balance, the forward-verified primary XNS name, and whether the address is the zero address, a precompile or a system contract.
- **Advice** compares the recipient with the token `Transfer` logs `from` sent in the scanned window (`history`):
  - `FIRST_TIME_RECIPIENT`: no earlier transfer from `from` to this address.
  - `LOOKALIKE_RECIPIENT`: same first and last 4 hex digits as a past recipient.
  - `POISONING_PATTERN`: the recipient sent `from` a zero-value transfer.
- **Other flags:** `ZERO_ADDRESS`, `PRECOMPILE`, `SYSTEM_CONTRACT`, `CONTRACT_RECIPIENT`, `NEW_ADDRESS` and `NAME_MISMATCH`.

The node does not index native transfers, so a wallet should combine `firstTime` with its own history. If the result cap is hit, the window shrinks and `history.complete` is `false`.

The XNS registry gives every live name a unique confusable skeleton, so a look-alike of a registered name can never resolve. An unregistered name fails with `-32602`. If it looks like a registered name, the error message names that name's address with a `LOOKALIKE_NAME` hint, for example `a1ice.xdevs is not registered (or expired); LOOKALIKE_NAME: it looks like a registered name that resolves to 0x… — check the spelling (advice, not fact)`.

#### `xdevs_getApprovals`

`xdevs_getApprovals(owner, options?, block?)` or `xdevs_getApprovals(owner, block)`, where `options = {tokens?: address[], fromBlock?: quantity}`.

The node scans the owner's approval events for candidate pairs: `Approval`, XDRC-20 `ApprovalWithExpiry`, `ApprovalForAll`, and Permit2Expiring `Approval`/`Permit`. It then reads each pair's live allowance, so revoked, spent and expired approvals drop out.

- By default the scan covers the last `max_logs_range` blocks. A `fromBlock` further back fails with `-32005`.
- At most 256 tokens and 256 `(token, spender)` pairs are allowed per request. More pairs fail with `-32005`; narrow the request with `tokens` or `fromBlock`, or use the indexer.

```json
[
  {
    "kind": "erc20",
    "token": "0x…",
    "symbol": "USDX",
    "owner": "0x…",
    "spender": "0x…",
    "amount": "unlimited",
    "rawAmount": "0xffff…ffff",
    "unlimited": true,
    "expires": null,
    "lastUpdatedBlock": "0x46",
    "transactionHash": "0x…"
  }
]
```

`amount` is the remaining raw amount as a decimal integer string, `"unlimited"` (at least 2^160−1), or `"all"` for operator approvals (`erc721-all`, `erc1155-all`). `kind` is `xdrc20` when the token reports an expiry (`allowanceExpiry`). Items are sorted newest first.

#### `xdevs_getVaultQueue`

`xdevs_getVaultQueue(account, block?)` reads the account's own state (`lastActionId`, `getAction`, `guardianVotes`). It returns the pending Guardian Vault actions, oldest first: actions that are queued, not cancelled, not executed, not invalidated by a recovery and not past their 14-day execution window. Only the latest 256 action ids are checked. An address that is not a smart account returns `[]`.

```json
[
  {
    "account": "0x…",
    "id": "0x2",
    "target": "0x…",
    "value": "0xde0b6b3a7640000",
    "data": "0x",
    "executeAfter": "0x6554e780",
    "expiresAt": "0x6567ac80",
    "status": "queued",
    "calls": [{ "target": "0x…", "value": "0xde0b6b3a7640000", "data": "0x" }],
    "cancelVotes": 0
  }
]
```

`status` is `queued` during the cancel window and `ready` once anyone may `execute(id)`. `target`, `value` and `data` mirror the `VaultActionQueued` event, and a batch is reported as the equivalent `executeBatch` self-call. For cancelled and executed history, use the indexer API.

#### `xdevs_resolveName` / `xdevs_reverseName`

- `xdevs_resolveName(name, block?)` returns the address a normalized `.xdevs` name resolves to (`XNSRegistry.resolveAddr`), or `null`.
- `xdevs_reverseName(address, block?)` returns the forward-verified primary name (`XNSRegistry.primaryName`), or `null`.

#### `eth_subscribe("vaultEvents")`

`["vaultEvents", {"account": "0x…"}]` streams Guardian Vault alerts from committed blocks. Each notification is:

```json
{ "type": "queued", "account": "0x…", "id": "0x7", "target": "0x…", "value": "0x…", "data": "0x", "executeAfter": "0x6553f100", "blockNumber": "0x2a", "blockHash": "0x…", "transactionHash": "0x…", "logIndex": "0x0" }
```

| `type` | Extra fields |
|---|---|
| `queued` | `id`, `target`, `value`, `data`, `executeAfter` |
| `cancelled` | `id`, `by` |
| `executed` | `id` |
| `invalidated` | `epoch` (a recovery invalidated everything queued before) |
| `frozen` | `frozenUntil` |
| `unfrozen` | none |

Always pass `account`. Without it, matching events from any contract are delivered, and only the emitter address proves which smart account an event belongs to. The same subscription limits as `logs` apply.

### Encrypted mempool (Phase 5 prototype)

Research-grade and **not enabled on any network**: a chain uses it only if its genesis
sets `encryptedMempoolBlock` and an `encryptedMempool` committee key. Design, threat
model and limitations: [`docs/research/encrypted-mempool.md`](https://github.com/JACELL100/xdevs-chain/blob/main/docs/research/encrypted-mempool.md).

An **envelope** is an ordinary signed transaction to `0x0000000000000000000000000000000000001100`
whose calldata is `0x01 ‖ keyId ‖ innerGasLimit (8 bytes) ‖ U ‖ c ‖ z ‖ body`: the
threshold encryption (secp256k1 hashed ElGamal + ChaCha20-Poly1305, with a proof of
knowledge bound to the envelope's sender) of a complete signed inner transaction. The
envelope pays its own gas in block N; the committee releases decryption shares once N is
final, and the inner transaction executes at the start of block N+1, in N's order. If the
envelope and the inner transaction come from the same account, the inner nonce is the
envelope's nonce + 1. Encrypt in the client (`xdevs_crypto::encrypted_tx::MempoolKey::seal`
in Rust); the RPC node never sees the plaintext.

#### `xdevs_getMempoolEncryptionKey`

No parameters. Returns `null` when the chain has no encrypted mempool, else:

```json
{
  "active": true,
  "activationBlock": "0x1",
  "keyId": "0x5c1f…",
  "publicKey": "0x02a1…",
  "threshold": "0x3",
  "committeeSize": "0x4",
  "inbox": "0x0000000000000000000000000000000000001100",
  "maxInnerGas": "0xe4e1c0",
  "payloadVersion": "0x1"
}
```

`active` says whether an envelope sent now lands in an encrypted-mempool block.
`maxInnerGas` bounds the inner gas commitment of one envelope (half the block gas limit,
shared by all envelopes of a block).

#### `xdevs_sendEncryptedTransaction`

Takes the raw signed envelope (`0x…` EIP-2718 bytes). The node checks that it is a call to
the inbox with a well-formed payload for the current key, an inner gas commitment within
`maxInnerGas` and a proof that verifies for the envelope's sender, then pools and gossips
it like `eth_sendRawTransaction` and returns the envelope's hash. Malformed envelopes are
refused with `-32000`. (Sending one through `eth_sendRawTransaction` works too: it is
included, pays gas and is never decrypted.) The inner transaction's receipt appears under
its own hash in block N+1.

## Not implemented yet

- Filter methods: `eth_newFilter`, `eth_newBlockFilter`, `eth_getFilterChanges`, `eth_uninstallFilter`.
- `eth_getProof`, `eth_createAccessList`.
- `debug_`, `trace_` and `txpool_` namespaces (`debug_traceTransaction` is planned for the explorer in Phase 3; these will not be public).
- Blob (type 3) transactions.

The RPC methods targeted for Phase 1–2 are listed in [§19 of the implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md).

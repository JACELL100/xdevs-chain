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
| `xdevs_validators(block?)` | Active validator set, power, uptime | <span class="status planned">Phase 3</span> |
| `xdevs_epochInfo(block?)` | Current epoch and next transition | <span class="status planned">Phase 3</span> |
| `xdevs_simulateTransaction(tx, block?)` | Asset changes and plain-language warnings before signing | <span class="status planned">Phase 4</span> |
| `xdevs_checkRecipient(from, to)` | First-time or look-alike recipient, XNS name, contract? | <span class="status planned">Phase 4</span> |
| `xdevs_getApprovals(owner, block?)` | All live approvals, for a revoke screen | <span class="status planned">Phase 4</span> |
| `xdevs_getVaultQueue(vault, block?)` | Pending Guardian Vault actions | <span class="status planned">Phase 4</span> |
| `xdevs_resolveName(name)` / `xdevs_reverseName(address)` | XNS lookups | <span class="status planned">Phase 4</span> |
| `xdevs_subscribe("vaultEvents", account)` | Real-time vault alerts (WebSocket) | <span class="status planned">Phase 4</span> |
| `xdevs_localFee(contract, block?)` | Local fee-market price for a contract | <span class="status planned">Phase 5</span> |

The plan originally named this method `xdevs_getCommit`; the node and the SDK's `getCommit` use `xdevs_getCommitCertificate`.

Planned methods currently answer "method not found" (`-32601`); the [SDK](/developers/sdk) turns that into `XdevsMethodNotAvailableError` with the phase number. Their JSON shapes are proposed in the [SDK spec](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/sdk.md) and may change before the node implements them.

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

## Not implemented yet

- Filter methods: `eth_newFilter`, `eth_newBlockFilter`, `eth_getFilterChanges`, `eth_uninstallFilter`.
- `eth_getProof`, `eth_createAccessList`.
- `debug_`, `trace_` and `txpool_` namespaces (`debug_traceTransaction` is planned for the explorer in Phase 3; these will not be public).
- Blob (type 3) transactions.

The RPC methods targeted for Phase 1–2 are listed in [§19 of the implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md).

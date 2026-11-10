# Configuration reference

The node reads a TOML file passed with `--config` (or `<data-dir>/config.toml` if it exists). **Every section and field is optional**; unknown keys and sections are ignored. Command-line flags override the file. `xdevs init --dev` and `xdevs-cli testnet init` write complete files you can start from.

Source of truth: [`crates/node/src/config.rs`](https://github.com/JACELL100/xdevs-chain/blob/main/crates/node/src/config.rs).

## Full example

```toml
[chain]
genesis = "genesis.json"     # omit in dev mode to use the built-in dev genesis
data_dir = ".xdevs/data"
pruning = "full"             # full (10,000 blocks of state history) | minimal (128) | archive
# state_history_blocks = 10_000   # optional: overrides the pruning mode's window

[dev]                        # single-node producer; only for a genesis without validators
enabled = true
block_time_ms = 2000
fee_recipient = "0x0000000000000000000000000000000000000000"

[validator]                  # absent / disabled => full node (genesis with validators)
enabled = true
key_file = "/var/lib/xdevs/keys/consensus.json"   # Ed25519 seed: keystore v3 or hex
password_file = "/var/lib/xdevs/keys/password"    # optional (else $XDEVS_KEY_PASSWORD)
fee_recipient = "0x..."
max_timestamp_drift_secs = 2

[consensus]                  # block time and round timeouts
block_time_ms = 2000
timeout_propose_ms = 1000
timeout_propose_delta_ms = 500
timeout_prevote_ms = 500
timeout_prevote_delta_ms = 250
timeout_precommit_ms = 500
timeout_precommit_delta_ms = 250
max_timeout_ms = 10000

[p2p]                        # Malachite consensus network
listen = "/ip4/0.0.0.0/tcp/30333"
persistent_peers = ["/dns4/val2/tcp/30333"]
node_key_file = "/var/lib/xdevs/keys/node_key.json"   # omit = ephemeral identity
discovery = false

[gossip]                     # transaction gossip network
enabled = true
listen = "/ip4/0.0.0.0/tcp/30334"
bootnodes = []
max_peers = 50

[rpc]
http = "127.0.0.1:8545"
ws = "127.0.0.1:8546"        # optional: extra listener (the http port also speaks WS)
cors = ["*"]
gas_cap = 50_000_000
max_logs_range = 10_000
max_batch = 50

[mempool]
max_txs = 10_000
max_bytes = 67_108_864
max_per_sender = 64

[log]
level = "info"
format = "text"
```

## Sections

### `[chain]`

| Key | Default | Meaning |
|---|---|---|
| `genesis` | built-in dev genesis | Path of `genesis.json` |
| `data_dir` | `.xdevs/data` | Database directory |
| `pruning` | `"full"` | State history window: `full` = 10,000 blocks, `minimal` = 128, `archive` = all. Blocks and receipts are always kept |
| `state_history_blocks` | none | Explicit window in blocks; overrides `pruning` |

### `[dev]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Produce blocks locally (only with a genesis that has no validators) |
| `block_time_ms` | chain config (`blockTimeMs`, 2000 in the dev genesis) | Block interval |
| `fee_recipient` | zero address | Receives priority fees |

### `[validator]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Sign as a consensus validator. Requires a genesis with validators |
| `key_file` | none | Ed25519 consensus key: encrypted keystore v3 or plain hex (dev only) |
| `password_file` | none | Keystore password file; otherwise the `XDEVS_KEY_PASSWORD` environment variable |
| `fee_recipient` | zero address | Receives priority fees of blocks this validator proposes |
| `max_timestamp_drift_secs` | `2` | How far a proposed block's timestamp may run ahead of the local clock |

### `[consensus]`

Block time and Tendermint round timeouts. Each `*_delta_ms` is added per extra round, capped at `max_timeout_ms`. All validators of a network should use the same values.

### `[p2p]` (consensus network)

| Key | Meaning |
|---|---|
| `listen` | Listen multiaddr, default `/ip4/0.0.0.0/tcp/30333` (TCP; QUIC later) |
| `persistent_peers` | Peers to keep connected, as multiaddrs |
| `node_key_file` | libp2p Ed25519 identity (`xdevs-cli keygen --node-key`). Omitted = a new identity on every start |
| `discovery` | Kademlia discovery; off in Phase 2 (static peers) |

### `[gossip]` (transaction gossip)

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Gossip transactions between nodes |
| `listen` | `/ip4/0.0.0.0/tcp/30334` | Listen multiaddr |
| `bootnodes` | `[]` | Peers dialled at start (usually ending in `/p2p/<peer id>`) |
| `max_peers` | `50` | Max connected peers |
| `max_tx_size` | 128 KiB max | Largest gossiped transaction |
| `quic` | `false` | Also listen on QUIC (off until a Linux soak) |
| `discovery` | `false` | Kademlia discovery |

### `[rpc]`

| Key | Default | Meaning |
|---|---|---|
| `http` | `127.0.0.1:8545` | HTTP listen address; also accepts WebSocket upgrades |
| `ws` | none | Optional separate WebSocket listener |
| `cors` | `["*"]` | Allowed CORS origins (empty = no CORS headers) |
| `namespaces` | `["eth", "net", "web3"]` | Parsed but not used for filtering yet: every implemented method is served |
| `gas_cap` | `50000000` | Gas cap of `eth_call` / `eth_estimateGas` |
| `max_logs_range` | `10000` | Max `eth_getLogs` block range |
| `max_batch` | `50` | Max JSON-RPC batch size |

### `[mempool]`

| Key | Default | Meaning |
|---|---|---|
| `max_txs` | `10000` | Max pooled transactions |
| `max_bytes` | `67108864` (64 MiB) | Max pooled bytes |
| `max_per_sender` | `64` | Max transactions per sender |

### `[log]`

| Key | Default | Meaning |
|---|---|---|
| `level` | `"info"` | `tracing` filter, e.g. `info`, `xdevs_rpc=debug`. `--log` and `RUST_LOG` override it |
| `format` | `"text"` | Only `text` is supported yet; other values log a warning |

### `[metrics]`

`xdevs-cli testnet init` writes a `[metrics] listen` address (Prometheus `/metrics`, served by the `xdevs-metrics` crate). Keep it on a private address. At the time of writing, starting this endpoint from the node binary is still being wired up, so check that your build serves it before relying on it.

## Genesis file

`genesis.json` follows the shape in [§21 of the implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md): a `config` object (`chainId`, `blockTimeMs`, `baseFee` parameters including `burnBps`, `treasury`, `xip0001Block`, ...), `gasLimit`, `alloc` (balances, code, storage, nonces) and, for consensus networks, `validators`. `xdevs init --dev` writes the dev genesis; [`deploy/configs/dev-genesis.json`](https://github.com/JACELL100/xdevs-chain/blob/main/deploy/configs/dev-genesis.json) is a copy.

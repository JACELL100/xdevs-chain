# Run a node

The node binary is `xdevs`. Today you can run it as a **single-node dev chain**, or as a **validator** or **full node** in a multi-validator network you create yourself. There is no public network to join yet; see [Network endpoints](/network/endpoints).

## Get the binaries

**Build from source** (Rust stable, pinned by `rust-toolchain.toml`, plus a C toolchain; on Windows the `x86_64-pc-windows-gnu` toolchain with MinGW gcc):

```bash
git clone https://github.com/JACELL100/xdevs-chain.git
cd xdevs-chain
cargo build --release      # target/release/xdevs, xdevs-cli, xdevs-loadgen
```

**Prebuilt Linux binaries:** the release workflow builds `xdevs` and `xdevs-cli` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, with SHA-256 checksums, whenever a `v*` tag is pushed. Check the [GitHub Releases](https://github.com/JACELL100/xdevs-chain/releases) page.

**Docker:** CI publishes an image used by the localnet scripts (`ghcr.io/jacell100/xdevs-chain:main`); see [Local multi-validator network](/operators/local-network#docker-localnet).

## Run modes

The mode follows from the genesis file and the config:

| Genesis | Config | Mode | What it does |
|---|---|---|---|
| No validators | `[dev] enabled = true` (or `--dev`) | **Dev** | One node seals a block every tick from its mempool |
| No validators | dev disabled | **RPC only** | Serves the stored chain, produces nothing |
| With validators | `[validator] enabled = true` + consensus key | **Validator** | Proposes, re-executes and votes (Malachite BFT) |
| With validators | validator disabled | **Full node** | Verifies and commits decided blocks, never signs; syncs from peers |

`[dev]` is ignored when the genesis has validators: such a chain only grows through consensus. Enabling `[validator]` with a genesis that has no validators is an error. If the consensus engine halts, `xdevs run` exits with an error instead of restarting in-process.

## Commands

```text
xdevs init --dev [--data-dir .xdevs] [--force]       # write dev genesis.json + config.toml
xdevs run --dev [--block-time 2s] [--http 127.0.0.1:8545] [--data-dir .xdevs]
xdevs run --config path/to/config.toml               # dev, validator or full node
```

`xdevs node` is an alias of `xdevs run`.

| `xdevs run` flag | Meaning |
|---|---|
| `--dev` | Single-node dev chain (built-in dev genesis unless `--genesis` or the config says otherwise) |
| `--config FILE` | Path to `config.toml` (default: `<data-dir>/config.toml` if it exists) |
| `--genesis FILE` | Path to `genesis.json` (overrides the config) |
| `--data-dir DIR` | Node home; the database lives in `<data-dir>/data` (default `.xdevs`) |
| `--block-time D` | Dev block interval, e.g. `2s`, `500ms`, `1.5s` |
| `--http ADDR` | JSON-RPC listen address (HTTP + WebSocket) |
| `--fee-recipient ADDR` | Receives the priority fees of produced or proposed blocks |
| `--log FILTER` | Log filter, e.g. `info`, `debug`, `xdevs::producer=debug` (`RUST_LOG` wins) |

## Storage and restarts

- Chain data is stored in `redb` with **one atomic write per block**. A restarted node resumes from the same head and state root, and re-verifies its head state root at startup (it refuses to start on a mismatch).
- Consensus state (Malachite WAL and the double-sign guard) lives in `<data_dir>/consensus`, apart from the chain database, so wiping one never resets the other.
- The state root is computed **incrementally** over trie nodes persisted in the database.
- State is queryable for the last 10,000 blocks by default (`[chain] pruning`); all blocks and receipts are kept (block pruning is not implemented yet).

## Running as a service

The repository includes a systemd unit at [`deploy/systemd/xdevs.service`](https://github.com/JACELL100/xdevs-chain/blob/main/deploy/systemd/xdevs.service). Keep JSON-RPC on `127.0.0.1` unless it sits behind a proxy with rate limiting; the planned public endpoints use Cloudflare in front of the node.

## Load testing

`xdevs-loadgen` sends a configurable transaction mix and reports inclusion:

```bash
./target/release/xdevs-loadgen --rpc http://127.0.0.1:8545 --tps 100 --duration 60s \
    --senders 10 --mix transfer=80,erc20=20
```

Use `--rpc` with several comma-separated URLs to spread sends across nodes, `--fund-from dev:0` to create and fund fresh senders, `--report json --out report.json` for machine-readable output, and `--min-inclusion 0.95` to fail when too few transactions land. It reports what it measured on your hardware; xDEVS Chain doesn't publish TPS figures yet.

## Validators on a public network

Joining a public network as a validator (registering stake in the Staking contract, rewards, slashing) is **Phase 3**. See [Validators](/operators/validators).

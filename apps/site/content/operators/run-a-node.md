# Run a node

The node binary is `xdevs`. You can run it as a **full node that follows Enclave**, the live public testnet; as a **single-node dev chain** for offline development; or as a **validator** or **full node** in a multi-validator network you create yourself. The Enclave validators run release [`v0.3.0-enclave.1`](https://github.com/JACELL100/xdevs-chain/releases/tag/v0.3.0-enclave.1).

## Get the binaries

**Build from source** (Rust stable, pinned by `rust-toolchain.toml`, plus a C toolchain; on Windows the `x86_64-pc-windows-gnu` toolchain with MinGW gcc):

```bash
git clone https://github.com/JACELL100/xdevs-chain.git
cd xdevs-chain
cargo build --release      # target/release/xdevs, xdevs-cli, xdevs-loadgen
```

**Prebuilt Linux binaries:** the release workflow builds `xdevs` and `xdevs-cli` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, with SHA-256 checksums, whenever a `v*` tag is pushed. The current Enclave release is [`v0.3.0-enclave.1`](https://github.com/JACELL100/xdevs-chain/releases/tag/v0.3.0-enclave.1) ("Latest" on [GitHub Releases](https://github.com/JACELL100/xdevs-chain/releases)).

**Docker:** CI publishes an image used by the localnet scripts (`ghcr.io/jacell100/xdevs-chain:main`); see [Local multi-validator network](/operators/local-network#docker-localnet).

## Follow Enclave with a full node

Most users and apps don't need a node: the public RPC `https://rpc.xdevschain.tech` serves Enclave (see [Network endpoints](/network/endpoints)). Run your own full node when you want your own RPC without the public limits, or to prepare for running a validator later.

:::warning[Peering is allow-listed today]
The four Enclave validators accept P2P connections **only from each other** (an allow-list in their firewalls), and there are no public bootnodes yet. A new full node therefore can't sync until the maintainers add its IP: ask on [GitHub](https://github.com/JACELL100/xdevs-chain/issues) with your node's IP and peer id. Until then, use the public RPC.
:::

1. **Binary.** Use node release [`v0.3.0-enclave.1`](https://github.com/JACELL100/xdevs-chain/releases/tag/v0.3.0-enclave.1) or newer (Linux `x86_64` / `aarch64`, with SHA-256 checksums). Older binaries can't decode blocks from **97,200** on, when the staking-driven validator set activates, and stop following the chain there.
2. **Genesis.** No ready-made Enclave `genesis.json` is attached to the releases yet. It is built by the deterministic [genesis builder](https://github.com/JACELL100/xdevs-chain/tree/main/tools/genesis-builder) from [`tools/genesis-builder/configs/enclave.json`](https://github.com/JACELL100/xdevs-chain/blob/main/tools/genesis-builder/configs/enclave.json) (Node.js 22.18+ and Foundry, from a checkout of the repository):
   ```bash
   node tools/genesis-builder/genesis-builder.ts build \
     --config tools/genesis-builder/configs/enclave.json --out genesis.json
   ```
   After the node starts, compare block 0 with the network: `cast block 0 -f hash` against your node and against `https://rpc.xdevschain.tech` must print the same hash. If they differ (for example because contract sources changed since launch), get the exact `genesis.json` from the maintainers instead of guessing.
3. **Config.** A full node is a node without `[validator]`. The validators' consensus addresses are the `p2p` multiaddrs in `enclave.json`:
   ```toml
   [chain]
   genesis = "genesis.json"
   data_dir = "/var/lib/xdevs/data"
   pruning = "full"                  # 10,000 blocks of state history; "archive" keeps all

   [p2p]
   listen = "/ip4/0.0.0.0/tcp/30333"
   persistent_peers = [
     "/ip4/140.245.245.54/tcp/30333/p2p/12D3KooWP65HsosrdGnXhdcJKkeQNyRzzFAK9aGb5P38Q2EvtRnU",
     # ... the other validators from enclave.json
   ]
   node_key_file = "node_key.json"   # xdevs-cli keygen --node-key --out node_key.json

   [rpc]
   http = "127.0.0.1:8545"           # keep it local, or put a rate-limiting proxy in front

   [execution]
   parallel = true                   # Block-STM, as on the Enclave validators; worth it from about 4 cores
   ```
4. **Run** `xdevs run --config config.toml`. The node syncs from genesis, verifies every block's commit certificate and re-executes it, then follows the head. Check it with `cast block-number --rpc-url http://127.0.0.1:8545`.

See [Configuration reference](/operators/configuration) for every key.

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

The repository includes a systemd unit at [`deploy/systemd/xdevs.service`](https://github.com/JACELL100/xdevs-chain/blob/main/deploy/systemd/xdevs.service). Keep JSON-RPC on `127.0.0.1` unless it sits behind a proxy with rate limiting; the public Enclave RPC runs on a full node behind a Cloudflare Tunnel and opens no inbound port.

## Load testing

`xdevs-loadgen` sends a configurable transaction mix and reports inclusion:

```bash
./target/release/xdevs-loadgen --rpc http://127.0.0.1:8545 --tps 100 --duration 60s \
    --senders 10 --mix transfer=80,erc20=20
```

Use `--rpc` with several comma-separated URLs to spread sends across nodes, `--fund-from dev:0` to create and fund fresh senders, `--report json --out report.json` for machine-readable output, and `--min-inclusion 0.95` to fail when too few transactions land. It reports what it measured on your hardware; xDEVS Chain doesn't publish TPS figures yet.

## Validators on Enclave

Enclave has a static set of four validators until the staking-driven set activates at block 97,200; becoming a validator is not open to the public yet. See [Validators](/operators/validators).

# xDEVS Chain

xDEVS Chain is an EVM-equivalent, Proof-of-Stake Layer 1 written in Rust, built by **THE xDEVS**. It runs on `revm` and `alloy`, so Ethereum tooling (MetaMask, Foundry, ethers/viem) works unchanged. The native coin is **XDS** (testnet: **tXDS**, 18 decimals; the smallest unit is the "dev").

> **Status: Phase 1 in progress.** The goal is a single-node dev chain that MetaMask and Foundry can talk to. Consensus, p2p networking and multi-node networks arrive in Phase 2 and later. Expect parts of the commands below to be unimplemented.

## Workspace layout

```
crates/
  primitives/   tx, block, header, receipt types and roots (alloy)
  crypto/       secp256k1, p256, ed25519, keccak, keystores
  storage/      Database trait, redb + in-memory backends, tables
  state/        state provider, state diffs, MPT state root
  execution/    revm block executor, eth_call / estimateGas
  genesis/      genesis.json parsing and initial state
  mempool/      transaction pool
  rpc/          jsonrpsee server: eth_, net_, web3_
  node/         config, wiring, dev block production
bin/
  xdevs/        node binary
  xdevs-cli/    keys and transaction sending
docs/           ADRs (docs/adr), specs (docs/specs), runbooks (docs/runbooks)
deploy/         systemd unit, local Docker setup
```

The shared Phase-1 interfaces are documented in [docs/specs/phase1-interfaces.md](docs/specs/phase1-interfaces.md). Design decisions are in [docs/adr/](docs/adr/).

## Prerequisites

- Rust stable (the version is pinned in `rust-toolchain.toml`) with `rustfmt` and `clippy`.
- A C toolchain for your platform (on Windows, the `x86_64-pc-windows-gnu` toolchain with MinGW gcc).
- Optional: [Foundry](https://book.getfoundry.sh/) and MetaMask for testing against the dev chain.

## Quickstart (target UX)

```bash
# Build (debug builds locally; see "CI and releases" below)
cargo build

# Local single-node dev chain (Phase 1): chain id 7117, RPC on 127.0.0.1:8545
./target/debug/xdevs node --dev

# Keys and transactions
xdevs-cli keygen --out alice.json
xdevs-cli send --private-key 0x<dev-key> --to 0xBob... --value 1000000000000000000 --rpc http://127.0.0.1:8545

# Talk to it with Foundry
cast chain-id --rpc-url http://127.0.0.1:8545     # 7117
cast block-number --rpc-url http://127.0.0.1:8545
```

The plan's §33.3 target UX (`xdevs init --dev`, `xdevs run --dev --block-time 2s`, `xdevs-cli send --from alice.json --value 10`, multi-node Docker Compose) will be adopted as those features land. Today's binary uses `xdevs node`.

## Add the local dev chain to MetaMask

**Chain id 7117 is for local development only.** It is not a public network, and its dev keys are public.

| Field | Value |
|---|---|
| Network name | xDEVS Dev (local) |
| RPC URL | `http://127.0.0.1:8545` |
| Chain ID | `7117` |
| Currency symbol | `tXDS` |
| Block explorer | (none) |

In MetaMask: Settings → Networks → Add a network manually, then enter the values above.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the rules (determinism, Ethereum compatibility, ADR process).

## CI and releases

Heavy builds run in GitHub Actions, because local disk space is limited. These include release builds and the cross-architecture x86_64/ARM64 matrix:

- `.github/workflows/ci.yml` runs fmt, clippy, tests and `cargo audit` on `ubuntu-latest` and `ubuntu-24.04-arm`.
- `.github/workflows/release.yml` runs on a `v*` tag. It builds `xdevs` and `xdevs-cli` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, writes SHA-256 checksums and publishes a GitHub Release.

Locally, prefer `cargo check -p <crate>` / `cargo test -p <crate>` and avoid `--release`.

## Roadmap

The full design and phase plan is in [xDEVS_Chain_Implementation_Plan.md](xDEVS_Chain_Implementation_Plan.md).

## Security

See [SECURITY.md](SECURITY.md). Do not report vulnerabilities in public issues.

## License

Apache-2.0. See [LICENSE](LICENSE).

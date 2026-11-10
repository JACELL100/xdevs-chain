# xDEVS Chain

**Website:** [xdevschain.xdevs.in](https://xdevschain.xdevs.in) (coming soon) · **Built by [THE xDEVS](https://xdevs.in)**

xDEVS Chain is an EVM-equivalent, Proof-of-Stake Layer 1 written in Rust, built around one idea: **protecting users at the protocol level** from scams, lost keys, front-running and drained wallets. It runs on `revm` and `alloy`, so Ethereum tooling (MetaMask, Foundry, Hardhat, viem, ethers) works unchanged.

The native coin is **XDS** (testnet: **tXDS**, 18 decimals; the smallest unit is the "dev"). Names end in `.xdevs` (XNS, the xDEVS Name Service).

> **Status: Phase 1 working, public testnet not live yet.** A single-node dev chain runs locally and works with MetaMask and Foundry. The system and user-safety contracts, the TypeScript SDK and the faucet are built and tested. Multi-node BFT consensus (Phase 2) and the public devnet (Phase 3) are next. Testnet tokens have no value.

## What's in this repo

```
crates/            Rust node (xdevs-* crates)
  primitives/      block, header, tx and chain config types (alloy)
  crypto/          sender recovery, signing, Web3 Secret Storage keystores
  storage/         redb tables with atomic per-block write batches
  state/           flat state and Ethereum MPT state roots
  execution/       revm block executor (Osaka rules), fees, eth_call / estimateGas
  genesis/         genesis.json parsing and the dev genesis
  mempool/         transaction pool
  rpc/             JSON-RPC server: eth_, net_, web3_
  node/            dev block producer, RPC backend, chain store
bin/
  xdevs/           node binary
  xdevs-cli/       keys, transfers, balances
tests/e2e/         end-to-end tests against an in-process node
contracts/         Solidity predeploys (Foundry): staking, governance, smart accounts,
                   Guardian Vault, XNS names, expiring approvals; see contracts/README.md
sdk/ts/            @xdevs/sdk: viem chain configs and typed xdevs_ RPC actions
apps/faucet/       tXDS faucet (Cloudflare Worker + Turnstile)
docs/              ADRs (docs/adr), specs (docs/specs), runbooks (docs/runbooks)
deploy/            dev genesis, systemd unit, local Docker setup
```

## Public endpoints (planned)

These are not live yet. They will be announced on [xdevschain.xdevs.in](https://xdevschain.xdevs.in) when the public testnet launches.

| Service | URL |
|---|---|
| Website and docs | `https://xdevschain.xdevs.in` |
| Testnet JSON-RPC | `https://xdevschain-rpc.xdevs.in` |
| Testnet WebSocket | `wss://xdevschain-ws.xdevs.in` |
| Block explorer | `https://xdevschain-explorer.xdevs.in` |
| Faucet | `https://xdevschain-faucet.xdevs.in` |

Service hosts use one subdomain level (`xdevschain-*.xdevs.in`) so they are covered by Cloudflare's free certificate for `*.xdevs.in`. The testnet chain id will be published here once it is registered on [chainlist](https://chainlist.org).

## Quickstart (local dev chain)

Prerequisites: Rust stable (pinned in `rust-toolchain.toml`) with `rustfmt` and `clippy`, and a C toolchain for your platform (on Windows, the `x86_64-pc-windows-gnu` toolchain with MinGW gcc). Optional: [Foundry](https://getfoundry.sh) and MetaMask.

```bash
# Build
cargo build

# Run a single-node dev chain: chain id 7117, JSON-RPC on 127.0.0.1:8545, a block every 2 s
./target/debug/xdevs run --dev --block-time 2s

# Or write a genesis.json + config.toml into a node home first, then run from it
./target/debug/xdevs init --dev --data-dir ./.xdevs
./target/debug/xdevs run --data-dir ./.xdevs

# Keys and transfers
./target/debug/xdevs-cli keygen --out alice.json
./target/debug/xdevs-cli balance 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
./target/debug/xdevs-cli send --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
    --to 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 --value 1.5 --wait

# Foundry works as on any EVM chain
cast chain-id --rpc-url http://127.0.0.1:8545        # 7117
cast block-number --rpc-url http://127.0.0.1:8545
```

The dev genesis pre-funds the 10 well-known Anvil/Hardhat test accounts (mnemonic `test test test test test test test test test test test junk`) with 10,000 tXDS each. **These keys are public: never use them outside a local dev chain.**

Current Phase 1 limits: only the latest state is kept (queries pinned to an older block return "historical state not available"), and WebSocket subscriptions (`eth_subscribe`) are not implemented yet.

## Add the local dev chain to MetaMask

**Chain id 7117 is for local development only.** It is not a public network.

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
# Rust node
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Solidity contracts (clone with --recurse-submodules first)
cd contracts && forge test

# SDK and faucet (Node.js 24)
cd sdk/ts && npm ci && npm test
cd apps/faucet && npm ci && npm test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the rules (determinism, Ethereum compatibility, ADR process). Design decisions are in [docs/adr/](docs/adr/) and component specs in [docs/specs/](docs/specs/).

## CI and releases

Heavy builds run in GitHub Actions rather than on developer laptops:

- `.github/workflows/ci.yml` checks the Rust workspace on x86_64 and ARM64 (fmt, clippy, tests), runs `cargo audit`, and tests the contracts, SDK and faucet. Accepted audit advisories are documented with a reason and re-check date in `.cargo/audit.toml`.
- `.github/workflows/release.yml` runs on a `v*` tag. It builds `xdevs` and `xdevs-cli` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, writes SHA-256 checksums and publishes a GitHub Release.

## Roadmap

The full design and phase plan is in [xDEVS_Chain_Implementation_Plan.md](xDEVS_Chain_Implementation_Plan.md).

## Security

See [SECURITY.md](SECURITY.md). Do not report vulnerabilities in public issues.

## License

Apache-2.0. See [LICENSE](LICENSE).

# xDEVS Chain

**Website:** [xdevschain.xdevs.in](https://xdevschain.xdevs.in) (coming soon) · **Built by [THE xDEVS](https://xdevs.in)**

xDEVS Chain is an EVM-equivalent, Proof-of-Stake Layer 1 written in Rust, built around one idea: **protecting users at the protocol level** from scams, lost keys, front-running and drained wallets. It runs on `revm` and `alloy`, so Ethereum tooling (MetaMask, Foundry, Hardhat, viem, ethers) works unchanged.

The native coin is **XDS** (testnet: **tXDS**, 18 decimals; the smallest unit is the "dev"). Names end in `.xdevs` (XNS, the xDEVS Name Service).

> **Status: Phase 1 working, public testnet not live yet.** A single-node dev chain runs locally and works with MetaMask and Foundry. The system and user-safety contracts, the TypeScript SDK and the faucet are built and tested. Multi-node BFT consensus (Phase 2) and the public devnet (Phase 3) are next. Testnet tokens have no value.

## Why xDEVS Chain?

Most blockchains compete on speed and fees. Meanwhile the things that actually cost ordinary users money (lost seed phrases, phishing and address poisoning, forgotten unlimited token approvals, front-running bots, hacked bridges) are left for wallets and apps to fix, or not. xDEVS Chain makes **user protection part of the chain itself**, while staying fully compatible with Ethereum tools.

### How it compares

| Problem users face | Ethereum | Solana | xDEVS Chain |
|---|---|---|---|
| **Lost seed phrase = lost funds** | Smart wallets exist as optional add-ons (ERC-4337) | Wallet-level only | Smart accounts are a built-in system contract: **passkey login (Face ID / fingerprint), no seed phrase**, social recovery by guardians |
| **Stolen key drains the wallet instantly** | Nothing at protocol level | Nothing at protocol level | **Guardian Vault**: large transfers, approvals and unknown contract calls wait out a cancel window, and switching protection off is delayed too. An opt-in protocol rule (XIP-0001) extends this to existing MetaMask addresses |
| **Forgotten unlimited approvals get drained years later** | ERC-20 approvals are unlimited and permanent by default | Token delegations persist until revoked | **Approvals expire**: the XDRC-20 standard and a Permit2-style contract that requires an expiry on every allowance, plus one-click "revoke all" |
| **Address poisoning and look-alike names** | Wallet warnings only; ENS still allows ASCII look-alikes such as `paypa1` | Wallet warnings only | Native `.xdevs` names that **reject look-alikes** (`paypa1` can't be registered while `paypal` exists), plus planned RPC checks for first-time and look-alike recipients |
| **Blind signing of unreadable transactions** | Third-party simulation services | Third-party simulation services | Planned built-in `xdevs_simulateTransaction` RPC that shows asset changes and plain-language warnings before you sign |
| **Front-running / sandwich bots (MEV)** | Private relays and builder markets, which centralise | Validator-level, widely exploited | Planned **encrypted mempool**: transactions stay hidden until their order is final |
| **Time until a payment is final** | About 13 minutes (2 epochs) | Seconds, with past full-network outages | Planned **BFT finality in about 1–2 seconds**, with no reorgs after commit |
| **One hot app makes everyone pay more** | Single global fee market | Local fee markets | Planned **local fee markets**: a congested contract gets its own fee; everyone else keeps paying the normal fee |
| **Bridge hacks** | Many bridges secured by small multisigs | Same | Policy: **light-client bridges only**, no multisig bridges |
| **Developer compatibility** | The standard | Different VM and languages | **EVM-equivalent** (revm, Osaka rules): Solidity, MetaMask, Foundry, Hardhat and viem work unchanged |

All protection features are **opt-in** and **never reverse a finalized transaction**: there is no admin key that can freeze or seize anyone's funds.

### What's built today vs planned

| Feature | Status |
|---|---|
| EVM-equivalent execution, MetaMask/Foundry-compatible JSON-RPC, local dev chain | ✅ Built and tested |
| Smart accounts with passkeys, social recovery, session keys | ✅ Contracts built and tested |
| Guardian Vault (time-locked transfers with a cancel window) | ✅ Contracts built and tested |
| Expiring approvals (XDRC-20, Permit2Expiring), `.xdevs` names with look-alike protection | ✅ Contracts built and tested |
| Staking, governance, treasury, gas sponsorship (paymasters) | ✅ Contracts built and tested |
| Guardian freeze (breaks owner-vs-thief recovery stalemates), EOA lock for existing MetaMask addresses | ✅ Contracts built and tested |
| Node enforcement of the EOA lock ([XIP-0001](docs/specs/xip-0001-eoa-lock.md)) | ✅ Built and tested (dev node: block validity, block building, mempool, RPC) |
| Multi-node BFT consensus with ~1–2 s finality, public testnet | 📋 Next (Phases 2–3) |
| Transaction simulation and recipient-check RPCs, reference wallet | 📋 Planned (Phase 4) |
| Encrypted mempool, parallel execution, local fee markets | 📋 Planned (Phase 5) |
| Light-client bridges | 📋 Planned (Phase 7) |

### Phase 1 node status (what the Rust node does today)

Works and is covered by tests (unit tests per crate plus in-process end-to-end tests driven only through JSON-RPC):

- `xdevs run --dev`: one node that produces a block every `--block-time` (default 2 s) from its mempool, with EIP-1559 fees (base fee split 80% burned / 20% treasury, tip to `--fee-recipient`).
- Legacy, EIP-2930, EIP-1559 and EIP-7702 (type 4, set-code) transactions; tx hashes, RLP, receipts root, tx root and state root computed the Ethereum way. Blob (type 3) transactions are rejected. Osaka precompiles, including P256VERIFY (secp256r1, EIP-7951) at `0x100` for passkeys.
- [XIP-0001](docs/specs/xip-0001-eoa-lock.md) EOA lock, active from genesis on dev chains (`"xip0001Block": 0`): an EIP-7702 account delegated to the xDEVS Smart Account with lock word 1 may only send value-0 calls to itself (one per block, gas ≤ 2,000,000, max fee ≤ 2 × base fee), with word 2 nothing at all, and EIP-7702 authorizations signed by a locked key are skipped. Enforced when executing and building blocks and on mempool admission (re-checked on every new head); `eth_sendRawTransaction` / `eth_estimateGas` return `-32003` with `data.reason` (`XIP0001_SEALED`, `XIP0001_NOT_SELF_CALL`, ...), and `xdevs_getEoaLock(address, block)` reports the lock state.
- Storage in `redb` with one atomic write per block; restarts resume from the same head and state root.
- JSON-RPC over HTTP (WebSocket transport on the same port): `eth_chainId`, `eth_blockNumber`, `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`, `eth_call`, `eth_estimateGas`, `eth_sendRawTransaction`, block / transaction / receipt lookups, `eth_getBlockReceipts`, `eth_getLogs`, `eth_gasPrice`, `eth_maxPriorityFeePerGas`, `eth_feeHistory`, `net_*`, `web3_*` and `xdevs_getEoaLock`.
- State queries at past blocks: the node keeps reverse state diffs for the last 10,000 blocks (`[chain] pruning = "full"`; `"minimal"` keeps 128, `"archive"` keeps all, `state_history_blocks` sets any window), so `cast ... --block N`, `anvil --fork-url` and `forge script` can pin a block while new blocks keep coming. Older blocks return `-32000 historical state not available`.
- Public-RPC limits from §19.3 of the plan: `eth_getLogs` at most 10,000 blocks per query and 10,000 results (the node stops collecting as soon as the cap is passed), `eth_call` gas cap 50M, batches of at most 50 requests, request/response size limits.

Not done yet:

- Multi-node consensus, peer-to-peer networking and state sync (Phase 2). Without `--dev` the node produces no blocks.
- `eth_subscribe` (WebSocket subscriptions), filter methods (`eth_newFilter`, `eth_getFilterChanges`, ...), `eth_getProof`, and `debug_` / `trace_` / `txpool_` namespaces.
- No pending block: the `pending` tag means `latest`, except that `eth_getTransactionCount` with `pending` counts pool transactions.
- Block and receipt pruning (all blocks are kept), an incremental state root (the root is recomputed from the full flat state each block, so large states get slow), and JSON log output.
- Dev block timestamps are whole seconds and at least parent + 1, so block times under 1 s run ahead of the wall clock.
- Graceful shutdown on Ctrl-C is tested in-process; it has not been checked by hand with a console Ctrl-C on Windows.
- XIP-0001 extras that depend on later phases: the `xdevs_simulateTransaction` warning (the method itself is Phase 4) and "do not gossip invalid transactions" (no gossip before Phase 2).

### Honest trade-offs

- **Security grows with adoption.** A new chain has far less stake and fewer independent validators than Ethereum. Until many independent operators join, it is a testnet, not a decentralized network.
- **Fast finality means a bounded validator set** (up to 100 on testnet), a deliberate choice of strong safety and speed over maximum decentralization.
- **MEV can be reduced, not eliminated**, and the encrypted mempool is research-grade work that will ship only after external review.
- **Protection is opt-in.** A user who turns nothing on gets ordinary Ethereum-style behaviour.

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
  rpc/             JSON-RPC server: eth_, net_, web3_, xdevs_getEoaLock
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

Current Phase 1 limits: state is readable for the last 10,000 blocks by default (older blocks return "historical state not available"), and WebSocket subscriptions (`eth_subscribe`) are not implemented yet. See [Phase 1 node status](#phase-1-node-status-what-the-rust-node-does-today).

## Run a local 4-validator network

```bash
# Keys, one shared genesis.json and node0..node3/config.toml (dev keys, unencrypted)
./target/debug/xdevs-cli testnet init --validators 4 --out ./testnet

# One terminal per validator (RPC on 127.0.0.1:30302, :30312, :30322, :30332)
./target/debug/xdevs run --config ./testnet/node0/config.toml
./target/debug/xdevs run --config ./testnet/node1/config.toml
./target/debug/xdevs run --config ./testnet/node2/config.toml
./target/debug/xdevs run --config ./testnet/node3/config.toml

cast block-number --rpc-url http://127.0.0.1:30332

# Options: --chain-id 7117 --block-time 2s --base-port 30300 --host 127.0.0.1 --full-nodes 1
#          --password-file pw.txt (encrypted consensus keys) --force (replace an existing testnet)
# Single keys: xdevs-cli keygen --consensus --out consensus_key.json
#              xdevs-cli keygen --node-key --out node_key.json
```

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

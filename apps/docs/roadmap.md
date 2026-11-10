# Roadmap

xDEVS Chain is built in phases, each with exit criteria. The full plan, with tasks and risks, is in the [implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md). Dates aren't promised: the project is built by a small team.

## Phase 0 — Foundations <span class="status ok">Done</span>

Repository, Apache-2.0 license, contribution and security policies, and the architecture decision records ([ADRs 0001–0008](https://github.com/JACELL100/xdevs-chain/tree/main/docs/adr)).

## Phase 1 — Single-node chain, MetaMask-compatible <span class="status ok">Done</span>

- Block, transaction and receipt types (legacy, EIP-2930, EIP-1559, EIP-7702), Ethereum-exact hashes and roots.
- `redb` storage with atomic per-block writes; Ethereum MPT state root; state history for past-block queries.
- revm executor (Osaka), EIP-1559 fees with 80% burn / 20% treasury.
- Mempool, dev block producer, Ethereum JSON-RPC, `xdevs-cli` (keys, transfers, balances).
- Exit criteria met: MetaMask sends tXDS; `forge create` and `cast` work; restarts keep the same state root.

## Phase 2 — Multi-node BFT network <span class="status ok">Essentially done</span>

- Malachite (Tendermint-style) consensus with execute-before-vote and stored commit certificates (`xdevs_getCommitCertificate`).
- libp2p networking: consensus network plus transaction gossip with mempool deduplication.
- Block sync for nodes that fall behind or join late; validator and full-node modes; persistent double-sign guard.
- WebSocket subscriptions: `newHeads`, `logs`, `newPendingTransactions`.
- Incremental state root over persisted trie nodes.
- `xdevs-cli testnet init`, `xdevs-loadgen`, an in-process multi-node test harness, and a Docker localnet with chaos scripts run in CI.
- Exit criteria: 4 nodes produce blocks; killing one keeps the chain going, killing two halts it and restarting resumes it, with no forks; a new node syncs from genesis. A 24-hour soak under load is part of the exit and runs on dedicated VMs.

## Phase 3 — Proof of Stake and public devnet <span class="status next">Next</span>

- System contracts in genesis (Staking, Governance, Treasury, FeeConfig, WXDS, the safety contracts, Multicall3, EntryPoint). <span class="status ok">Contracts written</span>
- Epoch system calls, dynamic validator set, rewards, downtime and double-sign slashing.
- Public devnet on free cloud VMs, faucet, explorer v1 and indexer, chainlist registration, dashboards and alerts.
- Release pipeline (x86_64 and ARM64) <span class="status ok">Built</span>, faucet <span class="status ok">Built</span>, this docs site <span class="status ok">Built</span>.
- Exit criteria: anyone can add the devnet to MetaMask, use the faucet, deploy contracts and see them in the explorer; a new validator joins through `register()`; slashing proven with a deliberately double-signing node.

## Phase 4 — User-safety layer <span class="status planned">Planned</span>

Most of the on-chain part is already written and tested ahead of schedule: smart accounts with passkeys, social recovery, session keys, Guardian Vault, guardian freeze, XNS, Permit2Expiring, XDRC-20, paymasters, EIP-7702 and the XIP-0001 EOA lock (enforced by the node). Still to do:

- `xdevs_simulateTransaction`, `xdevs_checkRecipient`, `xdevs_getApprovals`, `xdevs_getVaultQueue`, XNS RPCs and `vaultEvents` subscriptions.
- A reference wallet: passkey sign-up, vault setup, simulation preview, approvals manager, sending to names.
- An alert bot for vault events (email / Telegram).
- Exit criteria: a new user creates an account with only a passkey; the stolen-key demo works end to end in a wallet; simulation flags an unlimited approval in plain language.

## Phase 5 — Performance and MEV protection <span class="status planned">Planned</span>

Profiling and optimization, 1 s blocks if metrics allow, Block-STM parallel execution behind a flag, local fee markets, BLS aggregate signatures, and an **encrypted mempool** (research, prototype, then external review). Exit criteria include published benchmarks (throughput and finality latency) measured on reference hardware.

## Phase 6 — Public testnet and community <span class="status planned">Planned</span>

Long-lived `xdevs-testnet-1` with no resets, 20+ independent validators, snapshot sync, remote signers, an incentivized testnet (points, not tokens), a bug bounty, hackathons, and real governance on testnet.

## Phase 7 — Interop and mainnet preparation <span class="status planned">Later</span>

Light-client bridges only (no multisig bridges), canonical bridged tokens, independent security audits, legal and entity setup, mainnet tokenomics and a genesis ceremony.

::: info No mainnet date
There is no mainnet and no token sale. Testnet tokens (tXDS) have no value and will not convert to mainnet XDS.
:::

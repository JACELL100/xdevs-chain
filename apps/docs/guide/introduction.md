# Introduction

xDEVS Chain is an **EVM-equivalent, Proof-of-Stake Layer 1** written in Rust, built by the startup [THE xDEVS](https://xdevs.in). It is built around one idea: **protecting users at the protocol level** from scams, lost keys, front-running and drained wallets, while staying compatible with every Ethereum tool.

It executes transactions with [`revm`](https://github.com/bluealloy/revm) using Ethereum's Osaka rules and uses [`alloy`](https://github.com/alloy-rs/alloy) types, so MetaMask, Foundry, Hardhat, viem, ethers and Solidity work unchanged.

## At a glance

| | |
|---|---|
| Type | Layer 1, EVM-equivalent, Proof of Stake |
| Native coin | **XDS** (testnet: **tXDS**), 18 decimals. The smallest unit is the "dev" (1 XDS = 10¹⁸ dev) |
| Names | `.xdevs` (XNS, the xDEVS Name Service), for example `alice.xdevs` |
| Execution | `revm`, Osaka rules, including the P-256 (passkey) precompile at `0x100` |
| Consensus | Tendermint-style BFT via [Malachite](https://github.com/informalsystems/malachite), Ed25519 validator keys |
| Finality | Deterministic: a block is final when committed, no reorgs |
| Block time | 2 s at launch, 1 s target later |
| Fees | EIP-1559; 80% of the base fee is burned, 20% goes to the Treasury; tips go to the block producer |
| State | Ethereum Merkle-Patricia Trie (incremental root over persisted trie nodes), stored in `redb` |
| Networking | libp2p: one network for consensus, one for transaction gossip |
| Local dev chain id | `7117` |
| Public testnet chain id | To be announced |

## Where the project is today

::: tip Phases 1 and 2 work on your own machines
- A **single-node dev chain** (`xdevs run --dev`) produces blocks, serves Ethereum JSON-RPC, and works with MetaMask and Foundry.
- A **multi-validator BFT network** (`xdevs-cli testnet init --validators 4`) reaches consensus with Malachite, syncs late-joining nodes from genesis, gossips transactions between nodes, and keeps going with one of four validators down. A Docker localnet with chaos scripts (kill, pause, partition, packet loss) runs in CI. See [Local multi-validator network](/operators/local-network).
- **WebSocket subscriptions** (`eth_subscribe`: `newHeads`, `logs`, `newPendingTransactions`) and a load generator (`xdevs-loadgen`).
- The **system and user-safety contracts** (staking, governance, smart accounts, Guardian Vault, XNS names, expiring approvals, paymasters) are written and tested in Foundry.
- The node **enforces the EOA lock** ([XIP-0001](/concepts/eoa-lock)) in block validity, block building, the mempool and the RPC.
- The **TypeScript SDK** (`@xdevs/sdk`) and the **faucet** (Cloudflare Worker) are built and tested.
:::

::: warning Not yet
- There is **no public network**. The public devnet with faucet and explorer is Phase 3. The [planned endpoints](/network/endpoints) are not live.
- The validator set is still **fixed in the genesis file**. Joining through the Staking contract, rewards and slashing are Phase 3.
- The system contracts are **not yet in the dev genesis**: on the local dev chain the predeploy addresses are empty. A genesis builder that places them is part of Phase 3. See [Predeployed contracts](/concepts/predeploys).
- The safety RPCs (`xdevs_simulateTransaction`, `xdevs_checkRecipient`, ...) are specified and wrapped by the SDK, but the node implements them only from Phase 4.
- Nothing has had an external security audit.
:::

## Where to go next

- [Why xDEVS Chain?](/guide/why): the problems it solves, compared with Ethereum and Solana, and the honest trade-offs.
- [Quickstart](/guide/quickstart): run a dev chain on your machine and connect MetaMask and Foundry.
- [The user-safety layer](/concepts/safety-layer): how smart accounts, the Guardian Vault, expiring approvals and XNS fit together.
- [JSON-RPC reference](/developers/json-rpc): what the node supports today.

## Source and design documents

Everything is open source under Apache-2.0 at [github.com/JACELL100/xdevs-chain](https://github.com/JACELL100/xdevs-chain).

- [Implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md): the full design and phase plan.
- [Architecture decision records](https://github.com/JACELL100/xdevs-chain/tree/main/docs/adr): why revm + alloy, why Malachite, execute-before-vote, and more.
- [Specifications](https://github.com/JACELL100/xdevs-chain/tree/main/docs/specs): smart accounts, XIP-0001, XNS and approvals, system predeploys, SDK.

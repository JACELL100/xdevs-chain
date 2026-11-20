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
| Enclave testnet chain id | `717171` ([live](/network/endpoints)) |

## Where the project is today

:::tip[The Enclave testnet is live]
The public testnet **Enclave** (chain id 717171) has run since 17 November 2026 on four validators in Oracle Cloud (two in Hyderabad, two in Mumbai), with 2 s blocks and BFT finality. Start with the [Quickstart](/guide/quickstart):

- RPC `https://rpc.xdevschain.tech` · WebSocket `wss://ws.xdevschain.tech`
- [Explorer](https://explorer.xdevschain.tech) · [wallet](https://wallet.xdevschain.tech) (passkey smart accounts) · [faucet](https://faucet.xdevschain.tech) · [ERC-4337 bundler](/network/endpoints) · [status](https://status.xdevschain.tech)
:::

:::info[Live on Enclave now]
The validators run node release [`v0.3.0-enclave.1`](https://github.com/JACELL100/xdevs-chain/releases/tag/v0.3.0-enclave.1):

- Everything from Phases 1 and 2: Ethereum JSON-RPC and WebSocket subscriptions, Malachite BFT consensus, block sync, transaction gossip.
- The **system and user-safety contracts** in genesis at `0x…1000`–`0x…1009` (staking, governance, smart accounts, Guardian Vault, XNS names, expiring approvals, paymaster), plus Multicall3 and the ERC-4337 EntryPoint v0.9. See [Predeployed contracts](/concepts/predeploys).
- The [EOA lock](/concepts/eoa-lock) (XIP-0001), enforced by the node from genesis.
- The **Phase 4 safety RPCs**: `xdevs_simulateTransaction`, `xdevs_checkRecipient`, `xdevs_getApprovals`, `xdevs_getVaultQueue`, `xdevs_resolveName` / `xdevs_reverseName` and the `vaultEvents` subscription, plus `xdevs_localFee`.
- **Block-STM parallel execution** on all validators.
:::

:::info[Live since block 97,200]
Since block **97,200** (19 November 2026, about 21:45 UTC) the **staking-driven validator set** (epochs of 3,600 blocks, a 1 tXDS block reward, downtime and double-sign slashing), **local fee markets** and the **`ED25519VERIFY` precompile** are active on Enclave.
:::

:::warning[Not yet]
- The **encrypted mempool** is built but off on Enclave: it needs a distributed key setup first.
- The **light-client bridges** to Ethereum Sepolia are a prototype and not deployed.
- **Becoming a validator** on Enclave is not open to the public yet. See [Validators](/operators/validators).
- Nothing has had an external security audit, and tXDS has no value.
:::

You can also run everything on your own machines: a **single-node dev chain** (`xdevs run --dev`, chain id 7117) and **multi-validator BFT networks** (`xdevs-cli testnet init`, or a Docker localnet with chaos scripts). See the [Quickstart](/guide/quickstart#develop-offline-a-local-dev-chain) and [Local multi-validator network](/operators/local-network).

## Where to go next

- [Quickstart](/guide/quickstart): add Enclave to MetaMask, get tXDS from the faucet and deploy a contract with Foundry.
- [Why xDEVS Chain?](/guide/why): the problems it solves, compared with Ethereum and Solana, and the honest trade-offs.
- [The user-safety layer](/concepts/safety-layer): how smart accounts, the Guardian Vault, expiring approvals and XNS fit together.
- [JSON-RPC reference](/developers/json-rpc): what the node serves, and what is live on Enclave.

## Source and design documents

Everything is open source under Apache-2.0 at [github.com/JACELL100/xdevs-chain](https://github.com/JACELL100/xdevs-chain).

- [Implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md): the full design and phase plan.
- [Architecture decision records](https://github.com/JACELL100/xdevs-chain/tree/main/docs/adr): why revm + alloy, why Malachite, execute-before-vote, and more.
- [Specifications](https://github.com/JACELL100/xdevs-chain/tree/main/docs/specs): smart accounts, XIP-0001, XNS and approvals, system predeploys, SDK.

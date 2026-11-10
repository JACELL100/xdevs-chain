# Chain IDs & parameters

## Chain IDs

| Network | Chain id | Hex | Status |
|---|---|---|---|
| Local dev chain (`xdevs run --dev`) and local test networks (`xdevs-cli testnet init`) | **7117** | `0x1bcd` | <span class="status ok">Works today</span>, local only |
| Public testnet (`xdevs-testnet-1`) | **To be announced** | | <span class="status next">Coming soon</span> |
| Mainnet | Not planned yet | | |

::: warning About 717171
The SDK's `xdevsTestnet` and some examples use **717171** as a placeholder. It is not registered and may change. The real testnet chain id will be published on this site and registered on [chainlist](https://chainlist.org) when the public testnet launches.
:::

Chain id 7117 is for local development only. Never assume a transaction signed for 7117 is safe to replay elsewhere; any two people's local chains share it.

## Wallet parameters (local dev chain)

| Field | Value |
|---|---|
| Network name | xDEVS Dev (local) |
| RPC URL | `http://127.0.0.1:8545` |
| Chain ID | `7117` |
| Currency symbol | `tXDS` (18 decimals) |
| Block explorer | none |

The [SDK](/developers/sdk) can add the network with `addXdevsChainToWallet(wallet, xdevsDevnet)`.

## Protocol parameters

| Parameter | Dev chain today | Planned testnet v1 |
|---|---|---|
| Block time | 2 s (configurable) | 2 s, then 1 s target |
| Block gas limit | 30,000,000 | 30,000,000 (tuned by benchmark) |
| Min base fee | 0.1 gwei | 0.1 gwei |
| Base fee max change | 12.5% per block | 12.5% per block |
| Base fee split | 80% burn / 20% treasury | 80% burn / 20% treasury |
| Max tx size (gossip) | 128 KiB | 128 KB |
| EVM rules | Osaka | Osaka |
| XIP-0001 (EOA lock) | Active from genesis | Active from genesis |
| Epoch length | n/a (static validator set) | 3,600 blocks |
| Max validators | Set in genesis | 100 |
| Guardian Vault default delay | 24 h (1 h–7 d, per account) | Same |
| Social recovery delay | 48 h (1 h–30 d, per account) | Same |

Planned values come from §33.1 of the [implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md) and may change before launch.

## Transaction types

| Type | Supported |
|---|---|
| 0 Legacy (EIP-155) | Yes |
| 1 EIP-2930 access list | Yes |
| 2 EIP-1559 | Yes |
| 3 EIP-4844 blob | **No** (rejected) |
| 4 EIP-7702 set-code | Yes (subject to [XIP-0001](/concepts/eoa-lock)) |

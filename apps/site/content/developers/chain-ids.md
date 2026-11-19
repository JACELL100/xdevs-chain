# Chain IDs & parameters

## Chain IDs

| Network | Chain id | Hex | Status |
|---|---|---|---|
| Local dev chain (`xdevs run --dev`) and local test networks (`xdevs-cli testnet init`) | **7117** | `0x1bcd` | <span class="status ok">Works today</span>, local only |
| **Enclave** public testnet (`xdevs-enclave-1`) | **717171** | `0xaf173` | <span class="status ok">Live</span> since 17 Nov 2026 · [endpoints](/network/endpoints) |
| Internal devnet (`xdevs-devnet-1`) | **717170** | `0xaf172` | Reserved for short-lived internal networks |
| Mainnet | Not planned yet | | |

:::info[717171 is Enclave]
Chain id **717171** belongs to the Enclave testnet; it was unused on [chainid.network](https://chainid.network) when it was chosen, and it will be registered on [chainlist](https://chainlist.org). In the SDK it is `xdevsEnclave` (`xdevsTestnet` is a deprecated alias).
:::

Chain id 7117 is for local development only. Never assume a transaction signed for 7117 is safe to replay elsewhere; any two people's local chains share it.

## Wallet parameters (Enclave testnet)

| Field | Value |
|---|---|
| Network name | xDEVS Chain Enclave |
| RPC URL | `https://rpc.xdevschain.tech` |
| Chain ID | `717171` |
| Currency | EnclaveXDS, symbol `tXDS` (18 decimals) |
| Block explorer | `https://explorer.xdevschain.tech` |

## Wallet parameters (local dev chain)

| Field | Value |
|---|---|
| Network name | xDEVS Dev (local) |
| RPC URL | `http://127.0.0.1:8545` |
| Chain ID | `7117` |
| Currency symbol | `tXDS` (18 decimals) |
| Block explorer | none |

The [SDK](/developers/sdk) can add the network with `addXdevsChainToWallet(wallet, xdevsEnclave)` (or `xdevsLocal` for the local chain).

## Protocol parameters

| Parameter | Dev chain | Enclave testnet |
|---|---|---|
| Block time | 2 s (configurable) | 2 s (1 s target later) |
| Block gas limit | 30,000,000 | 30,000,000 (tuned by benchmark) |
| Min base fee | 0.1 gwei | 0.1 gwei |
| Base fee max change | 12.5% per block | 12.5% per block |
| Base fee split | 80% burn / 20% treasury | 80% burn / 20% treasury |
| Max tx size (gossip) | 128 KiB | 128 KB |
| EVM rules | Osaka | Osaka |
| XIP-0001 (EOA lock) | Active from genesis | Active from genesis |
| Epoch length | n/a (static validator set) | 3,600 blocks (staking set from block 97,200) |
| Block reward | none | 1 tXDS per block from block 97,200 |
| Local fee markets | off | Activate at block 97,200 |
| `ED25519VERIFY` precompile (`0x0A00…0001`) | off | Activates at block 97,200 |
| Encrypted mempool | off | Off (needs a distributed key setup first) |
| Max validators | Set in genesis | 4 in genesis today; up to 100 once the staking set activates at block 97,200 |
| Guardian Vault default delay | 24 h (1 h–7 d, per account) | Same |
| Social recovery delay | 48 h (1 h–30 d, per account) | Same |

Enclave values come from its genesis config, [`tools/genesis-builder/configs/enclave.json`](https://github.com/JACELL100/xdevs-chain/blob/main/tools/genesis-builder/configs/enclave.json); the targets behind them are in §33.1 of the [implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md). Values may change in later network upgrades.

## Transaction types

| Type | Supported |
|---|---|
| 0 Legacy (EIP-155) | Yes |
| 1 EIP-2930 access list | Yes |
| 2 EIP-1559 | Yes |
| 3 EIP-4844 blob | **No** (rejected) |
| 4 EIP-7702 set-code | Yes (subject to [XIP-0001](/concepts/eoa-lock)) |

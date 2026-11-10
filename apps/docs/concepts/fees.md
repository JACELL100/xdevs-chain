# Fees & the tXDS coin

## The coin

| | |
|---|---|
| Mainnet coin | **XDS** (no mainnet exists yet) |
| Testnet / dev coin | **tXDS** |
| Decimals | 18 |
| Smallest unit | "dev": 1 XDS = 10¹⁸ dev (the same ratio as ETH to wei) |

::: warning tXDS has no value
Testnet tokens have **no monetary value**, are free from the faucet, must never be sold, and **will not convert** to mainnet XDS.
:::

## How fees work (EIP-1559)

xDEVS Chain uses Ethereum's EIP-1559 fee model, so wallets estimate fees exactly as on Ethereum.

- Every block has a **base fee** that rises when blocks are more than half full and falls when they are less than half full, by at most **12.5% per block**.
- The base fee never drops below a **minimum of 0.1 gwei** (an anti-spam floor).
- Users add a **priority fee** (tip) on top.

### Where the fee goes

| Part | Destination |
|---|---|
| Base fee × gas used, **80%** | **Burned** |
| Base fee × gas used, **20%** | **Treasury** contract (`0x…1002`), spent on grants and public goods by governance |
| Priority fee (tip) | The block producer's fee recipient (`--fee-recipient` on the dev node) |

This split works today on the dev chain. Under heavy use the burn can exceed new issuance, making the supply deflationary.

### Dev chain values

From the built-in dev genesis:

| Parameter | Value |
|---|---|
| Initial base fee | 1 gwei |
| Minimum base fee | 0.1 gwei |
| Elasticity (target = gas limit / 2) | 2 |
| Max change denominator (12.5%) | 8 |
| Burn share | 8,000 bps (80%) |
| Block gas limit | 30,000,000 |
| Block time | 2 s |

On the dev node, the default `--fee-recipient` is the zero address, so tips go there unless you set one.

### Governance

On the public networks, the `FeeConfig` contract (`0x…1003`) holds parameters such as the gas limit, minimum base fee and burn share, and only Governance (a stake-weighted vote plus a timelock) can change them. FeeConfig is built and tested but not yet wired into the dev genesis.

## Planned: local fee markets (Phase 5)

On Ethereum, one popular mint makes everyone pay more. xDEVS Chain plans **local fee markets**: if a single contract uses more than 25% of block gas, transactions to it pay an extra local base fee that rises and decays like EIP-1559, while everyone else keeps paying the normal fee. The price will be exposed through `xdevs_localFee(address)`. The first version is keyed by the top-level `to` address only.

## Planned: gas sponsorship

Apps can pay their users' gas through the [paymaster](/concepts/smart-accounts#paymasters-gas-sponsorship), and an example paymaster accepts approved stablecoins. These are built as contracts; using them on a network needs the predeploys and an ERC-4337 bundler.

## Planned testnet supply

These numbers are from the [implementation plan](https://github.com/JACELL100/xdevs-chain/blob/main/xDEVS_Chain_Implementation_Plan.md) and apply to the future public testnet only. Mainnet tokenomics will be designed separately, with legal advice.

| | |
|---|---|
| Genesis supply | 1,000,000,000 tXDS |
| Issuance | 7% in year 1, −1% per year, floor 2%, paid as staking rewards |
| Burn | 80% of base fees |

| Genesis allocation | Share |
|---|---|
| Faucet & community reserve | 30% |
| Ecosystem grants | 25% |
| Treasury (governance) | 20% |
| Core development | 15% |
| Genesis validators' stake | 5% |
| Reserve (incentivized testnet) | 5% |

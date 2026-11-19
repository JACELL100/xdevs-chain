# Fees & the tXDS coin

## The coin

| | |
|---|---|
| Mainnet coin | **XDS** (no mainnet exists yet) |
| Testnet / dev coin | **tXDS** (on Enclave its name is EnclaveXDS); free from the [faucet](https://faucet.xdevschain.tech) |
| Decimals | 18 |
| Smallest unit | "dev": 1 XDS = 10¹⁸ dev (the same ratio as ETH to wei) |

:::warning[tXDS has no value]
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
| Priority fee (tip) | The block producer's fee recipient (`[validator] fee_recipient`, or `--fee-recipient` on the dev node) |

This split is live on Enclave and on the dev chain. Under heavy use the burn can exceed new issuance, making the supply deflationary.

### Values on Enclave and the dev chain

From the Enclave genesis and the built-in dev genesis (the same values):

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

On the public networks, the `FeeConfig` contract (`0x…1003`) holds parameters such as the gas limit, minimum base fee and burn share, and only Governance (a stake-weighted vote plus a timelock) can change them. FeeConfig is in the Enclave genesis; the built-in dev genesis doesn't contain it.

## Local fee markets (activate at block 97,200 on Enclave)

On Ethereum, one popular mint makes everyone pay more. xDEVS Chain has **local fee markets**: if a single contract uses more than 25% of block gas, transactions to it pay an extra local base fee that rises and decays like EIP-1559, while everyone else keeps paying the normal fee. The price is exposed through `xdevs_localFee(address)`, already served by the public RPC (`active: false` until the activation). The first version is keyed by the top-level `to` address only. Details: [JSON-RPC reference](/developers/json-rpc#local-fee-markets-phase-5).

## Gas sponsorship

Apps can pay their users' gas through the [paymaster](/concepts/smart-accounts#paymasters-gas-sponsorship) (`0x…1007`, in the Enclave genesis), and an example paymaster accepts approved stablecoins. On Enclave, sponsored UserOperations go through the public [ERC-4337 bundler](/network/endpoints); the [wallet](https://wallet.xdevschain.tech) uses it to sponsor the activation of new passkey accounts.

## Enclave supply

From the Enclave genesis config ([`enclave.json`](https://github.com/JACELL100/xdevs-chain/blob/main/tools/genesis-builder/configs/enclave.json)). Mainnet tokenomics will be designed separately, with legal advice.

| | |
|---|---|
| Genesis supply | 1,000,000,000 tXDS |
| Issuance | None before block 97,200; from then on a fixed 1 tXDS block reward to the Staking contract (about 1.6% of the supply per year, below the plan's 7% schedule until its inflation curve is implemented) |
| Burn | 80% of base fees |

| Genesis allocation | Share |
|---|---|
| Faucet, grants, core development and reserve (held by the first validator's operator account, which funds the faucet) | 75% |
| Treasury (governance) | 20% |
| Genesis validators' stake (4 × 12,500,000 tXDS) | 5% |

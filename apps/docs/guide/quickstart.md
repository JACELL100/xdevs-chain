# Quickstart: local dev chain

This guide runs a **single-node xDEVS dev chain** on your machine, connects MetaMask, funds an account and deploys a contract with Foundry. It takes about 15 minutes, most of it spent compiling.

::: info Local only
The dev chain (chain id **7117**) runs only on your computer. There is no public xDEVS network yet; see [Network endpoints](/network/endpoints) for what is planned. To run several validators with real BFT consensus instead, see [Local multi-validator network](/operators/local-network).
:::

## 1. Prerequisites

- **Rust** stable. The exact version is pinned in [`rust-toolchain.toml`](https://github.com/JACELL100/xdevs-chain/blob/main/rust-toolchain.toml), so `rustup` picks it up automatically.
- A **C toolchain** for your platform. On Windows, use the `x86_64-pc-windows-gnu` toolchain with MinGW gcc.
- Optional: [Foundry](https://getfoundry.sh) (`forge`, `cast`) and [MetaMask](https://metamask.io).

## 2. Build and run the node

```bash
git clone https://github.com/JACELL100/xdevs-chain.git
cd xdevs-chain
cargo build

# A single-node dev chain: chain id 7117, JSON-RPC on 127.0.0.1:8545, a block every 2 s
./target/debug/xdevs run --dev --block-time 2s
```

On Windows the binaries are `target\debug\xdevs.exe` and `target\debug\xdevs-cli.exe`.

The node stores its database in `./.xdevs/data` and resumes from the same head after a restart. To start over from block 0, stop the node and delete the `.xdevs` folder.

You can also write the genesis and config files into a node home first, edit them, and run from there:

```bash
./target/debug/xdevs init --dev --data-dir ./.xdevs   # writes genesis.json + config.toml
./target/debug/xdevs run --data-dir ./.xdevs
```

See [Run a node](/operators/run-a-node) for every flag and [Configuration reference](/operators/configuration) for `config.toml`.

Check that it is up:

```bash
cast chain-id --rpc-url http://127.0.0.1:8545       # 7117
cast block-number --rpc-url http://127.0.0.1:8545   # grows every 2 s
```

## 3. Dev accounts and funds

The dev genesis pre-funds the 10 well-known Anvil/Hardhat test accounts (mnemonic `test test test test test test test test test test test junk`) with **10,000 tXDS each**. The first two:

| | Address | Private key |
|---|---|---|
| Account 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| Account 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |

::: danger These keys are public
Everyone knows these private keys. Use them **only** on a local dev chain, and never send real funds to these addresses on any network.
:::

## 4. Add the dev chain to MetaMask

In MetaMask: **Settings → Networks → Add a network → Add a network manually**, then enter:

| Field | Value |
|---|---|
| Network name | xDEVS Dev (local) |
| RPC URL | `http://127.0.0.1:8545` |
| Chain ID | `7117` |
| Currency symbol | `tXDS` |
| Block explorer URL | (leave empty) |

Chain id 7117 is for local development only. It is not a public network.

To get funds into MetaMask, either import one of the dev private keys above (**Add account → Import account**), or create a fresh MetaMask account and send it tXDS from a dev account (next step).

::: tip Reset MetaMask after wiping the chain
If you delete `.xdevs` and restart from block 0, MetaMask still remembers old nonces. Use **Settings → Advanced → Clear activity tab data** for the dev network.
:::

## 5. Send tXDS with `xdevs-cli`

```bash
# Balance of a dev account
./target/debug/xdevs-cli balance 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

# Send 1.5 tXDS from dev account 0 to another address and wait for the receipt
./target/debug/xdevs-cli send \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --to 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  --value 1.5 --wait
```

Replace `--to` with your own MetaMask address to fund it.

For your own keys, create an encrypted keystore (Web3 Secret Storage v3) instead of passing raw private keys:

```bash
./target/debug/xdevs-cli keygen --out alice.json         # prompts for a password
./target/debug/xdevs-cli address --key alice.json
./target/debug/xdevs-cli send --from alice.json --to 0x... --value 0.25 --wait
```

`xdevs-cli` commands:

| Command | What it does |
|---|---|
| `keygen [--out FILE] [--light-kdf] [--force]` | New secp256k1 key. Without `--out` the private key is printed (dev only). `--light-kdf` uses cheap scrypt parameters for throwaway keys. |
| `address --key FILE` | Prints the address of a keystore. |
| `send --to ADDR --value AMOUNT (--from FILE \| --private-key HEX)` | Signs and sends an EIP-1559 transfer. `AMOUNT` is in tXDS (`1.5`), or with a `wei` / `gwei` suffix. Optional `--gas-limit`, `--max-fee`, `--priority-fee` (gwei), `--nonce`, `--wait`, `--rpc`. |
| `balance ADDR [--block TAG] [--wei] [--rpc URL]` | Prints a balance in tXDS. |

Keystore passwords come from `--password`, `--password-file`, the `XDEVS_PASSWORD` environment variable, or an interactive prompt (in that order). Every command defaults to `--rpc http://127.0.0.1:8545`.

## 6. Deploy a contract with Foundry

Foundry works as on any EVM chain:

```bash
forge init counter && cd counter

export RPC=http://127.0.0.1:8545
export PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80   # dev account 0

# Deploy (recent Foundry versions need --broadcast to actually send)
forge create src/Counter.sol:Counter --rpc-url $RPC --private-key $PK --broadcast
# -> Deployed to: 0x...

export COUNTER=0x...   # the address printed above
cast send $COUNTER "increment()" --rpc-url $RPC --private-key $PK
cast call $COUNTER "number()(uint256)" --rpc-url $RPC          # 1
```

More tools (Hardhat, viem, `forge script`) are covered in [Deploying contracts](/developers/deploy-contracts).

## Current limits to know about

- **Filter methods** (`eth_newFilter`, `eth_getFilterChanges`, ...) are not implemented. Use `eth_getLogs`, or `eth_subscribe` over WebSocket (`ws://127.0.0.1:8545`).
- **State history:** state is readable for the last 10,000 blocks by default; older blocks return `historical state not available`.
- **No pending block:** the `pending` tag means `latest`, except that `eth_getTransactionCount` with `pending` counts pool transactions.
- **Blob (type 3) transactions** are rejected.
- **The system contracts are not in the dev genesis yet**, so calls to `0x…1000`–`0x…1009` hit empty accounts on the dev chain. See [Predeployed contracts](/concepts/predeploys).

The full list is in the [JSON-RPC reference](/developers/json-rpc#not-implemented-yet).

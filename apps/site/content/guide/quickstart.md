# Quickstart: use the Enclave testnet

This guide uses **Enclave**, the live public testnet of xDEVS Chain: add it to MetaMask, get free tXDS from the faucet, deploy a contract with Foundry, look it up in the explorer and try the passkey wallet. It takes about 10 minutes and needs no node of your own.

:::warning[Testnet]
Enclave (chain id **717171**) is a test network. Its coin, tXDS, has **no value**, and the network may be reset with notice on the [status page](https://status.xdevschain.tech). All endpoints are listed in [Network endpoints](/network/endpoints).
:::

## 1. Add Enclave to MetaMask

In MetaMask: **Settings → Networks → Add a network → Add a network manually**, then enter:

| Field | Value |
|---|---|
| Network name | xDEVS Chain Enclave |
| RPC URL | `https://rpc.xdevschain.tech` |
| Chain ID | `717171` (`0xaf173`) |
| Currency symbol | `tXDS` (EnclaveXDS, 18 decimals) |
| Block explorer URL | `https://explorer.xdevschain.tech` |

The [faucet](https://faucet.xdevschain.tech) and the [status page](https://status.xdevschain.tech) can also add the network with one click.

Check that the RPC answers:

```bash
cast chain-id --rpc-url https://rpc.xdevschain.tech       # 717171
cast block-number --rpc-url https://rpc.xdevschain.tech   # grows every 2 s
```

## 2. Get tXDS from the faucet

Open [faucet.xdevschain.tech](https://faucet.xdevschain.tech), paste your address, solve the captcha (Cloudflare Turnstile) and claim. Each claim pays **100 tXDS**, once every 24 hours per address and per IP.

For the Foundry steps below, use a fresh throwaway key rather than your everyday one:

```bash
cast wallet new          # prints a new address and private key: testnet use only
```

Claim tXDS for that address, then check the balance:

```bash
cast balance <your address> --ether --rpc-url https://rpc.xdevschain.tech
```

## 3. Deploy a contract with Foundry

Foundry works as on any EVM chain. Every committed block is final, so a receipt you get back will never be reorged away.

```bash
forge init counter && cd counter

export RPC=https://rpc.xdevschain.tech
export PK=0x...          # the throwaway key from step 2

# Deploy (recent Foundry versions need --broadcast to actually send)
forge create src/Counter.sol:Counter --rpc-url $RPC --chain 717171 --private-key $PK --broadcast
# -> Deployed to: 0x...

export COUNTER=0x...     # the address printed above
cast send $COUNTER "increment()" --rpc-url $RPC --chain 717171 --private-key $PK
cast call $COUNTER "number()(uint256)" --rpc-url $RPC   # 1
```

To keep keys off the command line, import them into an encrypted Foundry keystore (`cast wallet import enclave --interactive`) and pass `--account enclave` instead of `--private-key`. Hardhat, viem and `forge script` are covered in [Deploying contracts](/developers/deploy-contracts).

## 4. Find it in the explorer

Open `https://explorer.xdevschain.tech/address/<COUNTER>` to see the contract and its transactions, or paste a transaction hash into the search box of the [explorer](https://explorer.xdevschain.tech). The explorer also shows blocks, tokens, `.xdevs` names and the validator set.

## 5. Try the passkey wallet

Open [wallet.xdevschain.tech](https://wallet.xdevschain.tech) and create an account with a **passkey** (Face ID, fingerprint or a security key). There is no seed phrase: the account is an xDEVS [smart account](/concepts/smart-accounts), and its activation is sponsored, so a brand-new account needs no tXDS to get started. From the wallet you can:

- send tXDS to an address or a `.xdevs` [name](/concepts/xns), with a simulation preview before you sign;
- add guardians and turn on the [Guardian Vault](/concepts/guardian-vault);
- review and revoke [approvals](/concepts/approvals);
- for a MetaMask address delegated with EIP-7702, turn on the [EOA lock](/concepts/eoa-lock).

Warnings in the preview are advice, not fact: read them, but they never prove that a transaction is safe or unsafe.

## What works differently

- **Filter methods** (`eth_newFilter`, `eth_getFilterChanges`, ...) are not implemented. Use `eth_getLogs`, or `eth_subscribe` over WebSocket at `wss://ws.xdevschain.tech`.
- **State history:** the public RPC answers state queries for the last 10,000 blocks; older blocks return `historical state not available`.
- **No pending block:** the `pending` tag means `latest`, except that `eth_getTransactionCount` with `pending` counts pool transactions.
- **Blob (type 3) transactions** are rejected.
- **Finality:** `safe` and `finalized` equal `latest`.

The full list is in the [JSON-RPC reference](/developers/json-rpc#not-implemented-yet).

## Develop offline: a local dev chain

To work without the network, run a **single-node dev chain** on your machine. It uses chain id **7117**, which is for local development only. To run several validators with real BFT consensus instead, see [Local multi-validator network](/operators/local-network).

### Prerequisites

- **Rust** stable. The exact version is pinned in [`rust-toolchain.toml`](https://github.com/JACELL100/xdevs-chain/blob/main/rust-toolchain.toml), so `rustup` picks it up automatically.
- A **C toolchain** for your platform. On Windows, use the `x86_64-pc-windows-gnu` toolchain with MinGW gcc.

### Build and run the node

```bash
git clone https://github.com/JACELL100/xdevs-chain.git
cd xdevs-chain
cargo build

# A single-node dev chain: chain id 7117, JSON-RPC on 127.0.0.1:8545, a block every 2 s
./target/debug/xdevs run --dev --block-time 2s
```

On Windows the binaries are `target\debug\xdevs.exe` and `target\debug\xdevs-cli.exe`. Prebuilt Linux binaries are on [GitHub Releases](https://github.com/JACELL100/xdevs-chain/releases).

The node stores its database in `./.xdevs/data` and resumes from the same head after a restart. To start over from block 0, stop the node and delete the `.xdevs` folder.

You can also write the genesis and config files into a node home first, edit them, and run from there:

```bash
./target/debug/xdevs init --dev --data-dir ./.xdevs   # writes genesis.json + config.toml
./target/debug/xdevs run --data-dir ./.xdevs
```

See [Run a node](/operators/run-a-node) for every flag and [Configuration reference](/operators/configuration) for `config.toml`.

### Dev accounts and funds

The dev genesis pre-funds the 10 well-known Anvil/Hardhat test accounts (mnemonic `test test test test test test test test test test test junk`) with **10,000 tXDS each**. The first two:

| | Address | Private key |
|---|---|---|
| Account 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| Account 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |

:::danger[These keys are public]
Everyone knows these private keys. Use them **only** on a local dev chain, and never send real funds, or Enclave tXDS, to these addresses.
:::

To use the dev chain in MetaMask, add a network with RPC URL `http://127.0.0.1:8545`, chain ID `7117` and symbol `tXDS` (no explorer), then import a dev key or send tXDS to your own address. If you wipe `.xdevs` and restart from block 0, use **Settings → Advanced → Clear activity tab data** so MetaMask forgets the old nonces.

The Foundry commands from step 3 work unchanged with `RPC=http://127.0.0.1:8545`, `--chain 7117` and dev account 0's key.

### Send tXDS with `xdevs-cli`

```bash
# Balance of a dev account
./target/debug/xdevs-cli balance 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

# Send 1.5 tXDS from dev account 0 to another address and wait for the receipt
./target/debug/xdevs-cli send \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --to 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  --value 1.5 --wait
```

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

Keystore passwords come from `--password`, `--password-file`, the `XDEVS_PASSWORD` environment variable, or an interactive prompt (in that order). Every command defaults to `--rpc http://127.0.0.1:8545`; pass `--rpc https://rpc.xdevschain.tech` to use Enclave.

:::info[System contracts on the dev chain]
The built-in dev genesis does not contain the system contracts, so on chain id 7117 the addresses `0x…1000`–`0x…1009` are empty accounts. Enclave has them from genesis. To get them locally, start the dev node with [`deploy/configs/dev-genesis-with-predeploys.json`](https://github.com/JACELL100/xdevs-chain/blob/main/deploy/configs/dev-genesis-with-predeploys.json) (`--genesis`). See [Predeployed contracts](/concepts/predeploys).
:::

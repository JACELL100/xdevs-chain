# Deploying contracts

xDEVS Chain is EVM-equivalent: it runs `revm` with Ethereum's **Osaka** rules, computes transaction hashes, receipts and state roots the Ethereum way, and accepts legacy, EIP-2930, EIP-1559 and EIP-7702 transactions. Your existing Solidity code and tooling work unchanged.

The examples below target **Enclave**, the live public testnet. Get tXDS for your deployer address from the [faucet](https://faucet.xdevschain.tech) first (100 tXDS per claim, every 24 h), and use a throwaway key: `cast wallet new` makes one.

| | Enclave testnet | Local dev chain |
|---|---|---|
| RPC URL | `https://rpc.xdevschain.tech` | `http://127.0.0.1:8545` |
| WebSocket | `wss://ws.xdevschain.tech` | `ws://127.0.0.1:8545` |
| Chain id | `717171` | `7117` |
| Explorer | [explorer.xdevschain.tech](https://explorer.xdevschain.tech) | none |
| Funds | [Faucet](https://faucet.xdevschain.tech) | Dev account 0, key `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` (public!) |

To develop offline, start a [local dev chain](/guide/quickstart#develop-offline-a-local-dev-chain) and swap in the right-hand column.

## Foundry

```bash
export RPC=https://rpc.xdevschain.tech
export PK=0x...   # a throwaway key funded from the faucet

# Deploy (recent Foundry versions are dry-run without --broadcast)
forge create src/Counter.sol:Counter --rpc-url $RPC --chain 717171 --private-key $PK --broadcast

# Interact
cast send <address> "increment()" --rpc-url $RPC --chain 717171 --private-key $PK
cast call <address> "number()(uint256)" --rpc-url $RPC

# Scripts
forge script script/Counter.s.sol --rpc-url $RPC --chain 717171 --private-key $PK --broadcast
```

Prefer an encrypted keystore over raw keys: `cast wallet import enclave --interactive`, then `--account enclave` instead of `--private-key`.

Useful `cast` commands:

```bash
cast chain-id --rpc-url $RPC                       # 717171
cast balance <address> --ether --rpc-url $RPC
cast receipt <tx-hash> --rpc-url $RPC
cast call <address> "number()(uint256)" --block 100 --rpc-url $RPC   # state at a past block (last 10,000 blocks)
```

Look up what you deployed at `https://explorer.xdevschain.tech/address/<address>` or `/tx/<hash>`. To fork Enclave into Anvil: `anvil --fork-url https://rpc.xdevschain.tech` (state of the last 10,000 blocks only).

## Hardhat

```ts
// hardhat.config.ts
import type { HardhatUserConfig } from 'hardhat/config'

const config: HardhatUserConfig = {
  solidity: '0.8.28',
  networks: {
    enclave: {
      url: 'https://rpc.xdevschain.tech',
      chainId: 717171,
      accounts: [process.env.ENCLAVE_PK!], // a throwaway key funded from the faucet
    },
    xdevsLocal: {
      url: 'http://127.0.0.1:8545',
      chainId: 7117,
      // Dev account 0. Public key material: local use only.
      accounts: ['0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'],
    },
  },
}
export default config
```

Then deploy with your usual script or Ignition module, passing `--network enclave`.

## viem

```ts
import { createWalletClient, createPublicClient, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { xdevsEnclave } from '@xdevs/sdk'   // or xdevsLocal for chain id 7117

const account = privateKeyToAccount(process.env.ENCLAVE_PK as `0x${string}`)
const wallet = createWalletClient({ account, chain: xdevsEnclave, transport: http() })
const client = createPublicClient({ chain: xdevsEnclave, transport: http() })

const hash = await wallet.deployContract({ abi, bytecode })
const receipt = await client.waitForTransactionReceipt({ hash })
console.log(receipt.contractAddress)
```

`xdevsEnclave` already carries the RPC, WebSocket and explorer URLs. See [TypeScript SDK](/developers/sdk) for how to install `@xdevs/sdk` before it is published.

## Account abstraction (ERC-4337)

Enclave has the canonical **EntryPoint v0.9** at `0x433709009B8330FDa32311DF1C2AFA402eD8D009` (in genesis) and a public **bundler** at `https://bundler.xdevschain.tech` (standard `eth_sendUserOperation`, `eth_estimateUserOperationGas`, `eth_getUserOperationReceipt`, ...). It is the bundler behind the [wallet](https://wallet.xdevschain.tech)'s passkey accounts and their sponsored activation. Smart accounts come from the factory at `0x…1006`; see [Smart accounts & passkeys](/concepts/smart-accounts).

## Things that differ from Ethereum (today)

- **No filter methods** (`eth_newFilter`, `eth_getFilterChanges`, ...). Use `eth_getLogs`, or `eth_subscribe` (`newHeads`, `logs`, `newPendingTransactions`) over WebSocket (`wss://ws.xdevschain.tech` on Enclave). viem and ethers poll automatically over HTTP.
- **No `debug_` / `trace_` methods.** Foundry's `cast run` and trace-based debugging against the live chain don't work yet; debug locally with `forge test`.
- **No blob transactions** (type 3).
- **BFT finality semantics:** every committed block is final, so `safe` and `finalized` equal `latest`, and logs are never `removed`.
- **System contracts:** on Enclave, `0x…1000`–`0x…1009`, Multicall3 and the EntryPoint are in genesis. The built-in dev genesis (chain id 7117) does not contain them; start the dev node with `--genesis deploy/configs/dev-genesis-with-predeploys.json`, or test against them in Foundry by etching the runtime code, as the [contracts test suite](https://github.com/JACELL100/xdevs-chain/tree/main/contracts) does.
- **Osaka precompiles** are available, including `P256VERIFY` at `0x100` for passkey signatures. Compile with `evm_version = "osaka"` (or `prague`) as needed.

## Building on the safety features

- Prefer [expiring approvals](/concepts/approvals): read `allowance()` before every pull and handle `AllowanceExpired` by asking again.
- Support ERC-1271 signatures from [smart accounts](/concepts/smart-accounts), and expect that a vault-protected account may queue your approval instead of executing it instantly.
- Resolve [`.xdevs` names](/concepts/xns) through the registry at `0x…1004` and show `primaryName`, not raw reverse records.

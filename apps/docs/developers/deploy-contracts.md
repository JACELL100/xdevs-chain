# Deploying contracts

xDEVS Chain is EVM-equivalent: it runs `revm` with Ethereum's **Osaka** rules, computes transaction hashes, receipts and state roots the Ethereum way, and accepts legacy, EIP-2930, EIP-1559 and EIP-7702 transactions. Your existing Solidity code and tooling work unchanged.

The examples below target the [local dev chain](/guide/quickstart). Use the public RPC once the testnet is live (see [Network endpoints](/network/endpoints)).

| | Local dev chain |
|---|---|
| RPC URL | `http://127.0.0.1:8545` |
| Chain id | `7117` |
| Funded key (public!) | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |

## Foundry

```bash
export RPC=http://127.0.0.1:8545
export PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Deploy (recent Foundry versions are dry-run without --broadcast)
forge create src/Counter.sol:Counter --rpc-url $RPC --private-key $PK --broadcast

# Interact
cast send <address> "increment()" --rpc-url $RPC --private-key $PK
cast call <address> "number()(uint256)" --rpc-url $RPC

# Scripts
forge script script/Counter.s.sol --rpc-url $RPC --private-key $PK --broadcast
```

Useful `cast` commands:

```bash
cast chain-id --rpc-url $RPC
cast balance 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 --ether --rpc-url $RPC
cast receipt <tx-hash> --rpc-url $RPC
cast call <address> "number()(uint256)" --block 100 --rpc-url $RPC   # state at a past block
```

To fork the dev chain into Anvil: `anvil --fork-url http://127.0.0.1:8545`.

## Hardhat

```ts
// hardhat.config.ts
import type { HardhatUserConfig } from 'hardhat/config'

const config: HardhatUserConfig = {
  solidity: '0.8.28',
  networks: {
    xdevsDev: {
      url: 'http://127.0.0.1:8545',
      chainId: 7117,
      // Dev account 0. Public key material: local use only.
      accounts: ['0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'],
    },
  },
}
export default config
```

Then deploy with your usual script or Ignition module, passing `--network xdevsDev`.

## viem

```ts
import { createWalletClient, createPublicClient, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { xdevsDevnet } from '@xdevs/sdk'   // or define chain id 7117 yourself

const account = privateKeyToAccount('0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80')
const wallet = createWalletClient({ account, chain: xdevsDevnet, transport: http() })
const client = createPublicClient({ chain: xdevsDevnet, transport: http() })

const hash = await wallet.deployContract({ abi, bytecode })
const receipt = await client.waitForTransactionReceipt({ hash })
console.log(receipt.contractAddress)
```

See [TypeScript SDK](/developers/sdk) for how to install `@xdevs/sdk` before it is published.

## Things that differ from Ethereum (today)

- **No filter methods** (`eth_newFilter`, `eth_getFilterChanges`, ...). Use `eth_getLogs`, or `eth_subscribe` (`newHeads`, `logs`, `newPendingTransactions`) over WebSocket on the RPC port. viem and ethers poll automatically over HTTP.
- **No `debug_` / `trace_` methods.** Foundry's `cast run` and trace-based debugging against the live chain don't work yet; debug locally with `forge test`.
- **No blob transactions** (type 3).
- **BFT finality semantics:** every committed block is final, so `safe` and `finalized` equal `latest`, and logs are never `removed`.
- **System contracts are not on the dev chain yet.** Don't hard-code calls to `0x…1000`–`0x…1009` and expect code there on chain id 7117. Test against them in Foundry by etching the runtime code, as the [contracts test suite](https://github.com/JACELL100/xdevs-chain/tree/main/contracts) does.
- **Osaka precompiles** are available, including `P256VERIFY` at `0x100` for passkey signatures. Compile with `evm_version = "osaka"` (or `prague`) as needed.

## Building on the safety features

- Prefer [expiring approvals](/concepts/approvals): read `allowance()` before every pull and handle `AllowanceExpired` by asking again.
- Support ERC-1271 signatures from [smart accounts](/concepts/smart-accounts), and expect that a vault-protected account may queue your approval instead of executing it instantly.
- Resolve [`.xdevs` names](/concepts/xns) through the registry at `0x…1004` and show `primaryName`, not raw reverse records.

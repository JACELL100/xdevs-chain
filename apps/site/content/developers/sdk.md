# TypeScript SDK (`@xdevs/sdk`)

`@xdevs/sdk` is a small TypeScript library on top of [viem](https://viem.sh): chain definitions, typed `xdevs_` RPC actions, XNS name helpers and address-poisoning helpers.

<span class="status ok">Built and tested</span> · Version 0.1.0, pre-release · ESM only · Node 22.18+ or any modern bundler · `viem@^2.21` peer dependency

:::warning[Not on npm yet]
The package is not published yet. Build it from the repository:

```bash
git clone https://github.com/JACELL100/xdevs-chain.git
cd xdevs-chain/sdk/ts
npm ci && npm run build          # emits dist/
npm pack                         # creates xdevs-sdk-0.1.0.tgz

# in your app
npm install viem /path/to/xdevs-sdk-0.1.0.tgz
```

Once published, it will be `npm install @xdevs/sdk viem`.
:::

## Connect

```ts
import { createPublicClient, http } from 'viem'
import { xdevsActions, xdevsEnclave, formatXds } from '@xdevs/sdk'

// Enclave, the public testnet: https://rpc.xdevschain.tech, chain id 717171
const client = createPublicClient({ chain: xdevsEnclave, transport: http() }).extend(xdevsActions)

const balance = await client.getBalance({ address: '0x...' })
console.log(`${formatXds(balance)} tXDS`)
```

## Chains

| Export | Chain id | RPC | Notes |
|---|---|---|---|
| `xdevsEnclave` | 717171 | `https://rpc.xdevschain.tech` (WS `wss://ws.xdevschain.tech`) | **Enclave public testnet**, <span class="status ok">Live</span>. Carries the explorer URL and explorer API. `xdevsTestnet` is a deprecated alias |
| `xdevsLocal` | 7117 | `http://127.0.0.1:8545` (WS on the same port) | Local dev chain (`xdevs run --dev`) |
| `xdevsDevnet` | 717170 | none | Internal devnet; point it at a node with `defineXdevsDevnet({ rpcUrl })` |

`XDEVS_ENCLAVE_ENDPOINTS` exports every Enclave URL (RPC, WebSocket, explorer, explorer API, wallet, faucet, status, bundler).

All chains carry the genesis predeploys as viem `contracts` (`multicall3`, `staking`, `xnsRegistry`, `wxds`, ...), and `predeploys` exports the raw addresses. They are in the Enclave genesis; the built-in local dev genesis doesn't contain them (see [Predeployed contracts](/concepts/predeploys)).

## Add the network to a wallet

```ts
import { createWalletClient, custom } from 'viem'
import { addXdevsChainToWallet, xdevsEnclave } from '@xdevs/sdk'

const wallet = createWalletClient({ transport: custom(window.ethereum!) })
await addXdevsChainToWallet(wallet, xdevsEnclave) // sends wallet_addEthereumChain
```

`addXdevsChainToWallet` refuses chains without an RPC URL (such as the unconfigured `xdevsDevnet`).

## `xdevs_` actions

| Action | RPC method | On Enclave |
|---|---|---|
| `getCommit({ height })` | `xdevs_getCommitCertificate` | Live |
| `getValidators()` | `xdevs_validators` | <span class="status planned">Planned</span> (not served yet; read the Staking contract or the [explorer](https://explorer.xdevschain.tech/validators)) |
| `getEpochInfo()` | `xdevs_epochInfo` | <span class="status planned">Planned</span> (not served yet) |
| `simulateTransaction(tx)` | `xdevs_simulateTransaction` | Live |
| `checkRecipient({ from?, to })` (`to`: address or XNS name) | `xdevs_checkRecipient` | Live |
| `getApprovals({ owner, tokens?, fromBlock? })` | `xdevs_getApprovals` | Live |
| `getVaultQueue({ vault })` | `xdevs_getVaultQueue` | Live |
| `watchVaultEvents({ account, onEvent })` (WebSocket transport) | `eth_subscribe("vaultEvents")` | Live |
| `resolveXnsName({ name })` | `xdevs_resolveName` | Live |
| `reverseXnsName({ address })` | `xdevs_reverseName` | Live |
| `getLocalFee({ contract })` | `xdevs_localFee` | Live on Enclave; reports a local fee once local fee markets activate at block 97,200 |

Against a node that does not serve a method yet, the SDK throws `XdevsMethodNotAvailableError` (the node answers "method not found"):

```ts
import { XdevsMethodNotAvailableError } from '@xdevs/sdk'

try {
  await client.getValidators()
} catch (e) {
  if (e instanceof XdevsMethodNotAvailableError) console.log(`needs a Phase ${e.phase}+ node`)
}
```

`xdevs_getEoaLock`, which the node implements, is not wrapped by the SDK yet; call it with `client.request`. Safety results are advice, not fact: every simulation and recipient check carries a `disclaimer`.

Every action is also exported as a standalone function taking the client first (`getValidators(client)`), for tree-shaking.

### Simulation preview (Phase 4)

```ts
import { parseXds } from '@xdevs/sdk'

const sim = await client.simulateTransaction({ account: me, to: shop, value: parseXds('12.5') })
for (const w of sim.warnings) console.warn(`[${w.severity}] ${w.code}: ${w.message}`)
console.log(sim.disclaimer) // warnings are advice, not facts
```

## Offline helpers (work today)

| Function | What it does |
|---|---|
| `parseXds(s)` / `formatXds(dev)` | Convert between tXDS strings and `bigint` dev units (10¹⁸) |
| `normalizeXnsName(input)` | Trim, lowercase, append `.xdevs`, validate; rejects non-ASCII and other TLDs |
| `checkXnsLabel`, `isValidXnsLabel`, `isValidXnsName` | Label rules: 3–63 of `[a-z0-9-]`, no edge or double hyphen |
| `xnsNamehash(name)`, `xnsLabelhash(label)` | EIP-137 namehash / labelhash |
| `xnsSkeleton(label)`, `areConfusableXnsLabels(a, b)` | Same confusable mapping as the registry contract |
| `isLookAlikeAddress(a, b, n = 4)` | Different addresses with equal first and last `n` hex characters |
| `findLookAlikeAddresses(target, known, n)` | Filter a recipient history for look-alikes |
| `getAddChainParameters(chain)` | EIP-3085 parameters for `wallet_addEthereumChain` |

Full API reference: [`docs/specs/sdk.md`](https://github.com/JACELL100/xdevs-chain/blob/main/docs/specs/sdk.md) · Source: [`sdk/ts`](https://github.com/JACELL100/xdevs-chain/tree/main/sdk/ts).

# TypeScript SDK (`@xdevs/sdk`)

`@xdevs/sdk` is a small TypeScript library on top of [viem](https://viem.sh): chain definitions, typed `xdevs_` RPC actions, XNS name helpers and address-poisoning helpers.

<span class="status ok">Built and tested</span> · Version 0.1.0, pre-release · ESM only · Node 22.18+ or any modern bundler · `viem@^2.21` peer dependency

::: warning Not on npm yet
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
import { xdevsActions, xdevsDevnet, formatXds } from '@xdevs/sdk'

const client = createPublicClient({ chain: xdevsDevnet, transport: http() }).extend(xdevsActions)

const balance = await client.getBalance({ address: '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266' })
console.log(`${formatXds(balance)} tXDS`)
```

## Chains

| Export | Chain id | RPC | Notes |
|---|---|---|---|
| `xdevsDevnet` | 7117 | `http://127.0.0.1:8545` | Local dev chain |
| `xdevsTestnet` | 717171 | `https://rpc.testnet.xdevs.invalid` | **Placeholder** id and URL, not a real network |

Both chains carry the genesis predeploys as viem `contracts` (`multicall3`, `staking`, `xnsRegistry`, `wxds`, ...), and `predeploys` exports the raw addresses. Remember that the dev genesis doesn't contain these contracts yet (see [Predeployed contracts](/concepts/predeploys)).

::: tip WebSocket URL
`xdevsDevnet` uses `ws://127.0.0.1:8545`: a default dev node serves WebSocket on the same port as HTTP.
:::

Until the public testnet exists, override its URLs if you run your own network:

```ts
import { defineChain } from 'viem'
import { xdevsTestnet } from '@xdevs/sdk'

const myNet = defineChain({ ...xdevsTestnet, rpcUrls: { default: { http: ['https://rpc.example.org'] } } })
```

## Add the network to a wallet

```ts
import { createWalletClient, custom } from 'viem'
import { addXdevsChainToWallet, xdevsDevnet } from '@xdevs/sdk'

const wallet = createWalletClient({ transport: custom(window.ethereum!) })
await addXdevsChainToWallet(wallet, xdevsDevnet) // sends wallet_addEthereumChain
```

`addXdevsChainToWallet` refuses chains that still use a placeholder `.invalid` URL.

## `xdevs_` actions

| Action | RPC method | Node support |
|---|---|---|
| `getCommit({ height })` | `xdevs_getCommitCertificate` | Implemented (Phase 2) |
| `getValidators()` | `xdevs_validators` | Phase 3 |
| `getEpochInfo()` | `xdevs_epochInfo` | Phase 3 |
| `simulateTransaction(tx)` | `xdevs_simulateTransaction` | Phase 4 |
| `checkRecipient({ from, to })` | `xdevs_checkRecipient` | Phase 4 |
| `getApprovals({ owner })` | `xdevs_getApprovals` | Phase 4 |
| `getVaultQueue({ vault })` | `xdevs_getVaultQueue` | Phase 4 |
| `resolveXnsName({ name })` | `xdevs_resolveName` | Phase 4 |
| `reverseXnsName({ address })` | `xdevs_reverseName` | Phase 4 |
| `getLocalFee({ contract })` | `xdevs_localFee` | Phase 5 |

None of these methods exist in the node yet. The SDK throws `XdevsMethodNotAvailableError` when the node answers "method not found":

```ts
import { XdevsMethodNotAvailableError } from '@xdevs/sdk'

try {
  await client.getValidators()
} catch (e) {
  if (e instanceof XdevsMethodNotAvailableError) console.log(`needs a Phase ${e.phase}+ node`)
}
```

`xdevs_getEoaLock`, which the node implements, is not wrapped by the SDK yet; call it with `client.request`. `xdevs_subscribe("vaultEvents")` is not wrapped either.

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

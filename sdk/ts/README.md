# @xdevs/sdk

TypeScript SDK for **xDEVS Chain** (by THE xDEVS) — an EVM-equivalent PoS L1 focused on user safety.
Built on [viem](https://viem.sh): chain definitions, typed `xdevs_` JSON-RPC actions, XNS (`.xdevs`)
name helpers and address-poisoning helpers.

> Status: pre-release (0.1.0). The public testnet chain id (717171) and URLs are **placeholders**.
> Most `xdevs_` methods are implemented by the node only from Phase 3–5 (see table below); the SDK
> throws `XdevsMethodNotAvailableError` when the node does not have them yet.

## Install

```sh
npm install @xdevs/sdk viem
```

ESM only, Node ≥ 22.18 or any modern bundler. `viem@^2.21` is a peer dependency.

## Connect

```ts
import { createPublicClient, http } from 'viem'
import { xdevsActions, xdevsDevnet, formatXds } from '@xdevs/sdk'

const client = createPublicClient({ chain: xdevsDevnet, transport: http() }).extend(xdevsActions)

const balance = await client.getBalance({ address: '0x…' })
console.log(`${formatXds(balance)} tXDS`)
```

| Chain | Export | Chain id | RPC |
|---|---|---|---|
| Local devnet | `xdevsDevnet` | 7117 | `http://127.0.0.1:8545`, `ws://127.0.0.1:8545` |
| Public testnet | `xdevsTestnet` | 717171 (**placeholder, TODO**) | `https://rpc.testnet.xdevs.invalid` (**placeholder**) |

Until the testnet is live, override its URLs:

```ts
import { defineChain } from 'viem'
const testnet = defineChain({ ...xdevsTestnet, rpcUrls: { default: { http: ['https://rpc.example.org'] } } })
```

Both chains carry the genesis predeploys as viem `contracts` (`multicall3`, `staking`, `xnsRegistry`, `wxds`, …),
and `predeploys` exports the raw addresses (kept in sync with `contracts/src/common/Predeploys.sol` by a test).

## Safety features

```ts
import { parseXds } from '@xdevs/sdk'

// Simulate before signing (anti blind-signing, Phase 4)
const sim = await client.simulateTransaction({ account: me, to: shop, value: parseXds('12.5') })
for (const w of sim.warnings) console.warn(`[${w.severity}] ${w.code}: ${w.message}`)
console.log(sim.disclaimer) // warnings are advice, not facts

// Address-poisoning checks (Phase 4)
const check = await client.checkRecipient({ from: me, to: recipient })
if (check.lookAlikeOf.length) alert('This address looks like one you used before — double-check it!')

// Live approvals for a revoke screen, Guardian Vault queue (Phase 4)
const approvals = await client.getApprovals({ owner: me })
const queue = await client.getVaultQueue({ vault: me })
```

Offline helper (no RPC needed):

```ts
import { isLookAlikeAddress } from '@xdevs/sdk'
isLookAlikeAddress(
  '0x1234567890abcdef1234567890abcdef1234abcd',
  '0x1234ffffffffffffffffffffffffffffffffabcd',
) // same first/last 4 hex chars, different address → true
```

## XNS names (`.xdevs`)

```ts
import { normalizeXnsName, xnsNamehash, areConfusableXnsLabels } from '@xdevs/sdk'

normalizeXnsName(' Alice ')   // 'alice.xdevs'
normalizeXnsName('аlice')     // throws InvalidXnsNameError (non-ASCII, homograph protection)
xnsNamehash('alice')          // ENS namehash of 'alice.xdevs'
areConfusableXnsLabels('paypa1', 'paypal') // true

const addr = await client.resolveXnsName({ name: 'alice' })      // Phase 4
const name = await client.reverseXnsName({ address: addr! })      // Phase 4
```

Label rules mirror `XNSLabel.sol`: 3–63 chars of `[a-z0-9-]`, no leading/trailing hyphen, no `--`.

## Add the network to MetaMask

```ts
import { createWalletClient, custom } from 'viem'
import { addXdevsChainToWallet, xdevsDevnet } from '@xdevs/sdk'

const wallet = createWalletClient({ transport: custom(window.ethereum!) })
await addXdevsChainToWallet(wallet, xdevsDevnet) // wallet_addEthereumChain (plan §23.1)
```

`addXdevsChainToWallet` refuses chains that still use a placeholder `.invalid` URL.

## `xdevs_` methods

| Action | RPC method | Node phase |
|---|---|---|
| `getCommit({ height })` | `xdevs_getCommitCertificate` | 2 |
| `getValidators()` | `xdevs_validators` | 3 |
| `getEpochInfo()` | `xdevs_epochInfo` | 3 |
| `simulateTransaction(tx)` | `xdevs_simulateTransaction` | 4 |
| `checkRecipient({ from, to })` | `xdevs_checkRecipient` | 4 |
| `getApprovals({ owner })` | `xdevs_getApprovals` | 4 |
| `getVaultQueue({ vault })` | `xdevs_getVaultQueue` | 4 |
| `resolveXnsName({ name })` | `xdevs_resolveName` | 4 |
| `reverseXnsName({ address })` | `xdevs_reverseName` | 4 |
| `getLocalFee({ contract })` | `xdevs_localFee` | 5 |

Every action is also exported as a standalone function taking the client first
(`getValidators(client)`), for tree-shaking. If the node answers "method not found" (-32601 / -32004):

```ts
import { XdevsMethodNotAvailableError } from '@xdevs/sdk'
try { await client.getValidators() } catch (e) {
  if (e instanceof XdevsMethodNotAvailableError) console.log(`needs a Phase ${e.phase}+ node`)
}
```

`xdevs_subscribe("vaultEvents")` is not wrapped yet.

## Development

```sh
npm install
npm test            # node --test on the .ts sources (Node's built-in type stripping, no network)
npx tsc --noEmit    # type-check src + tests
npm run build       # emit dist/ (ESM + .d.ts)
```

API reference: [`docs/specs/sdk.md`](../../docs/specs/sdk.md). License: Apache-2.0.

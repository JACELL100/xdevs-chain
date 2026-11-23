# @xdevs/sdk

TypeScript SDK for **xDEVS Chain** (by THE xDEVS) — an EVM-equivalent PoS L1 focused on user safety.
Built on [viem](https://viem.sh): chain definitions, typed `xdevs_` JSON-RPC actions, XNS (`.xdevs`)
name helpers and address-poisoning helpers.

> Status: pre-release (0.1.0). The public testnet is **xDEVS Chain Enclave** (chain id 717171, coin EnclaveXDS / tXDS);
> its endpoints under xdevschain.tech are announced on https://docs.xdevschain.tech/network/endpoints as they go live.
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
import { xdevsActions, xdevsEnclave, formatXds } from '@xdevs/sdk'

const client = createPublicClient({ chain: xdevsEnclave, transport: http() }).extend(xdevsActions)

const balance = await client.getBalance({ address: '0x…' })
console.log(`${formatXds(balance)} tXDS`)
```

| Chain | Export | Chain id | Coin | RPC |
|---|---|---|---|---|
| Enclave (public testnet) | `xdevsEnclave` | 717171 (`0xaf173`) | EnclaveXDS (tXDS) | `https://rpc.xdevschain.tech`, `wss://ws.xdevschain.tech`; explorer `https://explorer.xdevschain.tech` |
| Devnet | `xdevsDevnet` | 717170 | tXDS | no public RPC yet: `defineXdevsDevnet({ rpcUrl, wsUrl?, explorerUrl? })` |
| Local dev chain (`xdevs run --dev`) | `xdevsLocal` | 7117 | tXDS | `http://127.0.0.1:8545`, `ws://127.0.0.1:8545` |

`xdevsTestnet` / `XDEVS_TESTNET_CHAIN_ID` remain as deprecated aliases of `xdevsEnclave` / `XDEVS_ENCLAVE_CHAIN_ID`.
`XDEVS_ENCLAVE_ENDPOINTS` lists every Enclave service URL (rpc, ws, explorer, explorerApi, wallet, faucet, status,
bundler, site). Before 0.1.0 final, `xdevsDevnet` meant the local 7117 chain: use `xdevsLocal` for that now.

To point a chain at your own node, override its URLs:

```ts
import { defineChain } from 'viem'
const mine = defineChain({ ...xdevsLocal, rpcUrls: { default: { http: ['http://10.0.0.5:8545'] } } })
```

All chains carry the genesis predeploys as viem `contracts` (`multicall3`, `staking`, `xnsRegistry`, `wxds`, …),
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
import { addXdevsChainToWallet, xdevsEnclave } from '@xdevs/sdk'

const wallet = createWalletClient({ transport: custom(window.ethereum!) })
await addXdevsChainToWallet(wallet, xdevsEnclave) // wallet_addEthereumChain (plan §23.1)
```

`addXdevsChainToWallet` refuses chains without an RPC URL (e.g. the unconfigured `xdevsDevnet`) or with a placeholder `.invalid` URL.

## `xdevs_` methods

| Action | RPC method | Node phase |
|---|---|---|
| `getCommit({ height })` | `xdevs_getCommitCertificate` | 2 |
| `getValidators()` | `xdevs_validators` | 3 |
| `getEpochInfo()` | `xdevs_epochInfo` | 3 |
| `simulateTransaction(tx)` | `xdevs_simulateTransaction` | 4 |
| `checkRecipient({ from?, to })` (`to`: address or XNS name) | `xdevs_checkRecipient` | 4 |
| `getApprovals({ owner, tokens?, fromBlock? })` | `xdevs_getApprovals` | 4 |
| `getVaultQueue({ vault })` | `xdevs_getVaultQueue` | 4 |
| `watchVaultEvents({ account, onEvent })` (WebSocket transport) | `eth_subscribe("vaultEvents")` | 4 |
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

Vault alerts need a WebSocket transport:

```ts
const ws = createPublicClient({ chain: xdevsEnclave, transport: webSocket() }).extend(xdevsActions)
const stop = await ws.watchVaultEvents({ account: me, onEvent: (e) => console.log(e.type, e.id) })
```

## Development

```sh
npm install
npm test            # node --test on the .ts sources (Node's built-in type stripping, no network)
npx tsc --noEmit    # type-check src + tests
npm run build       # emit dist/ (ESM + .d.ts)
```

API reference: [`docs/specs/sdk.md`](../../docs/specs/sdk.md). License: Apache-2.0.

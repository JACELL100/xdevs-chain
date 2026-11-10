# `@xdevs/sdk` — API reference (v0.1.0)

Package root: `sdk/ts/`. ESM, strict TypeScript, viem `^2.21` peer dependency. Plan references: §1.4, §16,
§18, §19.2, §23, §23.1.

## 1. Chains

| Export | id | Notes |
|---|---|---|
| `xdevsDevnet` | 7117 (`DEV_CHAIN_ID` in `xdevs-primitives`) | `http://127.0.0.1:8545`, `ws://127.0.0.1:8545` |
| `xdevsTestnet` | 717171 = `0xAF173` | **TODO placeholder** id (not registered on chainlist) and URLs under `testnet.xdevs.invalid` |

Both: `nativeCurrency = { name: 'Testnet XDS', symbol: 'tXDS', decimals: 18 }`, `blockTime: 2000`,
`testnet: true`, and `contracts` = `multicall3` (`0xcA11bde05977b3631167028862bE2a173976CA11`) plus every
genesis predeploy (`staking`, `governance`, `treasury`, `feeConfig`, `xnsRegistry`, `smartAccountImpl`,
`smartAccountFactory`, `paymaster`, `permit2Expiring`, `wxds`), all `blockCreated: 0`.
Also exported: `xdevsChains`, `XDEVS_DEVNET_CHAIN_ID`, `XDEVS_TESTNET_CHAIN_ID`, `XDEVS_TESTNET_IS_PLACEHOLDER`,
`XDEVS_TESTNET_PLACEHOLDER_DOMAIN`, `testnetXds`, type `XdevsChain`.

## 2. Predeploys

`predeploys` mirrors `contracts/src/common/Predeploys.sol` exactly (a unit test parses the Solidity file):
`SYSTEM_ADDRESS` (`0xffff…fffe`), `STAKING` (`0x…1000`) … `WXDS` (`0x…1009`), `P256_VERIFY` (`0x…0100`).
Plus `MULTICALL3_ADDRESS`.

## 3. `xdevs_` RPC actions

Use `createPublicClient({...}).extend(xdevsActions)` or the standalone `fn(client, args)` forms.
Block-scoped methods accept `{ blockNumber?: bigint } | { blockTag?: BlockTag }` (default `'latest'`).
Quantities are hex on the wire and `bigint` in parsed results.

| Action | Method | Params on the wire | Result (parsed) | Phase |
|---|---|---|---|---|
| `getCommit({height})` | `xdevs_getCommitCertificate` | `[heightHex]` | `{height: bigint, round: number, blockHash, signatures: {validator, signature}[], raw} \| null` | 2 |
| `getValidators()` | `xdevs_validators` | `[block]` | `{operator, consensusKey, status, votingPower: bigint, totalStake: bigint, commissionBps, uptimeBps}[]` | 3 |
| `getEpochInfo()` | `xdevs_epochInfo` | `[block]` | `{epoch, epochLength, startBlock, nextTransitionBlock: bigint, validatorSetHash}` | 3 |
| `simulateTransaction(tx)` | `xdevs_simulateTransaction` | `[RpcTransactionRequest, block]` | §16.4 shape, `gasUsed: bigint` | 4 |
| `checkRecipient({from,to})` | `xdevs_checkRecipient` | `[from, to]` | `{firstTime, lookAlikeOf: Address[], name: string\|null, isContract, warnings?}` | 4 |
| `getApprovals({owner})` | `xdevs_getApprovals` | `[owner, block]` | `{kind, token, symbol?, spender, amount, expires: number\|null}[]` | 4 |
| `getVaultQueue({vault})` | `xdevs_getVaultQueue` | `[vault, block]` | `{id, target, value, data, executeAfter: bigint}[]` | 4 |
| `resolveXnsName({name})` | `xdevs_resolveName` | `[normalizedName]` | `Address \| null` (checksummed) | 4 |
| `reverseXnsName({address})` | `xdevs_reverseName` | `[address]` | `string \| null` | 4 |
| `getLocalFee({contract})` | `xdevs_localFee` | `[contract, block]` | `{contract, globalBaseFee, localBaseFee, totalBaseFee: bigint}` | 5 |

`simulateTransaction` result (§16.4): `{ success, gasUsed, assetChanges: {token: 'native'|Address, symbol, from,
to, amount}[], approvals: {token, spender, amount ('unlimited' or decimal), expires}[], warnings: {code, severity,
message}[], disclaimer, revertReason? }`. Severity: `'info' | 'low' | 'medium' | 'high'`.

Warning codes (`XdevsWarningCode`, §16.4 catalogue v1, open-ended for forward compatibility):
`UNLIMITED_APPROVAL`, `SET_APPROVAL_FOR_ALL`, `OWNERSHIP_TRANSFER`, `NEW_CONTRACT`, `UNVERIFIED_CONTRACT`,
`FIRST_TIME_RECIPIENT`, `LOOKALIKE_RECIPIENT`, `SELF_DESTRUCT_PROXY`, `HIGH_VALUE_TRANSFER`,
`PERMIT_SIGNATURE`, `PERMIT2_SIGNATURE`.

> The wire shapes for everything except `xdevs_simulateTransaction` are **SDK proposals**: plan §19.2 names
> the methods but not their JSON. The node (`crates/rpc`) should implement exactly these shapes, or this
> table and `sdk/ts/src/types.ts` must be updated together. `XdevsRpcSchema` is exported for use with
> viem's `rpcSchema<XdevsRpcSchema>()`.

### Errors

- `XdevsMethodNotAvailableError { method, phase }` — the node answered JSON-RPC `-32601` (method not found) or
  `-32004` (not supported), or `-32600/-32603/-32000` with a "method … not found" message. Not retried.
  `XDEVS_RPC_METHODS` lists `{ phase, spec, description }` per method.
- `InvalidXnsNameError { input, reason }` — thrown locally, before any RPC call.
- `isMethodNotFoundError(err)` — the predicate used for the mapping.
- Other RPC errors propagate unchanged (viem error classes).

## 4. Helpers

| Function | Behaviour |
|---|---|
| `parseXds(s)` / `formatXds(dev)` | `parseEther` / `formatEther` (1 XDS = 10¹⁸ dev). `XDS_DECIMALS`, `DEV_PER_XDS` |
| `normalizeXnsName(input)` | trim + ASCII-lowercase, append `.xdevs` to a bare label, validate every label; rejects non-ASCII and other TLDs |
| `checkXnsLabel(label)` | `null` or `'too-short' \| 'too-long' \| 'forbidden-character' \| 'bad-hyphen'` (mirrors `XNSLabel.check`: 3–63 of `[a-z0-9-]`, no edge/double hyphen) |
| `isValidXnsLabel`, `isValidXnsName` | boolean forms |
| `xnsNamehash(name)` / `xnsLabelhash(label)` | EIP-137 namehash (viem) of the normalized name / keccak of a label |
| `xnsSkeleton(label)`, `areConfusableXnsLabels(a, b)` | mirror `XNSLabel.skeleton` (`0→o 1→l rn→m vv→w cl→d`) |
| `isLookAlikeAddress(a, b, n = 4)` | different addresses with equal first `n` and last `n` hex chars (case-insensitive), `n ∈ 1..20` |
| `findLookAlikeAddresses(target, known, n)` | filter a recipient history |
| `getAddChainParameters(chain, {iconUrls?})` | EIP-3085 params per §23.1 (`chainId` lowercase hex, e.g. `0xaf173`) |
| `addXdevsChainToWallet(walletClient, chain, opts?)` | sends `wallet_addEthereumChain`; refuses placeholder `.invalid` URLs |

## 5. Not yet covered

- `xdevs_subscribe("vaultEvents")` (Phase 4, WebSocket) — types/action not written yet.
- Contract ABIs / typed wrappers for predeploys (Staking, XNS, Vault) — to add once the Solidity interfaces settle.

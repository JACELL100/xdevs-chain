# XNS and expiring approvals: periphery safety contracts

Status: testnet reference implementation. Plan refs: §1.4 (naming), §16.3 (XNS and address poisoning), §16.5 (expiring approvals), §18 (predeploys), §22.3 (canonical tokens).

Sources: `contracts/src/periphery/`. Tests: `contracts/test/periphery/`. Build and test with:

```
cd contracts && FOUNDRY_PROFILE=periphery forge test
```

## 1. Contracts and addresses

| Contract | Address | Kind | Init |
|---|---|---|---|
| `XNSRegistry` (registry, `.xdevs` registrar, public resolver and reverse registrar in one contract) | `0x0000…1004` (`Predeploys.XNS_REGISTRY`) | genesis predeploy | `initialize(...)` once, from `SYSTEM_ADDRESS` |
| `Permit2Expiring` | `0x0000…1008` (`Predeploys.PERMIT2_EXPIRING`) | genesis predeploy | none (no constructor, no init, no immutables) |
| `XDRC20` | any | normal deployment (reference token) | constructor |
| `XDRC20Bridged` | CREATE2 from the factory | deployed only by `CanonicalTokenFactory` | constructor |
| `CanonicalTokenFactory` | any | normal deployment (owned by governance) | constructor |

Runtime sizes: XNSRegistry 15.7 KB, Permit2Expiring 6.5 KB, XDRC20 6.0 KB. All are below EIP-170.

### 1.1 Genesis instructions for the Rust builder

1. Code. Put `deployedBytecode.object` from `contracts/out/periphery/XNSRegistry.sol/XNSRegistry.json` at `0x…1004`, and `Permit2Expiring.json` at `0x…1008`. You can also get the code from `forge inspect <C> deployedBytecode`. Use this runtime code as it is. Do not take code from a simulated CREATE: that would bake the simulating deployer into an immutable.
2. Initialize XNS. Run one call from `Predeploys.SYSTEM_ADDRESS` to `0x…1004` during genesis (or as the first system call of block 1). The canonical calldata is printed by:
   ```
   FOUNDRY_PROFILE=periphery forge script script/periphery/DeployPeriphery.s.sol --sig "genesisInitCalldata()"
   ```
   That is `initialize(GOVERNANCE=0x…1001, TREASURY=0x…1002, feePerYear=0, reserved=["xdevs","the-xdevs","official","support","admin","faucet"])`.
3. Permit2Expiring needs nothing more. Its EIP-712 domain separator is computed on every call from `block.chainid` and `address(this)`.

The initializer guard is in `GenesisInitializable`. `initialize` can run once, and only when `msg.sender` is `SYSTEM_ADDRESS` or the address that ran the constructor (an immutable). In genesis-placed code that immutable slot is zero-filled, and this is checked: the artifact has exactly one immutable reference and it is zero. So only `SYSTEM_ADDRESS` can initialize, and nobody can front-run the call. Before initialization, `register` reverts with `NotInitialized()`.

## 2. XNS: xDEVS Name Service (`alice.xdevs`)

### 2.1 ENS compatibility

The registry implements the EIP-137 ENS registry interface exactly: `owner`, `resolver`, `ttl`, `recordExists`, `setOwner`, `setResolver`, `setTTL`, `setRecord`, `setSubnodeOwner`, `setSubnodeRecord`, `setApprovalForAll`, `isApprovedForAll`. It also emits the ENS events `NewOwner`, `Transfer`, `NewResolver`, `NewTTL` and `ApprovalForAll`.

Nodes use the standard namehash. The tests check the vectors `namehash("eth") = 0x93cdeb70…c4ae`, `namehash("foo.eth") = 0xde9b09fd…f84f` and `namehash("addr.reverse") = 0x91d17777…a9e2`.

Every registered name gets `resolver(node) == 0x…1004`, the same contract. It implements these resolver profiles:

| Profile | Function | interfaceId |
|---|---|---|
| ENSIP-1 addr | `addr(bytes32) → address` | `0x3b3b57de` |
| ENSIP-9/11 multicoin | `addr(bytes32,uint256) → bytes` | `0xf1cb7e06` |
| ENSIP-5 text | `text(bytes32,string) → string` | `0x59d1d43c` |
| ENSIP-3 name (reverse) | `name(bytes32) → string` | `0x691f3431` |
| Versionable (`clearRecords`) | `recordVersions(bytes32)` | `0xd700ff33` |
| ERC-165 | `supportsInterface` | `0x01ffc9a7` |

The resolver setters are `setAddr(node, address)`, `setAddr(node, coinType, bytes)`, `setText`, `setName(node, string)` and `clearRecords(node)`. The caller must be the node owner or an approved operator, and the name must be live.

For coin type `60`, and for any ENSIP-11 EVM coin type (bit 31 set, for example `0x80000000 | chainId`), the address must be 0 or 20 bytes long.

Library adaptation:

- ethers v6: set the network's ENS registry (`ensAddress` / EnsPlugin) to `0x…1004`. Resolution then works: ethers calls `resolver(node)`, then `supportsInterface`, then `addr(node)`. Wildcard (ENSIP-10) is not advertised.
- viem: `getEnsAddress` and `getEnsName` go through ENS's UniversalResolver, which XNS does not include (see §6). You can call the registry and resolver directly with `readContract`, or use `resolveAddr(node)` and `primaryName(addr)`.

### 2.2 Registrar (`.xdevs`)

| Function | Notes |
|---|---|
| `register(string label, address owner, uint256 duration) payable → (node, expires)` | `duration` must be between 365 days and 10 years. `msg.value` must equal `rentPrice(duration)` exactly. The function sets the owner, sets the resolver to this contract, and sets `addr(node) = owner`. |
| `renew(string label, uint256 duration) payable` | Anyone can renew while the name is registered or in grace. The new expiry is `old + duration`, and it can be at most 10 years ahead of now. |
| `available(label)`, `valid(label)`, `rentPrice(duration)`, `nameExpires(uint256(labelhash))` | `nameExpires` is compatible with ENS BaseRegistrar. |
| `registerReserved(label, owner, duration)` | Governance only. It can take reserved labels and is free. |
| `setFeePerYear`, `setTreasury`, `setGovernance`, `setReserved(label,bool)` | Governance only. |

Lifecycle:

| Phase | Condition | Resolves? | Owner can manage? | Who can (re-)register? |
|---|---|---|---|---|
| active | `now < expires` | yes | yes | nobody |
| grace | `expires ≤ now < expires + 90 days` | no | no (anyone may renew) | nobody |
| released | `now ≥ expires + 90 days` | no | no | anyone |

- Fees are charged in native XDS/tXDS: `feePerYear * duration / 365 days`. The fee is forwarded to `treasury` (Treasury `0x…1002`) after all state changes. On testnet it is 0, and sending any value reverts.
- Expiry is safety-critical. When a name expires, the name and all of its subnames stop resolving immediately: `owner`, `resolver`, `ttl`, `addr`, `text` and `name` all return empty.
- On re-registration after grace, the generation number of the name is bumped. All old resolver records and all old subnames become unreachable. A new owner never inherits the previous owner's addresses.
- The root, `xdevs`, `reverse` and `addr.reverse` nodes are owned by the contract forever. Governance cannot take over names or the TLD.

### 2.3 Normalization rules (anti-scam)

A registrable label must:

1. be 3 to 63 bytes long;
2. contain only `a-z`, `0-9` and `-`;
3. have no leading or trailing `-`;
4. have no `--` anywhere (this also rules out punycode `xn--…`).

Invalid input is rejected, never normalized. Uppercase, whitespace, `.`, `_`, NUL, every non-ASCII byte (Cyrillic `а`, accented letters, zero-width joiners, emoji) and IDNA punycode all revert with `InvalidLabel(label, reason)`. Each label therefore has exactly one on-chain spelling, and Unicode homographs cannot be registered at all.

Confusable skeletons: every label is mapped to a skeleton, greedily from left to right:

| Look-alike | Maps to |
|---|---|
| `0` | `o` |
| `1` | `l` |
| `rn` | `m` |
| `vv` | `w` |
| `cl` | `d` |

These come from the UTS#39 confusables that are still possible in our alphabet. Only one active or in-grace name may hold a given skeleton. So while `paypal` is held, `paypa1` reverts with `ConfusableWithExisting`, and the same is true for `g00gle`/`google`, `rnetamask`/`metamask`, `vvallet`/`wallet` and `cloge`/`doge`. The first registrant wins. The skeleton becomes free again once the holder is past grace.

Reserved labels: governance keeps a list of labels that only governance can register. The genesis list is `xdevs`, `the-xdevs`, `official`, `support`, `admin`, `faucet`. Reserving a label has no effect on a name that is already registered.

Subname labels (`setSubnodeOwner` takes a raw labelhash) are not validated on-chain, the same as in ENS. Wallets should apply the same rules when they display subnames.

### 2.4 Reverse records and primary names

- `node(addr)` returns `namehash("<lowercase hex addr>.addr.reverse")`, which matches ENS.
- `setName(string name)` sets the caller's primary name. It only succeeds if `name` currently forward-resolves to the caller through `resolveAddr`, or if `name` is `""` to clear it. It also claims the caller's reverse node.
- `claim(owner)` hands control of the caller's reverse node to `owner`. This is useful for contracts.
- `primaryName(addr)` is the verified primary name. It re-checks the forward record at read time, so it returns `""` if the name has expired, has been repointed, or was written raw with `setName(node, …)`. Wallets and explorers should display `primaryName`, not the raw `name(node)`.
- `resolveAddr(node)` gives full client-style forward resolution. It reads the registry resolver (empty if expired) and then calls that resolver's `addr`. External resolvers are called with STATICCALL, and a revert or non-contract resolver gives `address(0)`.

### 2.5 Events for indexers (`xdevs_resolveName` / `xdevs_reverseName` / `xdevs_checkRecipient`)

| Event | Meaning |
|---|---|
| `NameRegistered(string label, bytes32 indexed labelhash, address indexed owner, uint256 expires, uint256 fee)` | New registration or re-registration. A re-registration drops all earlier records and subnames of this name. |
| `NameRenewed(string label, bytes32 indexed labelhash, uint256 expires, uint256 fee)` | The expiry moved. |
| `NewOwner`, `Transfer`, `NewResolver`, `NewTTL`, `ApprovalForAll` | ENS registry events. |
| `AddrChanged`, `AddressChanged`, `TextChanged`, `NameChanged`, `VersionChanged` | ENS resolver events. `VersionChanged` means every record of the node was cleared. |
| `ReverseClaimed(address indexed addr, bytes32 indexed node)` | Reverse node claimed. |
| `FeeChanged`, `TreasuryChanged`, `GovernanceChanged`, `ReservedChanged`, `Initialized` | Governance and setup. |

Indexers must apply expiry themselves (`expires` from the last `NameRegistered` or `NameRenewed`), or simply call the views. The views already return empty for expired names.

## 3. XDRC-20: reference token with expiring approvals

`XDRC20` is a fully compatible ERC-20 and EIP-2612 token, built on OpenZeppelin v5.7. It adds the following:

```solidity
function approve(address spender, uint256 value, uint64 expiry) external returns (bool);
function allowanceExpiry(address owner, address spender) external view returns (uint64);
function revokeAll() external;
function approvalEpoch(address owner) external view returns (uint32);
function permitWithExpiry(address owner, address spender, uint256 value, uint64 expiry,
                          uint256 deadline, bytes signature) external;   // ECDSA or ERC-1271
function defaultApprovalDuration() external view returns (uint64);
function maxApprovalDuration() external view returns (uint64);
function supportsInterface(bytes4) external view returns (bool);         // IXDRC20 + ERC-165
```

- An allowance is live when `now < expiry` and it was written in the owner's current `approvalEpoch`. `allowance()` returns 0 when it is not live. `transferFrom` then reverts with `AllowanceExpired(owner, spender, expiry)`, or with `ERC20InsufficientAllowance(spender, 0, value)` if the allowance was revoked or never existed. A zero-value `transferFrom` is still a no-op, as in standard ERC-20.
- A 3-argument `approve` needs `now < expiry ≤ now + maxApprovalDuration`. `value == 0` revokes, and the expiry argument is then ignored.
- The legacy `approve(spender, value)` and the standard EIP-2612 `permit(...)` use `expiry = now + defaultApprovalDuration`. Even `type(uint256).max` expires.
- `revokeAll()` bumps the caller's epoch and kills every allowance in O(1).
- EIP-712 domain: `name = token name` and `version = "1"` (OZ EIP712, which self-heals on a chain-id change). Type strings:
  - `Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)`
  - `PermitWithExpiry(address owner,address spender,uint256 value,uint64 expiry,uint256 nonce,uint256 deadline)`

  Both share `nonces(owner)`.
- Constructor arguments: `(name, symbol, recipient, initialSupply, defaultDuration, maxDuration)`. It requires `0 < defaultDuration ≤ maxDuration`. The recommended values are 30 days and 365 days.

### 3.1 Compatibility trade-off of the legacy `approve`

dApps written for "approve the maximum once, forever" will see the allowance drop to 0 after `defaultApprovalDuration` (30 days by default), and their `transferFrom` will revert. The user then has to approve again. This is intended: stale unlimited approvals are the main drainer vector. Integrators should:

- read `allowance()` before every pull;
- prefer `approve(spender, amount, expiry)` or `permitWithExpiry`, which they can detect through `supportsInterface(type(IXDRC20).interfaceId)`;
- treat `AllowanceExpired` as "ask again".

Tools that cache allowances off-chain (aggregators, some wallets) will show stale values unless they also read `allowanceExpiry`.

### 3.2 Events for indexers (`xdevs_getApprovals`)

- `Approval(owner, spender, value)` is the standard ERC-20 event.
- `ApprovalWithExpiry(address indexed owner, address indexed spender, uint256 value, uint64 expiry)` is emitted on every allowance write: approve (both forms), permit and permitWithExpiry.
- `AllApprovalsRevoked(address indexed owner, uint32 newEpoch)` marks every earlier approval of `owner` as dead.

Live set for an owner: take the latest `ApprovalWithExpiry` per spender that comes after the owner's latest `AllApprovalsRevoked`, and keep it if `value > 0` and `expiry > now`. Spending through `transferFrom` emits no approval event (OZ v5 behaviour), so read `allowance(owner, spender)` to get the remaining amount.

### 3.3 Canonical bridged tokens (§22.3)

`CanonicalTokenFactory(governance, defaultDuration, maxDuration)` has one deploy function. `createCanonicalToken(originChainId, originToken, name, symbol, decimals, bridge)` is governance only. It deploys an `XDRC20Bridged` (an XDRC-20 with expiring approvals, where only `bridge` can `mint` or `burn`) by CREATE2 with salt `keccak256(abi.encode(originChainId, originToken))`.

There is at most one canonical token per origin asset. Wallets and explorers verify a token with `isCanonical(token)`, `canonicalToken(chainId, originToken)` or `originOf(token)`, or index `CanonicalTokenCreated(originChainId, originToken, token, bridge, name, symbol, decimals)`. A copy that only reuses the name or symbol is never canonical.

## 4. Permit2Expiring (`0x…1008`)

This is a Permit2-style AllowanceTransfer for any ERC-20. The user approves `0x…1008` on the token once. Per-spender expiring allowances are then kept here. The interface follows Uniswap `IAllowanceTransfer`:

```solidity
function approve(address token, address spender, uint160 amount, uint48 expiration) external;
function permit(address owner, PermitSingle calldata, bytes calldata sig) external;
function permit(address owner, PermitBatch calldata, bytes calldata sig) external;
function transferFrom(address from, address to, uint160 amount, address token) external;
function transferFrom(AllowanceTransferDetails[] calldata) external;
function allowance(address user, address token, address spender) external view
    returns (uint160 amount, uint48 expiration, uint48 nonce);
function lockdown(TokenSpenderPair[] calldata) external;     // batch revoke
function revokeAll() external;                               // epoch bump: everything dies
function invalidateNonces(address token, address spender, uint48 newNonce) external;
function epochOf(address user) external view returns (uint48);
function DOMAIN_SEPARATOR() external view returns (bytes32);
uint256 constant MAX_EXPIRY_WINDOW = 365 days;
```

Rules:

- There are no permanent allowances. A non-zero amount needs `now < expiration ≤ now + 365 days`. Uniswap's "expiration 0 means this block" is not supported. `amount == 0` is a revoke.
- An allowance is spendable only while `now < expiration` and its epoch equals `epochOf(owner)`. `type(uint160).max` means "not decremented", but it still expires.
- `allowance()` returns `amount = 0` once the allowance has expired, and `(0, 0, nonce)` after `revokeAll`. Uniswap returns the raw amount instead.
- Permit nonces are per (owner, token, spender), must match exactly, and are checked to never wrap. `revokeAll` and `lockdown` never reset nonces, so an already-used signature can never be replayed.
- `revokeAll()` bumps `epochOf(owner)`. Every permit carries the `epoch` it was signed for, so this kills every allowance and every signed-but-unsubmitted permit at once.
- Signatures are checked with OZ `SignatureChecker`: ECDSA for EOAs, ERC-1271 for smart accounts and 7702-delegated accounts.

EIP-712:

```
EIP712Domain(string name,uint256 chainId,address verifyingContract)      name = "Permit2Expiring", no version
PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)
PermitSingle(PermitDetails details,address spender,uint256 sigDeadline,uint48 epoch)
PermitBatch(PermitDetails[] details,address spender,uint256 sigDeadline,uint48 epoch)
```

The domain separator is recomputed on every call from `block.chainid` and `address(this)`. A signature does not verify on another chain or at another contract address; the tests cover both. The domain name and the `epoch` field differ from Uniswap on purpose, so Uniswap's permit2-sdk needs a small adapter (custom domain name and an extra field).

Events for indexers (`xdevs_getApprovals`):

| Event | Meaning |
|---|---|
| `Approval(owner, token, spender, amount, expiration)` | All three addresses indexed. Also emitted with amount 0 for a revoke. |
| `Permit(owner, token, spender, amount, expiration, nonce)` | Allowance written from a signed permit. |
| `Lockdown(owner, token, spender)` | Pair revoked by `lockdown`. |
| `AllRevoked(owner, newEpoch)` | Everything earlier is dead. |
| `NonceInvalidation(owner, token, spender, newNonce, oldNonce)` | Nonces skipped. |

Live set: take the latest `Approval`, `Permit` or `Lockdown` for each (owner, token, spender) that comes after the owner's latest `AllRevoked`, and keep it if `amount > 0` and `expiration > now`. `transferFrom` emits no event here (the token's `Transfer` event fires), so read `allowance()` for the remaining amount.

## 5. Tests (what is proven)

Test files: `XNSRegistry.t.sol`, `XNSRegistry.invariant.t.sol`, `XDRC20.t.sol` (unit, fuzz, invariant), `Permit2Expiring.t.sol` (unit, fuzz, invariant) and `CanonicalTokenFactory.t.sol`.

- ENS namehash vectors, the reverse-node encoding and resolver interface IDs.
- Genesis placement: `vm.etch` of the runtime code at `0x…1004` and `0x…1008`. Initialization works only from `SYSTEM_ADDRESS`. The domain separator is dynamic per chain id and per address.
- Invariants:
  - an expired allowance is never spent (XDRC-20 and Permit2Expiring);
  - an allowance removed by revokeAll is never spent;
  - `allowance()` is 0 after expiry;
  - a name is never registered twice while it is active or in grace;
  - confusable names never coexist;
  - expired names never resolve.
- Fuzz:
  - normalization against a reference model on random bytes;
  - any forbidden byte or uppercase letter is rejected;
  - an expired allowance cannot be spent but a live one can;
  - revokeAll over N tokens × M spenders zeroes everything.
- Permits: replay (nonce), wrong chain id, wrong contract, tampered fields, deadline, ERC-1271 smart account, stale epoch after revokeAll, and nonces not reset by revokeAll.

## 6. Limitations and open issues

- No commit-reveal. With a zero fee on testnet, registrations can be front-run and squatted. A commit-reveal controller, or a mainnet fee with premium decay, is needed before mainnet.
- The name is not an NFT. ENS's BaseRegistrar is ERC-721; XNS keeps ownership in the registry only, with a single owner and no separate registrant/controller roles. Marketplaces cannot list names.
- There is no UniversalResolver and no ENSIP-10 wildcard or CCIP-read. viem's default ENS path needs ENS's UniversalResolver. It can be deployed later as an ordinary contract pointed at `0x…1004`, since it needs no registry change.
- `contenthash`, `ABI`, `pubkey` and `interfaceImplementer` resolver profiles and `multicall` are not implemented, to keep the predeploy compact.
- The confusable skeleton covers only the ASCII-lowercase UTS#39 pairs. It does not cover visual similarity such as `ii`/`u`, or hyphen insertion (`pay-pal` and `paypal` can coexist). Wallets should still flag look-alikes (`xdevs_checkRecipient`).
- Subname labels are not validated on-chain, the same as in ENS.
- Expired names stop resolving immediately, unlike ENS where they resolve during grace. This is safer but a stricter UX: renewal reminders matter.
- Governance is a plain address set at init (the Governance predeploy). Timelocking is the job of the Governance contract.
- XDRC-20 does not cap the legacy `approve` amount, only its lifetime. The smart-account allowance policy module (§16.5, bullet 3) belongs to the accounts work area.
- Permit2Expiring implements AllowanceTransfer only. Uniswap's SignatureTransfer (`permitTransferFrom` with unordered nonces and witness) is not included.
- Upgradeability: none. The plan says "upgradeable only via Governance + timelock", which for predeploys means a hard-fork code replacement. No proxy is used.
- Not yet run: Slither and Aderyn (only the `forge build` lints were run, and the warnings they report are informational or false positives: timestamp comparisons, events after internal calls), and coverage measurement.

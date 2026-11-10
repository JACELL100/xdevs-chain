# xDEVS Smart Accounts, Guardian Vault & Paymaster — Specification

Status: testnet v1 (Phase 4). Plan references: §8.2 (EIP-7702), §10.3 (P256VERIFY), §15.3 (paymaster / stablecoin
gas), §16.1, §16.2, §16.5, §16.6, §18, §33.1.

Source: `contracts/src/accounts/`, tests: `contracts/test/accounts/`, scripts: `contracts/script/accounts/`.
Build / test: `cd contracts && FOUNDRY_PROFILE=accounts forge test`.

---

## 1. Components

| Contract | Where | Role |
|---|---|---|
| `XdevsSmartAccount` | predeploy `0x…1005` (`Predeploys.SMART_ACCOUNT_IMPL`) | Account **core** (hot path): ERC-4337 v0.9 validation / execution, Guardian Vault dispatcher & queue, session-key enforcement, ERC-1271, token receivers. EIP-7702 delegation target and ERC-1967 proxy implementation. |
| `AccountExtension` | code hosted by the factory predeploy `0x…1006` | Account **cold path**: initialization, signer / guardian / policy / session management, social recovery, views, proxy upgrades. Reached from the core's `fallback()` with `DELEGATECALL`. |
| `SmartAccountFactory` (`is AccountExtension`) | predeploy `0x…1006` (`Predeploys.SMART_ACCOUNT_FACTORY`) | CREATE2 factory of ERC-1967 proxies (`createAccount`, `createAccountWithConfig`, `getAddress…`). Also the extension host. |
| `XdevsPaymaster` | predeploy `0x…1007` (`Predeploys.PAYMASTER`) | Shared deposit-based gas sponsorship (ERC-4337 v0.9 paymaster). |
| `StablecoinPaymaster` | normal deployment (example) | Pay gas in an approved ERC-20 at an operator-set rate (application layer, no protocol oracle). |
| `EntryPoint` v0.9 | canonical `0x433709009B8330FDa32311DF1C2AFA402eD8D009` | eth-infinitism EntryPoint, must be in genesis. |

Libraries / modules: `AccountCore` (authority + low-level calls), `SignatureVerifier`, `VaultDispatcher`,
`SessionExecutor`, `CallClassifier` (approval / transfer recognition), `WebAuthnP256` (passkeys via the precompile),
`AccountStorage` (ERC-7201 layout), `AccountTypes`, `AccountConstants`. Interfaces: `IGuardianVault` (plan §18.1),
`IAccountExtension`, `IAccountEvents`.

### Why a core + extension split

The full feature set compiled into one contract is ~32 KiB, above the EIP-170 limit (24 KiB). Predeploys could
ignore the limit, but upgraded implementations must be deployable by normal transactions, and tests deploy with
`new`. The core (15.6 KiB) keeps everything that runs on every operation; the extension (inside the 21.4 KiB factory)
holds rarely-used management code. Both operate on the same ERC-7201 storage, `msg.sender` and transient storage,
because the extension is only ever executed via `DELEGATECALL` from an account.

Hosting the extension in the factory predeploy avoids a new genesis address. Safety of that choice:
* factory entry points (`createAccount…`, `getAddress…`) carry `onlyFactory` (`address(this) == 0x…1006`) and revert
  when reached through an account;
* extension functions called directly on the factory are inert (no signers / guardians, non-empty code ⇒
  `initialize` and every owner / guardian action fails authorization).

The extension address is `AccountConstants.ACCOUNT_EXTENSION = Predeploys.SMART_ACCOUNT_FACTORY`.

---

## 2. Genesis placement & init params

Constructors never run at the predeploy addresses, nothing reads immutables, nothing caches `address(this)`.

| Address | Runtime code | Storage | Init call |
|---|---|---|---|
| `0x…1005` | `XdevsSmartAccount` `deployedBytecode` | none | **none** — `initialize` reverts at this address (`address(this) == SMART_ACCOUNT_IMPL` check), so the implementation can never be claimed |
| `0x…1006` | `SmartAccountFactory` `deployedBytecode` | none | **none** — the implementation address is the constant `Predeploys.SMART_ACCOUNT_IMPL` |
| `0x…1007` | `XdevsPaymaster` `deployedBytecode` | none | **none** — fully permissionless; after genesis anyone may fund its EntryPoint stake with `addStake(unstakeDelay)` |
| `0x433709…D009` | EntryPoint v0.9 | per its constructor | EntryPoint has constructor-set immutables (`SenderCreator`, EIP-712 domain): insert the code/state produced by its canonical deterministic deployment, not the raw artifact |
| `0x100` | `P256VERIFY` precompile | — | RIP-7212 / EIP-7951 semantics (160-byte input, returns `uint256(1)` or empty) |

`script/accounts/PredeployArtifacts.s.sol` prints sizes and code hashes of the three predeploys.

---

## 3. Storage namespaces (ERC-7201)

Slot = `keccak256(abi.encode(uint256(keccak256(id)) - 1)) & ~bytes32(uint256(0xff))`. Namespaced storage means an
EOA that delegates via EIP-7702 is never confused by storage written under a previous delegate (sequential slots or
other ids). `test_namespacedStorage_ignoresPreviousDelegateState` and `test_erc7201SlotsMatchFormula` cover this.
The `initialized` flag is in our own namespace (not OpenZeppelin's `Initializable` slot) for the same reason.

| Namespace id | Slot | Contents |
|---|---|---|
| `xdevs.account.core` | `0x4fb707dc31f3c888e2f3b1f3950af08abe11750bfa6dde38bfbc009dfff4e600` | `initialized` |
| `xdevs.account.signers` | `0x21ef5ba36445439568de9d93ffe50a1b636abb2f9f1dbdf7632c73b4de7b5700` | signers by id, id list |
| `xdevs.account.vault` | `0x3b7942c4632d59ecbc64fa168430d4f52e480831291d1cfa5d411ad6960f6800` | policy, daily counters, gas budget, queue, epoch, whitelist, token limits, approved hashes |
| `xdevs.account.recovery` | `0x16baddd4c0fda797a0d3882b67686eb4bfc7c2204c2df624c7e02b7d760da300` | guardians, threshold, delay, nonce, proposals |
| `xdevs.account.sessions` | `0x67bd91acb6e6fc2368bd19707c2fc36943099bf0066891511ab3c54667060000` | session keys, permissions, token caps |
| `xdevs.account.transient.selfauth` (EIP-1153 transient) | `0x88ad6324a261da9ba14b4c392d05cf2b1f49063cd1187e8e45deee7afae23600` | self-authorization flag |
| `xdevs.paymaster.main` | `0x8d3f6c7b5b47817ccada215d77f06104b1ae363f60066aafad2762303b92f800` | `XdevsPaymaster` sponsors |

ERC-1967 proxies additionally use the standard implementation slot.

---

## 4. Authority model

| Authority | Who | Can |
|---|---|---|
| **Owner** | `msg.sender` is the account itself (7702 EOA self-call / internal self-call), the EntryPoint (after an *owner* signature was validated), or a registered secp256k1 signer calling directly | `execute`, `executeBatch`, instant management (add signer / session, revocations, cancel), request delayed changes |
| **Self-authorized** | an internal self-call issued by the account's dispatcher (instant self-call allowed by the vault, or a matured queued action); flagged in transient storage | apply delayed changes (`delayedSelf` functions) |
| **Guardian** | addresses in the guardian set (EOAs or other smart accounts) | `cancel(id)` vault actions, `approveRecovery` |
| **Session key** | secp256k1 key with a session config | `executeWithSession` / session-signed `executeUserOp` within its limits |
| **Anyone** | — | `execute(id)` of a matured action, `executeRecovery` of a ready recovery, `pokeGasBudget`, deposits |

There is **no admin** anywhere: no key can freeze, seize or move user funds (§16.6). The paymaster's only privileged
role is Governance unlocking/withdrawing the paymaster's *stake* (never sponsor deposits).

---

## 5. Signers and signatures

Signers: up to 16 per account, each `Signer{kind, x, y}`:
* `ECDSA`: `x = bytes32(uint256(uint160(addr)))`, `y = 0`; id = the padded address.
* `Passkey`: P-256 public key `(x, y)` (validated on-curve); id = `keccak256(abi.encode(x, y))`.

The account address itself is an **implicit secp256k1 signer**: for an EIP-7702 EOA this is the EOA key; for a CREATE2
proxy nobody has such a key. Passkey-only accounts therefore work (no seed phrase anywhere).

Owner signature encodings (ERC-4337 `userOp.signature` and ERC-1271):

| Encoding | Meaning |
|---|---|
| 65 bytes `r‖s‖v` | secp256k1 |
| `0x01 ‖ r‖s‖v` (66 bytes) | secp256k1, explicit type |
| `0x02 ‖ abi.encode(bytes32 x, bytes32 y, WebAuthn.WebAuthnAuth auth)` | passkey (OpenZeppelin `WebAuthnAuth` struct) |
| `0x03 ‖ sessionKey(20) ‖ r‖s‖v` (exactly 86 bytes) | session key (ERC-4337 only) |

Dispatch is by exact length first (a raw 65-byte signature may begin with any byte — regression-tested).

**WebAuthn verification** (`WebAuthnP256`, same checks as OpenZeppelin `WebAuthn.verify(..., requireUV=true)`):
`"type":"webauthn.get"` at `typeIndex`; `"challenge":"<base64url(challenge)>"` at `challengeIndex` where the challenge
is the 32-byte hash being signed (userOpHash or ERC-1271 replay-safe hash); User-Present and User-Verified flags;
BE/BS consistency; low-s; P-256 signature over `sha256(authenticatorData ‖ sha256(clientDataJSON))` via the
`P256VERIFY` precompile at `0x100`. Origin, rpIdHash and the signature counter are not checked on-chain
(authenticator / wallet responsibility). There is **no pure-Solidity fallback** (it would break the size limit): if the
precompile is absent, passkey signatures are invalid (fail closed, tested).

**ERC-1271**: signers sign `replaySafeHash(hash)` = EIP-712 digest with domain
`{name:"xDEVS Smart Account", version:"1", chainId, verifyingContract: account}` and struct
`XdevsAccountMessage(bytes32 hash)` — prevents replaying one signature on another account sharing the key.
EIP-7702 compatibility: a raw 65-byte signature of the account's own EOA key over the raw `hash` is also accepted.
With the vault enabled and `allowInstantSignatures == false` (default), **only hashes pre-approved on-chain via the
delayed `approveHash`** are valid — otherwise a thief could bypass the vault with off-chain permits (Permit2, Seaport,
USDC's ERC-1271 permit, ...).

---

## 6. Flows

### 6.1 Factory account (new user, passkey only)
1. Wallet creates a passkey, computes `factory.getAddress([passkey], salt)` (or `getAddressWithConfig(params, salt)`).
2. Faucet / friend funds the counterfactual address (or a paymaster sponsors).
3. First UserOperation: `initCode = factory ‖ createAccount(signers, salt)`, signed by the passkey. The EntryPoint's
   SenderCreator calls the factory, which CREATE2-deploys `ERC1967Proxy(0x…1005, initialize(params))`.
   The address commits to the init data, so nobody can front-run with other owners.
`createAccount(signers, salt)` uses defaults: no guardians, vault **disabled** (opt-in, §16), recovery delay 48 h,
vault delay 24 h once enabled. `createAccountWithConfig(InitParams, salt)` sets everything at once.

### 6.2 Existing EOA via EIP-7702 (same address)
1. EOA signs an EIP-7702 authorization to `0x…1005` (type-4 tx).
2. In the same tx (or later) the EOA calls `initialize(params)` **on itself** (`msg.sender == address(this)`); any
   other caller is rejected (front-running protection). Alternatively anyone relays
   `initializeWithSignature(params, sig)` where `sig` is the EOA's EIP-191 signature over
   `initHash(params) = keccak256(abi.encode(INIT_TYPEHASH, chainId, account, keccak256(abi.encode(params))))`
   (usable through ERC-4337 `initCode = 0x7702…` + init call data).
3. Uninitialized, the account already works with the EOA key as implicit signer (direct self-calls, UserOps, 1271).
4. `upgradeToAndCall` is not available to 7702 accounts (they re-delegate instead).

### 6.3 ERC-4337 v0.9
* `validateUserOp`: only from the EntryPoint; owner signature over `userOpHash` (v0.9 hash is EIP-712 over the op,
  chain and EntryPoint), or a session signature (see §9). Prefund `missingAccountFunds` is paid as the standard
  requires; with the vault enabled it is charged to the **daily gas budget** (§7.6) and an over-budget op returns
  `SIG_VALIDATION_FAILED`. No TIMESTAMP / foreign storage access in validation (ERC-7562).
* Execution: `callData` may be `execute(target,value,data)`, `executeBatch(Call[])`, any owner function, or
  `executeUserOp` (`IAccountExecute`) with `callData = 0x8dd7712f ‖ abi.encode(Call[])` — required for session keys.
* `getNonce()`, `addDeposit()`, `getDeposit()` helpers.

### 6.4 Direct execution
Registered secp256k1 signers (and a 7702 EOA on itself) may call `execute` / `executeBatch` directly and pay gas
themselves. Session keys may call `executeWithSession(Call[])` directly.

---

## 7. Guardian Vault (plan §16.2, §16.5, interface §18.1)

### 7.1 Policy

```solidity
struct VaultPolicy {
    bool enabled;                 // master switch
    bool blockUnlimitedApprovals; // allowance policy: reject (not queue) unlimited approvals to non-whitelisted spenders
    bool allowInstantSignatures;  // ERC-1271 without on-chain pre-approval (default false)
    uint64 delay;                 // 0 => 24 h; bounds [1 h, 7 d]
    uint128 nativeThreshold;      // max native value of one instant execution (batch total)
    uint128 dailyLimit;           // max native value of instant executions per UTC day
    uint128 dailyGasBudget;       // max ERC-4337 prefund per UTC day; 0 => 1 tXDS
}
```
Plus a whitelist (≤ 64 trusted contracts / recipients / spenders) and per-token limits
`setTokenLimit(token, perTx, daily)` in token units (no oracle).

### 7.2 Classification (vault enabled)
A batch executes instantly only if **every** call is instant; otherwise the **whole batch is queued atomically**.

| Call | Instant when |
|---|---|
| Self-call (account management) | selector in the instant list: `addSigner`, `addSessionKey`, `revokeSessionKey`, `revokeAllSessionKeys`, `cancel`, `cancelRecovery`, `revokeHash`, `pokeGasBudget`. Everything else (unknown selectors included) is delayed. |
| Approval-like: `approve`, `increaseAllowance`, `setApprovalForAll`, Permit2 `approve(token,spender,amount,exp)`, XDRC-20 `approve(spender,amount,expiry)`, ERC-777 `authorizeOperator` | revocation (amount 0 / `false`) or spender whitelisted. Unlimited (≥ `uint160.max`, `setApprovalForAll(true)`, operators) to a non-whitelisted spender is **rejected** if `blockUnlimitedApprovals`, otherwise queued. |
| Asset transfer out: `transfer`, ERC-777 `send`, `transferFrom(self,…)`, `safeTransferFrom(self,…)` (721 / 1155), `safeBatchTransferFrom(self,…)` | recipient whitelisted, or a fungible transfer of a token with a configured limit and within `perTx` and the token's daily limit |
| Known selector with truncated calldata | never (queued) |
| Any other call with calldata | target whitelisted |
| Native value to a non-whitelisted target | batch total ≤ `nativeThreshold` and today's spend + total ≤ `dailyLimit` |

Whitelisted targets / recipients do not consume the daily limit. Below-threshold actions stay instant, so daily UX
is unaffected.

### 7.3 Queue lifecycle
* `VaultActionQueued(id, target, value, data, executeAfter)` with `executeAfter = now + policy.delay` (a batch is
  reported as `target = account`, `value = total`, `data = executeBatch(calls)` calldata).
* `cancel(id)`: owner (any signer) or any guardian, any time before execution → `VaultActionCancelled(id, by)`.
* `execute(id)`: **anyone**, only in `[executeAfter, executeAfter + 14 days]`; status is set to `Executed` before
  the calls run (no re-entrancy / double execution); if a call reverts the whole execution reverts and the action stays
  queued (retry later). → `VaultActionExecuted(id)`.
* A successful social recovery bumps the vault **epoch**: every action queued before is permanently invalid.
* Views: `getAction(id)`, `lastActionId()`, `spentToday()`, `vaultPolicy()`, `getWhitelist()`, `tokenLimit(token)`.
  Off-chain: wallets / the alert bot subscribe to these events (`xdevs_subscribe("vaultEvents", account)`).

### 7.4 Delayed changes ("a thief must not switch it off")
When the vault is enabled, these are queued with the *current* delay and are cancellable like any action:
`setPolicy`, `setVaultPolicy` (including disabling the vault), `setWhitelisted`, `setTokenLimit`, `approveHash`,
`removeSigner`, `setGuardians`, `setRecoveryDelay`, `upgradeToAndCall`. Implementation: these functions are
`delayedSelf`; called by an owner they route themselves through `execute(self, 0, msg.data)` and the dispatcher
queues them; they only apply inside a self-call made by the dispatcher (transient flag), so not even a 7702 EOA
calling itself can skip the queue. When the vault is disabled they apply immediately (enabling the vault via
`setPolicy` is therefore instant).

### 7.5 Allowance policy (§16.5)
Unlimited approvals are blocked (or queued, per `blockUnlimitedApprovals`) unless the spender is whitelisted;
limited approvals to non-whitelisted spenders are queued; revocations are always instant.

### 7.6 ERC-4337 gas budget
A thief with an owner key could run their own bundler and set an absurd gas price, converting the balance into
"fees" paid to themselves. With the vault enabled the value moved from the account to the EntryPoint is capped at
`dailyGasBudget` per UTC day. Because ERC-7562 forbids TIMESTAMP in validation, the counter is charged in validation
and reset at the first execution / `pokeGasBudget()` of a new day; worst case ≈ 2× the budget around a day boundary.
Paymaster-sponsored ops are not affected.

---

## 8. Social recovery (plan §16.1, §33.1)
* Guardians (≤ 16) and threshold N-of-M; recovery delay default **48 h** (bounds 1 h – 30 d).
* `approveRecovery(newSigners)` by each guardian (the first is the proposal). Approvals are per exact signer set.
  When approvals reach the threshold: `readyAt = now + delay` (`RecoveryReady`).
* `executeRecovery(newSigners)` by anyone after `readyAt`: replaces the whole signer set, revokes all session keys
  and invalidates every queued vault action.
* `cancelRecovery()` by the owner at any time before execution (nonce bump invalidates all proposals). Changing the
  guardian set also invalidates pending approvals.

## 9. Session keys
`addSessionKey(SessionKeyConfig)` (instant, owner): secp256k1 `key`, `[validAfter, validUntil]` (inclusive),
lifetime `nativeCap`, up to 32 `(target, selector)` permissions (`selector = 0` = any function; the account itself
can never be a target), per-token caps. Enforcement:
* every call must be permitted; native value is debited from `nativeCap`; ERC-20 transfers / `transferFrom(self)` /
  non-revoking approvals are debited from the token's cap (no cap ⇒ 0); NFTs are debited by token id (practically
  disallowed);
* the call then goes through the vault in "session mode": anything the vault would queue **reverts**, and instant
  native spend counts against the vault daily limit;
* ERC-4337: signature `0x03 ‖ key ‖ sig(userOpHash)`, `callData` must be `executeUserOp`; validation checks
  liveness and permissions and returns the time window in `validationData`; caps are enforced at execution.
* `revokeSessionKey(key)`, `revokeAllSessionKeys()`; re-adding a key resets its permissions and spend.

---

## 10. Paymasters (plan §15.3)

### 10.1 `XdevsPaymaster` (predeploy, shared)
* Apps ("sponsors") `deposit(sponsor)` tXDS (forwarded to the EntryPoint), `configure(active, openToAll,
  maxCostPerOp, perAccountCap, signer)`, `setAllowlist(accounts, bool)`, `resetAccountSpend(accounts)`,
  `withdraw(to, amount)` (own balance only).
* `paymasterAndData = paymaster ‖ verifGas(16) ‖ postOpGas(16) ‖ sponsor(20)`; signer mode adds
  `‖ validUntil(6) ‖ validAfter(6)` and the v0.9 paymaster-signature suffix `sig(65) ‖ uint16(65) ‖ 0x22e325a297439656`,
  `sig` = sponsor signer's EIP-191 signature over the userOpHash (the suffix is excluded from the hash).
* Per-account caps are lifetime (resettable) because validation cannot read the clock; time-boxed policies use
  signer mode.
* Accounting reserves `maxCost` in validation and charges `min(maxCost, actualGasCost + (postOpGasLimit + 15000) ×
  feePerGas)` in `postOp`, never less than what the EntryPoint deducts ⇒ `EntryPoint.balanceOf(paymaster) ≥
  Σ sponsor balances` (fuzz-tested). The surplus can be swept to the Treasury by anyone.
* Must be **staked** in the EntryPoint for bundlers (it reads sponsor-keyed storage in validation): anyone can
  `addStake`; only Governance can `unlockStake` / `withdrawStake`.

### 10.2 `StablecoinPaymaster` (example, normal deployment)
Operator sets `setToken(token, enabled, tokensPerNative)` (token base units per 1e18 wei) and a markup (≤ 20 %).
`paymasterData = token(20) ‖ maxTokensPerNative(32)` — the user's signed rate limit, so the operator cannot raise the
price under a pending op. Validation pulls `maxCost × rate` from the sender (prior `approve` required; with the vault
enabled, whitelist the paymaster or the approval is queued), `postOp` refunds the unused part. Deploy with
`script/accounts/DeployStablecoinPaymaster.s.sol`.

---

## 11. `InitParams` reference

```solidity
struct InitParams {
    Signer[] signers;        // >= 1 (passkey-only allowed), <= 16
    address[] guardians;     // may be empty (no recovery)
    uint8 guardianThreshold; // 0 iff no guardians, else 1..guardians.length
    uint64 recoveryDelay;    // 0 => 48 h; [1 h, 30 d]
    VaultPolicy policy;      // enabled=false => vault off; delay 0 => 24 h; dailyGasBudget 0 => 1 tXDS
    address[] whitelist;     // <= 64
    TokenLimit[] tokenLimits;
}
```
Recommended wallet defaults with the vault on: threshold 1,000 tXDS (§16.2 example), a daily limit of a few
thresholds, 24 h delay, `blockUnlimitedApprovals = true`, `allowInstantSignatures = false`, 2-of-3 guardians.

---

## 12. Limitations (documented honestly)

1. **Protects only assets held by the account.** Anything already approved to third parties before the vault was
   enabled, or held elsewhere, is out of scope.
2. **Doesn't stop the owner willingly paying a scammer after the delay** — the delay only buys time to notice and
   cancel.
3. **EIP-7702 accounts: the EOA key remains all-powerful at the protocol level.** It can send plain value
   transactions, call tokens directly, or re-delegate, bypassing every contract-level rule. For 7702 accounts the
   vault protects only the contract-level paths (e.g. a compromised session / dApp, a stolen *additional* passkey).
   Real "stolen key" protection requires a factory account (whose address has no private key).
4. **Whitelists are trust.** A whitelisted contract can be called instantly with any calldata; e.g. a whitelisted DEX
   router with an existing allowance lets a thief swap the account's tokens to an arbitrary recipient. Classification
   is syntactic (known selectors); unknown approval/transfer-like functions on a whitelisted contract are not
   recognised.
5. **Bounded instant loss.** A thief with an owner key can still spend up to the daily limit per day (and token
   daily limits), plus the gas budget.
6. **Owner vs. thief tie.** The owner can cancel recoveries (plan requirement) and guardians can cancel queued
   actions. A thief holding a signer key can therefore keep cancelling recoveries while the guardians keep cancelling
   the thief's queued actions: funds above the daily limit stay safe, but the account is stuck until the thief gives
   up. Likewise a single malicious guardian can block all *queued* actions (liveness, not theft). Guardians are
   trusted for liveness. Possible future mitigation: guardian-threshold "freeze" that disables instant spending.
7. ERC-1271 with the vault on requires on-chain pre-approval of each hash (delayed) unless the user opts into
   `allowInstantSignatures`, which re-opens the off-chain-permit bypass.
8. WebAuthn origin / rpId / counter are not verified on-chain. The `P256VERIFY` precompile is mandatory.
9. The extension lives in the factory predeploy; like the implementation itself, a governance hard-fork replacing
   predeploy code would change account behaviour (no runtime admin key exists).
10. Daily windows are UTC calendar days, not rolling 24 h windows.

---

## 13. Tests (`contracts/test/accounts`)

* `XdevsSmartAccount.t.sol` — factory determinism, genesis-style predeploys (code etched, no constructor), impl lock,
  passkey-only account deployed and used through the EntryPoint, WebAuthn negative cases (challenge, UV, BE/BS, key,
  high-s, type, missing precompile), ERC-1271 replay safety, signers, upgrades; fuzz of signature validation.
* `GuardianVault.t.sol` — thresholds, daily limit, atomic batches, whitelist, approvals / allowance policy, token
  limits, NFTs, cancel rights, expiry, retries, delayed policy changes, gas budget, **stolen-key demo**
  (`test_demo_stolenKey_guardianCancelsWithinWindow`), fuzz of classification / early execution / cancellation.
* `SocialRecovery.t.sol`, `SessionKeys.t.sol`, `EIP7702.t.sol`, `Paymaster.t.sol` (incl. solvency fuzz and the
  stablecoin paymaster).
* `Invariants.t.sol` — handler-based invariants: no early execution, cancelled never executes, no double execution,
  policy only changes through matured actions, value outflow matches ghost accounting; session cap & expiry; recovery
  threshold & delay and signer-set consistency.

Foundry's `prague` EVM has no `P256VERIFY`; tests etch `P256VerifierMock` (OpenZeppelin's Solidity verifier behind the
RIP-7212 ABI) at `0x100`, and use real P-256 signatures from `vm.signP256`.

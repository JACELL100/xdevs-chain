# Static analysis of the Solidity contracts (Slither + Aderyn)

Status: done for the Phase 1–4 contracts, 2026-11-10. Plan references: §18.3 (quality bar "Slither + Aderyn clean, or
documented false positives"), §25.2 (audit preparation).

Scope: everything under `contracts/src/` — `system/` (Staking, Governance, Treasury, FeeConfig, WXDS), `accounts/`
(XdevsSmartAccount + AccountExtension / SmartAccountFactory, XdevsPaymaster, StablecoinPaymaster, libraries),
`periphery/` (XNSRegistry, Permit2Expiring, XDRC20, XDRC20Bridged, CanonicalTokenFactory), `common/`. Dependencies
(`lib/`), tests and scripts are out of scope.

## 1. Result

| Tool | Before triage | After |
|---|---|---|
| Slither 0.11.6 (all 102 detectors) | **162**: 4 High, 34 Medium, 77 Low, 47 Informational | **0** with `contracts/slither.config.json`; 104 if the config exclusions are dropped (12 Medium `uninitialized-local`, 55 Low, 37 Info — all reviewed below) |
| Aderyn 0.6.8 (88 detectors) | 6 High (10 instances), 20 Low categories | **0 High**; 9 Low categories, all reviewed and accepted below (9 style detectors excluded in `contracts/aderyn.toml`, 2 categories fixed) |

**Real issues fixed: 1 (Low) + 2 cosmetic.** No High or Medium finding of either tool was a real bug; every one is a
false positive explained in §4–§5 and suppressed narrowly in the source with a reason and an id.

| Id | Severity | Contract | Issue | Fix | Regression test |
|---|---|---|---|---|---|
| **SA-01** | Low (found by manual review during triage, next to Slither's delegatecall / upgrade findings) | `AccountExtension.upgradeToAndCall` | The "not available to EIP-7702 accounts" rule was enforced only by `ERC1967Utils.getImplementation() == 0`. An EOA that previously delegated to another contract may carry a non-zero ERC-1967 implementation slot; after delegating to `0x…1005`, `upgradeToAndCall` then passed and its `DELEGATECALL` could rewrite **any** account storage — e.g. clear the XIP-0001 lock word, which `disableEoaLock` deliberately refuses while the EOA key is revoked. Reachable only with owner authority and through the vault delay (cancellable by guardians), so not an outsider attack, but a bypass of documented invariants. | Also revert `NotProxy` when `address(this)` carries the 7702 designator (`_isDelegatedEoa()`). | `EIP7702Test.test_upgrade_notAvailableFor7702_evenWithStaleErc1967Slot` (fails without the fix: the upgrade went through) |
| SA-02 | Info (Aderyn `unused-error`, `unused-import`) | `IPermit2Expiring`, `XNSRegistry` | Unused `LengthMismatch` error; unused `Predeploys` import | Removed (no bytecode change) | build |

Bytecode impact: only `SmartAccountFactory` (which hosts `AccountExtension`) changed — 22,710 B, same size, margin
1,866 B under EIP-170. **Every other contract's runtime bytecode is byte-identical** to commit `5ffa618` (checked by
hashing `deployedBytecode` of all 14 deployable contracts before / after; suppression comments do not affect
bytecode because `bytecode_hash = "none"` and `cbor_metadata = false`). Staking stays at 24,099 B (477 B margin).
No storage layout, `initialize` signature or event changed.

Tests: `forge test` → **340 passed, 0 failed** (339 before + the SA-01 regression test). `forge fmt --check` clean.

## 2. Tools and how to run

| Item | Version |
|---|---|
| Slither | 0.11.6 (crytic-compile 0.4.2), Python 3.11 |
| Aderyn | 0.6.8 (`aderyn-x86_64-unknown-linux-gnu`, sha256 `ffd6ca65…a691e46`) |
| Foundry / solc | forge 1.9.0, solc 0.8.38, `evm_version = osaka`, optimizer 200 runs |

```bash
cd contracts

# Slither — config picked up automatically from slither.config.json.
# NOTE: crytic-compile runs `forge clean` + `forge build --build-info --skip test script`, i.e. it wipes out/ and cache/.
python -m pip install --user slither-analyzer==0.11.6
slither .                              # expected: "0 result(s) found"
slither . --config-file all.json      # review run without the exclusions; all.json = slither.config.json minus
                                       # "detectors_to_exclude" (expected: 104 results, all in §4)

# Aderyn — Linux / macOS only (no Windows build). Config: aderyn.toml.
aderyn -o report.md                    # expected: 0 High, 9 Low categories (all in §5)
```

On Windows Aderyn was run from a throw-away `debian:bookworm-slim` container with the checksum-verified release
binary and a read-only copy of `contracts/`:
`docker run --rm -v <aderyn-dir>:/ad:ro -v <repo>/contracts:/src:ro debian:bookworm-slim sh -c 'mkdir /w && cp -r /src/src /src/lib /src/foundry.toml /src/aderyn.toml /w && cd /w && /ad/aderyn -o /w/r.md && cat /w/r.md'`.

CI: `.github/workflows/static-analysis.yml` runs on every PR / push to `main` touching `contracts/**`:
* `slither` — `crytic/slither-action@v0.4.2`, Slither 0.11.6, `contracts/slither.config.json`, `fail-on: low` (any new
  Low/Medium/High finding fails; informational ones that are not excluded also show up in the log).
* `aderyn` — pinned 0.6.8 release, sha256-verified; uploads `aderyn-report.{json,md}`; fails on any High.

### Suppression policy

* Reviewed false positives are suppressed **on the exact line** with
  `// slither-disable-next-line <detector> (SA-xx: reason)` or `// aderyn-fp-next-line(<detector>) SA-xx: reason`,
  where `SA-xx` is the row in this document. A new instance of the same detector elsewhere is still reported.
* Whole detectors are excluded only when they are pure style / gas, or when *every* instance is reviewed below and the
  detector cannot find a real bug in Solidity ≥ 0.8 code (`uninitialized-local`). The lists:
  * Slither (`slither.config.json`): `naming-convention`, `assembly`, `low-level-calls`, `too-many-digits`,
    `cyclomatic-complexity`, `uninitialized-local`, `timestamp`, `calls-loop`, `shadowing-local`.
  * Aderyn (`aderyn.toml`): `push-zero-opcode`, `unspecific-solidity-pragma`, `literal-instead-of-constant`,
    `large-numeric-literal`, `costly-loop`, `require-revert-in-loop`, `internal-function-used-once`,
    `modifier-used-only-once`, `unused-public-function`.

## 3. Focus areas reviewed manually (beyond the detectors)

| Area | Verdict |
|---|---|
| Native value transfers / reentrancy: Staking `withdrawUnbonded` / `claimRewards`, Treasury `grant`, Governance `execute`, WXDS `withdraw`, XNS fee, paymaster `withdraw` / `sweepSurplus`, vault `execute(id)` | All pull-based or governance-only, checks-effects-interactions; Staking/Treasury/Governance also `nonReentrant` (transient). Vault actions are marked `Executed` before their calls run. Read-only reentrancy: no view used by another contract (`getPastVotes`) changes during a payout. OK. |
| Access control on `initialize` / system functions | Staking/Governance/Treasury/FeeConfig: `SYSTEM_ADDRESS` + once. XNS: `GenesisInitializable` (system or constructor-deployer immutable = 0 in genesis code). Accounts: proxy constructor (`code.length == 0`) or 7702 self-call or EOA signature over `initHash` (chain id + account bound); impl and factory addresses can never be initialized. System calls `recordParticipation` / `slash` / `onEpoch`: `onlySystem`. OK. |
| Unchecked call returns | All low-level calls checked, except the ERC-4337 prefund in `validateUserOp` (deliberate, the EntryPoint verifies the deposit — ERC-4337 reference behaviour). `_routeSelfCall` ignores the queued id on purpose (SA-F13). OK. |
| DELEGATECALL to `AccountExtension` | Target is a constant (`0x…1006`); no state variables in any account contract (all state is ERC-7201 namespaced, slots tested against the formula) ⇒ no collisions between core, extension, factory or a previous 7702 delegate (lock word reset at init). Reachable only through the account's `fallback` / `setPolicy` forwarder; factory entry points are `onlyFactory`; extension functions called directly on the factory are inert. `upgradeToAndCall` hardened (SA-01). |
| Arbitrary send | SA-F01–F04: senders authorised by allowance / signature / `msg.sender`. OK. |
| Uninitialized storage | Impossible in 0.8 (compile error); all 12 Slither `uninitialized-local` hits are zero-initialised accumulators (SA-F06). |
| Signature replay | ERC-1271: EIP-712 domain `{name, version, chainId, verifyingContract = account}`; raw 65-byte path only for the account's own (non-revoked) EOA key. UserOps: v0.9 `userOpHash` (chain + EntryPoint + sender + nonce). Session keys sign the userOpHash. Paymaster signer: EIP-191 over the userOpHash (covers paymaster address, sponsor, validity window). `initializeWithSignature`: chain id + account, single use (`initialized`). Permit2Expiring: EIP-712 (chain id + address recomputed each call), per (owner, token, spender) nonce, epoch, deadline. XDRC-20: OZ EIP712 + `Nonces`, consumed before verification (reverts roll back). ECDSA via OZ (low-s), P-256 low-s enforced. OK. |
| Timestamp / block number | Used for delays, expiries, UTC days, freeze windows, unbonding. A proposer can skew `block.timestamp` by seconds; every window is ≥ 1 h (vault, governance, unbonding) or a UTC day (daily limits; documented ≈ 2× budget around the boundary). Not exploitable (SA-L01). |
| Division before multiplication (reward math) | None. `_credit`: `amount * bps / BPS` then `mulDiv(…, 1e30, totalShares)`; accrual rounded down, debt rounded up, so payouts never exceed credits (fuzzed + invariant). Remaining dust stays reserved (locked), never over-paid. Governance quorum `total * quorumBps / BPS`, fees `feePerYear * duration / 365 days`: multiply first. OK. |

## 4. Slither — every finding

Severity = Slither's impact. "FP" = false positive, suppressed as stated. Lines are as of this review.

### High (4)

| Id | Detector | Location | Verdict |
|---|---|---|---|
| SA-F01 | arbitrary-send-erc20 | `Permit2Expiring._transfer` | FP: `from` is not arbitrary — the allowance read is `_allowances[from][token][msg.sender]`, checked for epoch, expiry and amount before `safeTransferFrom`. Inline-suppressed. |
| SA-F02 | arbitrary-send-erc20 | `StablecoinPaymaster.validatePaymasterUserOp` | FP: only the EntryPoint can call; `userOp.sender` signed the op including `paymasterAndData` (token + max rate) and approved the paymaster; an invalid account signature reverts the whole bundle during validation. Inline-suppressed. |
| SA-F03 | arbitrary-send-eth | `Staking._send` | FP: only called with `msg.sender` (pull payments of the caller's own unbonded stake / rewards), after state updates, under `nonReentrant`. Inline-suppressed (also Aderyn `eth-send-unchecked-address`). |
| SA-F04 | arbitrary-send-eth | `XNSRegistry._payTreasury` | FP: recipient is `treasury`, settable only by governance (non-zero); amount is exactly the fee paid by the caller. Inline-suppressed. |

### Medium (34)

| Id | Detector | Location(s) | Verdict |
|---|---|---|---|
| SA-F05 | reentrancy-no-eth (4) + reentrancy-benign (4) + reentrancy-events (10, Low) | `AccountExtension` `delayedSelf` functions: `removeSigner`, `setGuardians`, `setRecoveryDelay`, `disableEoaLock`, `setPolicy`, `setVaultPolicy`, `setWhitelisted`, `setTokenLimit`, `approveHash`, `setFreezeDuration` | FP: the "external call" is the account calling **itself** (`execute(self, 0, msg.data)`) in the modifier's *else* branch, and the function body runs **only** in the *then* branch (self-authorised call issued by the dispatcher, transient flag). The body never executes after that call in the same frame; Slither is not path-sensitive across the modifier. Suppressed once, at the call (`_routeSelfCall`). |
| SA-F06 | uninitialized-local (12) | Staking `initialize.total`, `recordParticipation.signedPower/distributed`, `withdrawUnbonded.total/matured`; VaultDispatcher `_dispatch.nTokens/nativeTotal/needsQueue`, `_queue.totalValue`, `_consumeLimits.nativeSpent`; SessionExecutor `_executeSession.nativeTotal`; `XNSLabel.check.prev` | FP: zero-initialised value-type accumulators (Solidity guarantees 0 / false). Detector excluded in the config (it cannot find a real bug in ≥ 0.8 code; uninitialised storage pointers are a compile error). |
| SA-F07 | incorrect-equality (3) | Staking `recordParticipation` (`bn - 1 == lastSetChangeBlock`), `signingSet`, `_snapshotPrevSet` | FP: block-number bookkeeping, not balances; exact equality is the intended semantics. Inline. |
| SA-F08 | incorrect-equality | `XNSRegistry.renew` (`oldExpiry == 0`) | FP: 0 is the "never registered" sentinel. Inline. |
| SA-F09 | incorrect-equality (4) | VaultDispatcher `_consumeLimits` (×3), `AccountExtension.spentToday` | FP: comparing UTC day indexes (`timestamp / 1 days`). Inline. |
| SA-F10 | incorrect-equality | `XdevsPaymaster.sweepSurplus` (`amount == 0`) | FP: early exit when nothing to sweep; `surplus()` already saturates at 0. Inline. |
| SA-F11 | unused-return (4) | `ECDSA.tryRecoverCalldata` in XdevsSmartAccount (×2), SignatureVerifier, XdevsPaymaster | FP: the error code *is* checked; only the third return value (`errArg`) is ignored. Inline. |
| SA-F12 | unused-return (4) | Staking `_addVotes` / `_subVotes` (`Checkpoints.push`) | FP: `push` returns (old, new) values that are not needed. Inline. |
| SA-F13 | unused-return | `AccountExtension._routeSelfCall` | FP / by design: the queued action id is announced by `VaultActionQueued`; the routed functions return nothing. Inline (same comment as SA-F05). |

### Low (77)

| Id | Detector | Count | Verdict |
|---|---|---|---|
| SA-L01 | timestamp | 34 | Accepted, detector excluded. Reviewed every instance (Permit2/XDRC-20 expiries and deadlines, XNS expiry/grace, Governance voting/timelock/pause, Staking unbonding/jail/commission, vault delays/daily limits/gas-budget day, freeze, recovery, sessions). Validator timestamp skew is seconds; all windows are ≥ 1 h or a UTC day; ERC-4337 validation never reads TIMESTAMP (ERC-7562). |
| SA-L02 | incorrect-modifier | 1 | FP: `delayedSelf` intentionally does not run `_` when not self-authorised — it routes the call through the vault (instant or queued) instead. Inline. |
| SA-L03 | missing-zero-check | 2 | By design: Governance `guardian == 0` disables the emergency pause (spec §2). Inline. |
| SA-L04 | missing-zero-check | 2 | `XDRC20Bridged` constructor `bridge_`, `originToken_`: only deployed by `CanonicalTokenFactory.createCanonicalToken`, which reverts on zero for both. Inline. |
| SA-L05 | reentrancy-events | 3 | XdevsPaymaster `deposit` / `withdraw` / `sweepSurplus`: event after the call to the trusted EntryPoint; all state written before. Inline. (The other 10 reentrancy-events + 4 reentrancy-benign are SA-F05.) |
| SA-L06 | calls-loop | 6 | Accepted, excluded: Governance `execute` and account batch execution are external calls in a loop by definition; every call reverts the whole batch on failure, loops are bounded (16 governance actions; batch length chosen by the owner). |
| SA-L07 | shadowing-local | 15 | Accepted, excluded: all in *interfaces* — ENS-standard parameter names (`owner`, `resolver`, `ttl`, `node`) and `IStaking.initialize(governance, params, …)` vs getters of the same name. No runtime effect; renaming would diverge from EIP-137. |

### Informational (47)

| Id | Detector | Count | Verdict |
|---|---|---|---|
| SA-I01 | assembly | 18 | Accepted, excluded: ERC-7201 slot access, transient storage (EIP-1153), P256VERIFY staticcall, extension forwarding, `mstore` length trim in `_computeTopN` — all `memory-safe` and reviewed. |
| SA-I02 | low-level-calls | 7 | Accepted, excluded: native transfers via `call` (checked) and the account's arbitrary-call execution path. |
| SA-I03 | naming-convention (4), too-many-digits (3), cyclomatic-complexity (5) | 12 | Style; excluded. |
| SA-I04 | costly-loop | 1 | `Staking._bond` writes `totalBonded` in a loop only from the one-time genesis `initialize`. Inline. |
| SA-I05 | dead-code | 1 | `XdevsSmartAccount._routeSelfCall`: required implementation of the abstract `AccountCore` hook; the core has no `delayedSelf` function today (an unused internal function is not emitted). Inline. |
| SA-I06 | missing-inheritance | 1 | `IAccountExecuteSelf` is a one-function helper interface for the self-call; not worth inheriting. Inline. |
| SA-I07 | unused-state | 3 | `VOTE_*` constants are used by the core, not by `SmartAccountFactory` (shared base). Inline. |
| SA-I08 | unindexed-event-address | 4 | `XNS Initialized / TreasuryChanged / GovernanceChanged`, `CanonicalTokenFactory.GovernanceChanged`: rare admin events; indexing would change the log layout that indexers / the SDK already decode. Inline. |

## 5. Aderyn — every finding

### High (6 detectors, 10 instances) — all false positives, marked `aderyn-fp-next-line`

| Id | Detector | Location | Verdict |
|---|---|---|---|
| SA-A01 | abi-encode-packed-hash-collision | `SmartAccountFactory._bytecode` | FP: `creationCode ‖ abi.encode(impl, initData)` is CREATE2 init code (one dynamic part, then standard ABI encoding); no ambiguity. |
| SA-A02 | contract-locks-ether | `XdevsPaymaster`, `StablecoinPaymaster` | FP: neither has `receive`; every payable function forwards `msg.value` to the EntryPoint in the same call (deposit / stake), withdrawals go through `EntryPoint.withdrawTo/withdrawStake`. |
| SA-A03 | eth-send-unchecked-address | `WXDS.withdraw`, `Staking.withdrawUnbonded` (SA-F03), `XNSRegistry.renew` (SA-F04) | FP: recipient is `msg.sender` (own balance, CEI) or the governance-set treasury. |
| SA-A04 | reentrancy-state-change | `Governance.castVote` | FP: the "external call" is `getPastVotes`, a `view` (STATICCALL) on the fixed Staking predeploy. |
| SA-A05 | unprotected-initializer | `AccountExtension.initHash`, `GenesisInitializable.initialized()` | FP: name heuristic; both are `view` functions. The real initializers are protected (§3). |
| SA-A06 | yul-return | `XdevsSmartAccount._delegateToExtension` | FP: intended proxy-style forwarding of the extension's return data; only used as the last statement of `fallback` / `setPolicy`. |

### Low (20 categories)

| Id | Detector (instances) | Verdict |
|---|---|---|
| SA-02 | unused-error (1), unused-import (1) | **Fixed** (see §1). |
| SA-A07 | centralization-risk (17) | Accepted / by design: `onlyOwner` on account management means the *account owner* (not an admin); `StablecoinPaymaster` is an operator-owned example paymaster (Ownable2Step; users bound the price with `maxTokensPerNative`). No protocol admin key exists (spec smart-accounts §4). |
| SA-A08 | local-variable-shadowing (10) | FP: struct members of `FeeConfig.FeeParams` / `Staking.Validator.windowId` named like state variables; no shadowing at runtime. |
| SA-A09 | missing-inheritance (1) | `WXDS` is a faithful WETH9 port with its own event parameter names; inheriting `IERC20` would redeclare `Transfer`/`Approval`. Accepted. |
| SA-A10 | non-reentrant-not-first (2) | `Treasury.grant/grantToken`: `onlyGovernance` before `nonReentrant` makes no external call; order is irrelevant. Accepted. |
| SA-A11 | state-change-without-event (5) | Accepted: gas-budget / sponsor-reservation counters in ERC-4337 validation (`validateUserOp`, `pokeGasBudget`, `validatePaymasterUserOp`), and Permit2 `transferFrom` allowance decrement (as Uniswap Permit2; the token emits `Transfer`). |
| SA-A12 | state-no-address-check (2) | = SA-L03. |
| SA-A13 | unchecked-return (8) | = SA-F12 / SA-F13, plus `_dispatch` return ids in `executeUserOp` / `_routeSelfCall` / `_dispatchSession` (queued id not needed: sessions can never queue). |
| SA-A14 | uninitialized-local-variable (4) | = SA-F06. |
| SA-A15 | unused-state-variable (1) | FP: `XdevsPaymaster.STORAGE_SLOT` is used in inline assembly (`$.slot := STORAGE_SLOT`). |
| SA-A16 | push-zero-opcode (35) | Excluded: the chain runs revm Osaka (PUSH0 since Shanghai). Deploying the periphery contracts on a pre-Shanghai chain would need `evm_version` lowered. |
| SA-A17 | unspecific-solidity-pragma (29) | Excluded: solc pinned to 0.8.38 in `foundry.toml`; artifacts are deterministic (`bytecode_hash = none`). |
| SA-A18 | costly-loop (28), require-revert-in-loop (13), literal-instead-of-constant (48), large-numeric-literal (11), internal-function-used-once (4), modifier-used-only-once (3), unused-public-function (2) | Style / gas; excluded. Loops are bounded (signers ≤ 16, guardians ≤ 16, whitelist ≤ 64, permissions ≤ 32, unbonding entries ≤ 32, validators ≤ 256, candidates ≤ 2,000). |

## 6. Remaining risks for the external audit

Static analysis does not replace the audit planned in §25.2. Items the auditors should look at specifically:

1. **Governance: a Succeeded proposal that was never queued does not expire** — it can be queued (and executed after the
   timelock) at any later time, when the context may have changed. Same as OpenZeppelin Governor; mitigations today:
   proposer `cancel`, and the timelock gives a public window. Consider an expiry for un-queued proposals.
2. **Trust in the node:** `slash` evidence, `recordParticipation` bitmaps, epoch calls and the reward credit are trusted
   from `SYSTEM_ADDRESS` (the EVM cannot verify Ed25519). The Rust side (crates/execution, consensus) is out of scope of
   these tools.
3. **XIP-0001 EOA lock** is enforced by the node's transaction-validity rule; the contracts only write the lock word.
   The rule's implementation and its interaction with EIP-7702 re-delegation need review together (SA-01 closed one
   contract-side bypass).
4. **Vault classification is syntactic** (known selectors); whitelisted targets can be called with any calldata
   (spec smart-accounts §12). Unknown approval-like functions on whitelisted contracts are not recognised.
5. **ERC-4337 / ERC-7562 rules:** the paymasters read sponsor-/sender-keyed storage in validation and must be staked;
   bundler-side simulation compliance was tested with the EntryPoint in Foundry, not with a real bundler.
6. **`XdevsPaymaster.deposit`** truncates `msg.value` with `uint128(...)`; unreachable because the native supply is
   many orders of magnitude below 2¹²⁸ wei, but would need `SafeCast` if the contract were reused elsewhere.
7. **Code upgrades are hard forks** (predeploy code replacement at a fork height); storage layouts must stay
   append-only. Account behaviour also changes when `0x…1005` / `0x…1006` code is replaced.
8. **Staking:** voting power is bonded principal (not post-slash value); rounding dust of rewards stays reserved
   forever (never over-paid); `onEpoch` sorts up to 2,000 candidates in memory (CPU cost of system calls).
9. **Session keys:** NFT approvals / transfers are debited by token id against fungible caps (practically disallowed);
   malformed known selectors are debited at `type(uint256).max`.

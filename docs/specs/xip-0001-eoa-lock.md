# XIP-0001: EOA Lock for EIP-7702 Smart Accounts

| | |
|---|---|
| XIP | 0001 (xDEVS Improvement Proposal) |
| Title | Opt-in protocol-level lock of the EOA key of an EIP-7702 account delegated to the xDEVS Smart Account |
| Status | Draft, accepted for implementation (testnet: active from genesis) |
| Type | Consensus (transaction validity + EIP-7702 authorization processing), mempool, RPC |
| Layers | Account side: `contracts/src/accounts` (done, tested). Node side: crates/ (done, tested: `xdevs_primitives::xip0001`, block execution and building, mempool, RPC `-32003` and `xdevs_getEoaLock`; authorization pre-filter per §9 option 2). |
| Requires | EIP-1559, EIP-2718, EIP-7702; xDEVS Smart Account at `0x0000000000000000000000000000000000001005` |
| Plan refs | §16 (user-safety layer), ADR-0006 (§6), §13 (mempool), §15 (fees), §24.6 (fork activation) |

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

---

## 1. Summary

An EOA that has delegated to the xDEVS Smart Account through EIP-7702 is a smart account *and* still an EOA. Its
private key can always sign raw transactions that never enter the account's code: plain value transfers, direct
calls to tokens, contract creations, and EIP-7702 authorizations that re-delegate or remove the delegation. All of
these bypass the Guardian Vault. A thief who steals the EOA key therefore defeats every account-level protection.

XIP-0001 adds an **opt-in validity rule**. The account writes a lock word into a fixed storage slot. While it is
set, the node accepts only one transaction shape from that address: a **value-0 call to itself**. That call enters
the account's own code, where the vault policy (thresholds, queue, whitelists, daily limits, guardian cancels,
freeze) applies to the EOA key exactly as to any other signer. EIP-7702 authorizations signed by a locked authority
are skipped, so the key can neither re-delegate nor un-delegate. A stricter **sealed** state, which the account sets
while it is guardian-frozen or after a social recovery revoked the EOA key, rejects every transaction from the
address.

The rule never reverts, reorders or otherwise touches anything already included. It only decides whether a *new*
transaction or authorization is valid, from state the account owner wrote with their own key.

---

## 2. Motivation

* `docs/specs/smart-accounts.md` §12 listed this as limitation 3: "EIP-7702 accounts: the EOA key remains
  all-powerful at the protocol level". The only remedy so far was "use a factory account", which means a new address.
  That defeats the reason to use EIP-7702 (keep your address, ENS / XNS name, history, airdrops, ...).
* xDEVS Chain controls its own protocol (ADR-0001), so an opt-in validity rule is cheap: one code comparison and one
  storage read per transaction sender (both already in cache for EIP-7702 senders).
* The account side already treats `msg.sender == tx.origin == address(this)` as the EOA-key signer and routes it
  through the vault. The node rule just makes that route the *only* route.

---

## 3. Definitions

| Name | Value |
|---|---|
| `ACCOUNT_IMPL` | `0x0000000000000000000000000000000000001005` (`Predeploys.SMART_ACCOUNT_IMPL`) |
| `DESIGNATOR` | the 23 bytes `0xef0100 ‖ ACCOUNT_IMPL` = `0xef01000000000000000000000000000000000000001005` |
| `DESIGNATOR_HASH` | `keccak256(DESIGNATOR)` = `0x4879b5ba5d3e15615c1dc61bbc61a36231da1b95582299c433c2edbee1ad7333` |
| `LOCK_SLOT` | `bytes32(uint256(keccak256("xdevs.account.eoaLock")) - 1)` = **`0xfd81d7eef3094fecca95e5e53fe72709c7315b39f65e414992ead9c855e841ce`** |
| `LOCKED` | `1` |
| `SEALED` | `2` |
| `XIP0001_MAX_GAS` | `2_000_000` |
| `XIP0001_FEE_MULTIPLIER` | `2` |
| `XIP0001_MAX_TXS_PER_BLOCK` | `1` |

`keccak256("xdevs.account.eoaLock") = 0xfd81…41cf`; minus one gives the slot. The slot is a plain storage key. It is
**not** ERC-7201-masked and **not** inside a struct, so the node reads it without any ABI knowledge. The same constants are
`AccountConstants.EOA_LOCK_SLOT`, `EOA_LOCKED`, `EOA_SEALED` and `EIP7702_DESIGNATOR_HASH` in
`contracts/src/accounts/libraries/AccountConstants.sol`. `forge script script/accounts/PredeployArtifacts.s.sol`
(accounts profile) prints them.

For an address `A` and a state `S`:

```
word(A, S)      = S.storage[A][LOCK_SLOT]  as U256 (0 if absent)
delegated(A, S) = S.code[A] == DESIGNATOR           // byte-exact, length 23 (equivalently code_hash == DESIGNATOR_HASH)
status(A, S)    = UNLOCKED  if !delegated(A, S) || word(A, S) == 0
                = SEALED    if  delegated(A, S) && word(A, S) == 2
                = LOCKED    otherwise               // word 1, or any other non-zero value
```

Notes:

* `delegated` compares the **account code stored in the state trie** (the EIP-7702 designator), never the result of
  executing EVM opcodes. A delegation to any other address, a cleared delegation, a contract account (factory proxy)
  and an EOA without code are all `UNLOCKED`, whatever the slot holds.
* A non-zero word other than 1 or 2 (e.g. garbage left by a previous delegate) counts as `LOCKED`, not `SEALED`, so
  the EOA can still send the self-call that cleans it (`initialize` clears the slot).
* "Locked sender" below means `status ∈ {LOCKED, SEALED}`.

---

## 4. Activation

* A new optional chain-config field `xip0001Block` (u64, JSON key `"xip0001Block"` in the genesis `config` object,
  next to `hardfork`). The rules in §5 apply to every block with `number >= xip0001Block`. If the field is absent, the
  rules never apply.
* **Testnet (`xdevs-testnet-1`) and devnets: `"xip0001Block": 0`** (active from genesis).
* Any later network enables it at a fork height following §24.6: release the binary before N, validators upgrade,
  and the rule activates at N on all of them at once. Changing any constant of §3 is also a fork.
* Before activation the lock word is ordinary storage. The account contract behaves the same with or without node
  support. Locks written before activation become enforced at `xip0001Block` without any further transaction. Wallets
  SHOULD show "protocol enforcement active" only when the connected chain reports activation (§7, `xdevs_getEoaLock`).

---

## 5. Consensus rules

Let `S₀(tx)` be the state immediately before transaction `tx`. For the first transaction of a block that is the
block's pre-state (after any pre-block system call). Otherwise it is the post-state of the previous transaction in
the same block. **All XIP-0001 checks for `tx` use `S₀(tx)`**, before the sender's nonce increment and fee
deduction (rule d).

A block containing a transaction that is invalid under these rules is **invalid**, exactly like a block containing a
transaction with a bad nonce. Block builders MUST evaluate transactions sequentially and leave out any that are
invalid in their position.

### (a) Sender rule

Let `A = sender(tx)` and `st = status(A, S₀(tx))`.

1. If `st == SEALED`, the transaction is **invalid**.
2. If `st == LOCKED`, the transaction is **invalid** unless **all** of:
   * `tx.to` is present (not a contract creation), and `tx.to == A`;
   * `tx.value == 0`.

Calldata, access lists and authorization lists are unrestricted. The call enters the account's code at `A`
(`msg.sender == tx.origin == A`), and the account enforces its own policy there. This applies to every transaction
type the chain accepts (legacy, EIP-2930, EIP-1559, EIP-7702; blob transactions are rejected anyway).

### (b) EIP-7702 authorization rule

EIP-7702 processes each tuple `[chain_id, address, nonce, y_parity, r, s]` of a set-code transaction in order. If any
step fails, the tuple is **skipped** and processing continues with the next tuple. A failing tuple never invalidates
the transaction. XIP-0001 inserts one step between EIP-7702's step 3 (recover `authority`, check low-s) and step 4
(add `authority` to `accessed_addresses`):

> **3a.** If `status(authority, S₀(tx)) != UNLOCKED`, skip this tuple.

A skipped tuple has **no effect at all**: no code change, no nonce increment, no gas refund, and the authority is not
added to `accessed_addresses` (it is not warmed), the same as a tuple that fails steps 1–3. The intrinsic gas of the
tuple (`PER_EMPTY_ACCOUNT_COST`) is still charged, because intrinsic gas depends only on the list length. Clarifications:

* The check uses `S₀(tx)`, not the state after earlier tuples of the same list. Lock status depends on code *and*
  storage, and storage cannot change during authorization processing. The only possible difference would be a tuple
  that turns an address with a garbage lock word into a `0x…1005` delegation; that address then becomes `LOCKED` from
  the *next* transaction on. This keeps the rule exact for a pre-filter implementation (§9).
* It covers every target: re-delegation to another contract, delegation to `0x0` (clearing the code), and even
  "re-delegation" to `0x…1005` itself (a no-op, still skipped, nonce not bumped).
* It applies whoever sends the transaction (a relayer, a sponsor, the locked EOA itself). This is the important case:
  a thief can sign an authorization off-line and have anyone include it.
* Authorizations with `chain_id == 0` (valid on every chain) are covered like any other.
* The rule never affects tuples of other, unlocked authorities in the same list.

### (c) Fee-drain limits (sender `LOCKED`)

A `LOCKED` sender still pays gas for its self-calls, even when the account code rejects them (for example a thief
calling `execute` after a revoked or rejected action). To bound how fast a thief can burn the victim's balance as
fees, a transaction with `status(sender, S₀) == LOCKED` is **invalid** unless all of:

* **c1 (one per block):** no earlier transaction in the same block has the same sender. (This also covers the
  case where the earlier transaction was not yet subject to the lock, see (d).)
* **c2 (gas cap):** `tx.gas_limit <= XIP0001_MAX_GAS` (2,000,000).
* **c3 (fee cap):** `max_fee_per_gas <= XIP0001_FEE_MULTIPLIER × base_fee_per_gas` of the block being validated
  (legacy / EIP-2930: `gas_price` instead of `max_fee_per_gas`). When local fee markets (§15.2) are active, the base
  is `base_fee_per_gas + local_base_fee(tx.to)`. The usual EIP-1559 requirement `max_fee_per_gas >= base fee` still
  applies, so the valid range is `[base, 2·base]`. `max_priority_fee_per_gas` is not otherwise constrained (it is
  `<= max_fee_per_gas`), so the effective tip is at most one base fee per gas.

`SEALED` senders cannot send anything (a.1), so c1–c3 only matter for `LOCKED`.

**Residual fee-burn bound.** With one transaction per block, at most 2,000,000 gas each, and at most 2 × base fee per
gas:

```
max burn per block = 2,000,000 × 2 × B = 4·10⁶ × B wei
max burn per day   = 43,200 blocks (2 s) × 4·10⁶ × B = 1.728·10¹¹ × B wei
```

| base fee `B` | per block | per day, 2 s blocks | per day, 1 s blocks |
|---|---|---|---|
| 0.1 gwei (protocol minimum, §15.1) | 0.0004 tXDS | **17.28 tXDS** | 34.56 tXDS |
| 1 gwei (genesis initial) | 0.004 tXDS | **172.8 tXDS** | 345.6 tXDS |

* Of each fee, at most half is a tip to the proposer and the rest is base fee (80 % burned, 20 % to the Treasury). A
  thief profits at most from the tip, and only by controlling or bribing proposers. Otherwise this is pure griefing at
  the thief's own cost in time.
* The base fee only stays high if blocks stay more than 50 % full, which the thief would have to pay for separately.
* The bound applies **only while the word is `LOCKED` and the thief holds a still-active EOA key**, that is, from the
  theft until guardians freeze the account or a recovery completes. Both write `SEALED`, after which nothing can be
  sent at all. A user who notices the theft can also cut it to zero instantly by moving funds out through a
  below-threshold or whitelisted vault action.
* ERC-4337 operations signed by the EOA key are not covered by c1–c3 (the bundler is the sender). Inside the
  account they are bounded by the vault's `dailyGasBudget` (smart-accounts.md §7.6).

### (d) State reference and same-block interactions

* Every check uses `S₀(tx)`. A transaction's own effects never change its own validity: a self-call that enables the
  lock, or that is frozen during execution, was already decided valid.
* *Lock enabled earlier in the same block:* if tx *k* (sent by `A`, `A` unlocked at `S₀(k)`) calls
  `enableEoaLock()`, then any later tx *j > k* from `A` in the same block sees `LOCKED`. It is subject to (a), and it
  is invalid by c1 because tx *k* has the same sender. The builder must leave it out. It may become valid in a later
  block if it is a value-0 self-call within c2/c3.
* *Lock enabled by someone else in the same block:* a guardian's `freeze()` (seals a 7702 account), a recovery, or a
  matured queued action executed by anyone can change `A`'s word in tx *k*. Every later transaction from `A` in that
  block is judged against the new word.
* *Lock lifted in the same block:* if tx *k* executes a matured `disableEoaLock` action (word becomes 0), later
  transactions from `A` in the same block are unrestricted.
* *Authorizations* use `S₀` of the transaction that carries them (b).

### (e) Mempool

The pool MUST apply the same predicates on admission, against the current head state, treating the transaction as
if it were the first one in the next block:

1. reject if the sender is `SEALED`;
2. reject if the sender is `LOCKED` and the transaction is not a value-0 call to itself;
3. reject if the sender is `LOCKED` and `gas_limit > 2,000,000`;
4. reject if the sender is `LOCKED` and `max_fee_per_gas > 2 × base fee of the next block`. The base fee moves by
   at most 12.5 % per block, so a transaction priced at about 1.5 × base fee stays includable for several blocks.
   Wallets SHOULD default to 1.5 × base for locked senders.

On every new head the pool MUST re-check pending and queued transactions of senders whose lock status changed, and
evict the ones that became invalid by (a). Transactions that only fail c3 because the base fee moved MAY stay queued
and be retried. For `LOCKED` senders the pool SHOULD keep at most 4 transactions per sender (pending + queued)
instead of the normal 64, and the block builder MUST select at most one transaction per `LOCKED` sender per block
(the lowest nonce). The pool does **not** reject a set-code transaction because one of its tuples has a locked
authority: that tuple is simply skipped at execution (b). Invalid transactions MUST NOT be gossiped.

### (f) RPC behaviour

| Method | Behaviour |
|---|---|
| `eth_sendRawTransaction` | Runs the (e) checks. On failure: JSON-RPC error `-32003` ("transaction rejected") with `data.reason` set to one of `XIP0001_SEALED`, `XIP0001_NOT_SELF_CALL`, `XIP0001_VALUE_NOT_ZERO`, `XIP0001_GAS_LIMIT` or `XIP0001_FEE_CAP`, and a human message such as `"xip-0001: sender 0x… is EOA-locked: only value-0 calls to itself are allowed"`. |
| `eth_estimateGas` | If `from` is locked at the requested block: return the same errors for (a). For a valid shape, estimate normally; if the estimate exceeds 2,000,000, return `XIP0001_GAS_LIMIT` (the transaction could never be included). Fee fields in the request are checked against c3 only if present. |
| `eth_call`, `eth_createAccessList`, `debug_trace*` | **Unchanged.** Calls are simulations, not transactions, and dApps routinely `eth_call` with `from` = the user. They MUST NOT be rejected. |
| `xdevs_simulateTransaction` (§16.4) | Simulates as usual, and adds a `warnings[]` entry `{ "code": "XIP0001_INVALID", "severity": "high", "message": "This transaction can never be included: the sender is EOA-locked" }` when the transaction would fail (a)/(c). |
| `xdevs_getEoaLock(address, blockTag)` (new) | Returns `{ "active": <xip0001 active at that block>, "status": "unlocked" \| "locked" \| "sealed", "word": "0x…", "delegatedTo": "0x…1005" \| null }`. |
| Receipts / logs | Unchanged. The account emits `EoaLockUpdated(uint256 state)` whenever the word changes. Indexers and wallets SHOULD surface it. |

---

## 6. Account side (implemented, `contracts/src/accounts`)

| Function | Who | Effect on the word |
|---|---|---|
| `enableEoaLock()` | owner (the account itself / EOA key, any signer, an owner-signed UserOp); not while frozen | `0 → 1` (or `2` if the EOA key is revoked). **Instant** (on the vault's instant list). Reverts `NotDelegatedEoa` unless `address(this).codehash == DESIGNATOR_HASH` (factory accounts have no private key, so the lock means nothing there), and `VaultDisabled` if the vault is off (the lock would protect nothing). |
| `disableEoaLock()` | owner | `→ 0`. **Delayed** like every policy change (queued for the vault delay, cancellable by any signer or the guardian threshold). Reverts `EoaKeyInactive` while the EOA key is revoked: the lock is what keeps a revoked key from re-delegating. |
| `freeze()` reaching the guardian threshold | guardians | `→ 2` (SEALED) on a 7702 account, even if it was unlocked. A frozen account accepts no EOA-key action, so nothing from the EOA should be includable. |
| `unfreeze()` at threshold / `executeRecovery` | guardians / anyone | re-derived: `2` if the EOA key is revoked, else `1` (if it was locked) |
| `executeRecovery(newSigners)` | anyone after the delay | revokes the EOA key unless `newSigners` lists the account address as an ECDSA signer, which seals the address (`→ 2`) even if it was unlocked |
| `addSigner` / `removeSigner` of the account address | owner | re-derived (adding it back re-activates the key: `2 → 1`; removing it revokes: `→ 2`) |
| `refreshEoaLock()` | **anyone** | re-derives `1`/`2` from the current state, for example `2 → 1` after a freeze expired by itself (an expiry cannot write storage) |
| `initialize` | — | clears the slot (garbage from a previous delegate) |
| views | — | `isEoaLocked()`, `eoaLockState()`, `isEoaKeyActive()` |

The EOA key acts inside the account as `msg.sender == tx.origin == address(this)`, or as a signature recovering to
`address(this)`. After a recovery that key is revoked (`Core.eoaKeyRevoked`) for direct calls, UserOps and ERC-1271.
Internal self-calls made by the account's own dispatcher stay authorized (transient self-auth flag).

---

## 7. Rationale and alternatives

* **Why a raw slot and not a view call?** Validity checks run for every pending transaction. They must be cheap,
  deterministic and free of EVM execution. One code compare plus one `SLOAD`-equivalent read is all that is needed.
* **Why only delegations to `0x…1005`?** The rule trusts the account code to apply a policy to self-calls. Only the
  canonical implementation is known to do that. Arbitrary delegates writing the slot get no protocol effect (vector
  V10).
* **Why allow self-calls at all (LOCKED rather than always SEALED)?** So the owner keeps the convenient "my EOA pays
  its own gas" path for everything the vault allows instantly, and can always reach the account even without a
  bundler or paymaster. SEALED is used only when the EOA key can do nothing useful in the account anyway (frozen or
  revoked). That removes the fee-burn residual exactly when the key is known to be compromised.
* **Why c1–c3 instead of nothing?** Without them, a thief could fill blocks with gas-heavy failing self-calls at an
  absurd fee and turn the victim's whole balance into fees within minutes (the "bundler drain" of smart-accounts.md
  §7.6, at protocol level).
* **Why not a protocol "freeze"?** It would need an admin or governance with power over user funds, which §16.6
  forbids. XIP-0001 gives no one any power over anyone else's account. It only lets an account restrict *its own*
  key.

---

## 8. Why this does not violate ADR-0006

ADR-0006: "Safety features live in accounts and RPC, never in finality. No protocol-level reversals. Guardian Vault
delays are enforced by the account's own code, which the user opts into."

* **No reversal, no finality change.** XIP-0001 is a *validity* rule for transactions not yet included, of the same
  kind as nonce, balance and intrinsic-gas checks. A finalized block is never re-evaluated. Execution semantics
  (EVM, gas schedule, state transition of valid transactions) are unchanged.
* **Opt-in, by the key owner, from account state.** The only input is a storage word that the account's own code
  wrote: through the owner's own call (`enableEoaLock`), or through guardians and a recovery the owner configured
  in advance. No validator, governance or third party can lock an address. Storage at the address can only be
  written by code running in that address's context, which is the canonical implementation.
* **The policy still lives in the account.** The node does not interpret vault policies, limits or whitelists. It
  only routes the EOA key into the account, where the user's opted-in policy is enforced by the account's own code,
  exactly as ADR-0006 requires.
* **No admin key, no seizure (§16.6).** Funds are never immobile because of XIP-0001. The owner can always act
  through the account (self-calls, signers, UserOps), and disabling the lock is a normal, delayed owner action.
* **Risk warnings never block (§16.6).** XIP-0001 is not a risk heuristic. It enforces a rule the user chose.

---

## 9. Implementation notes for the node (non-normative)

* **Where:** (a) and (c) are pure functions of `(tx, S₀, block header, position)` and can run before handing the
  transaction to revm, in the same place as the nonce and balance checks (`crates/execution` block executor;
  `crates/mempool` validation). (b) must take effect inside EIP-7702 authorization processing. Two exact options:
  1. a custom revm handler step that checks each recovered authority before revm's own processing; or
  2. a pre-filter: recover each tuple's authority, and if it is locked at `S₀`, replace the tuple with a canonical
     tuple that fails EIP-7702 step 1 (for example `chain_id = 2^64-1`, which matches neither `0` nor the chain id).
     The list length, and therefore intrinsic gas, is unchanged; the replaced tuple has no effect and does not warm the
     authority. That matches 3a exactly *because* 3a uses `S₀` and sits before step 4.
* **Cost:** for each transaction, read the sender's code hash (already loaded for EIP-3607/7702 sender checks); only
  if it equals `DESIGNATOR_HASH`, read one storage slot. For each authorization tuple, the same check for the
  authority. These are node-side reads and are not charged as EVM gas.
* **Determinism:** compare the raw 23-byte code, or its keccak hash, against the constants. Compare the U256 word
  exactly (`== 0`, `== 2`, otherwise LOCKED).
* **Config:** `ChainConfig` gains `xip0001_block: Option<u64>` (`#[serde(default)]`, JSON `xip0001Block`). Genesis for
  the testnet sets it to 0.

---

## 10. Test vectors (the node MUST pass)

Common setup: `A` is an EOA whose code is `DESIGNATOR` (delegated to `0x…1005`) and whose balance is 100 tXDS. `F` is
another address. Base fee of the block under test is `B = 1 gwei` unless stated. "word" means
`storage[A][0xfd81d7ee…841ce]`. The Solidity reference model of these rules is `_nodeTxValid` / `_nodeAuthApplies` in
`contracts/test/accounts/EoaLock.t.sol` (`test_nodeRule_vectors` and the demos), evaluated against real account
state.

### Transaction validity (rule a / c), single transaction in the block

| # | word | tx | expected |
|---|---|---|---|
| V1 | 0 | `to=F, value=1 tXDS, gas=21000, maxFee=100 gwei` | valid (unlocked: no restriction) |
| V2 | 1 | `to=A, value=0, gas=300000, maxFee=2 gwei, data=execute(F,0.5 tXDS,"")` | **valid** |
| V3 | 1 | `to=A, value=1 wei` | invalid (`XIP0001_VALUE_NOT_ZERO`) |
| V4 | 1 | `to=F, value=0` (e.g. `F` is a token, `transfer(...)`) | invalid (`XIP0001_NOT_SELF_CALL`) |
| V5 | 1 | contract creation (`to` empty) | invalid (`XIP0001_NOT_SELF_CALL`) |
| V7a | 1 | `to=A, value=0, gas=2,000,000, maxFee=2 gwei` | valid |
| V7b | 1 | `to=A, value=0, gas=2,000,001` | invalid (`XIP0001_GAS_LIMIT`) |
| V8a | 1 | `to=A, value=0, maxFee=2,000,000,000 wei` (= 2·B) | valid |
| V8b | 1 | `to=A, value=0, maxFee=2,000,000,001 wei` | invalid (`XIP0001_FEE_CAP`) |
| V8c | 1 | legacy tx `to=A, value=0, gasPrice=2 gwei` | valid; `gasPrice=3 gwei` invalid |
| V13 | 2 | `to=A, value=0, gas=300000, maxFee=2 gwei` (the V2 shape) | invalid (`XIP0001_SEALED`) |
| V14 | `0xdead` (garbage) | V2 shape | valid (garbage counts as LOCKED, not SEALED); V4 shape invalid |
| V10 | 1, but code = `0xef0100 ‖ <other address>` | `to=F, value=1 tXDS` | valid (not delegated to `0x…1005`) |
| V11 | 1, `A` is a contract (factory proxy, real bytecode) | n/a (contracts cannot send transactions) / any authority check | status `UNLOCKED` |
| V15 | 1 | type-4 tx from `A`: `to=A, value=0`, auth list `[ (chain 0, F, nonce(A)+1) signed by A ]` | **tx valid**; the tuple is skipped: `A`'s code stays `DESIGNATOR`, `A`'s nonce is incremented only once (by the tx), and the tuple's authority is not warmed |

### Same block (rule c1 / d)

| # | setup | block content | expected |
|---|---|---|---|
| B1 | word 1 | tx1 from `A` (V2 shape, nonce n), tx2 from `A` (V2 shape, nonce n+1) | tx1 valid, **tx2 invalid** (c1). A block containing both is invalid; a builder includes tx1 only. |
| B2 | word 0 | tx1 from `A`: `to=A, value=0, data=enableEoaLock()`; tx2 from `A`: `to=F, value=1 tXDS` | tx1 valid (unlocked at `S₀`); after tx1 the word is 1; **tx2 invalid** (a) |
| B3 | word 0 | tx1 from `A`: `enableEoaLock()` self-call; tx2 from `A`: V2 shape | tx1 valid; **tx2 invalid** (c1, an earlier tx from `A` is in the block) |
| B4 | word 1 | tx1 from guardian `G1` = second `freeze()` vote (threshold reached → word 2); tx2 from `A`: V2 shape | tx1 valid; **tx2 invalid** (SEALED at `S₀(tx2)`) |
| B5 | word 1, a matured queued `disableEoaLock` action `id` | tx1 from anyone: `A.execute(id)` (word → 0); tx2 from `A`: `to=F, value=1 tXDS` | both valid (tx2 sees UNLOCKED) |
| B6 | word 1, a recovery ready | tx1 from anyone: `executeRecovery(newSigners)` without `A` (word → 2); tx2 from `A`: V2 shape | tx1 valid; **tx2 invalid** (SEALED) |

### Authorizations (rule b)

| # | setup | tx | expected |
|---|---|---|---|
| A1 | word 1 | relayer `R` sends a type-4 tx, `to=R2, value=0`, list `[ (chainId, X, nonce(A)) signed by A ]`, where `X` is any contract | tx valid; tuple **skipped**: `A`'s code stays `DESIGNATOR`, `A`'s nonce unchanged, no refund, `A` not warmed |
| A2 | word 1 | same, but the delegate is `0x0000…0000` (clear delegation) | tuple skipped; `A` keeps its code |
| A3 | word 1 | same, with `chain_id = 0` | tuple skipped |
| A4 | word 2 | same as A1 | tuple skipped |
| A5 | word 0 | same as A1 | tuple applied normally (`A` delegated to `X`, nonce + 1) |
| A6 | word 1 for `A`, `C` unlocked | list `[ tuple by A → X, tuple by C → 0x…1005 ]` | first skipped, second applied |
| A7 | `D`: code `0xef0100 ‖ X'` (other delegate), slot = 1 (garbage) | list `[ D → 0x…1005, D → X'' ]` (valid nonces n, n+1) | both evaluated against `S₀` where `D` is UNLOCKED: both applied; final code `0xef0100 ‖ X''`, status UNLOCKED |

### Activation

| # | setup | expected |
|---|---|---|
| F1 | `xip0001Block = 100`, word 1 | block 99: V4 shape valid; block 100: V4 shape invalid |
| F2 | `xip0001Block` absent | all rules inactive at every height |

### Mempool / RPC

| # | request | expected |
|---|---|---|
| M1 | `eth_sendRawTransaction` with V4 while word 1 | error `-32003`, `data.reason = "XIP0001_NOT_SELF_CALL"` |
| M2 | `eth_sendRawTransaction` with V2 while word 2 | error `-32003`, `XIP0001_SEALED` |
| M3 | `eth_estimateGas {from: A, to: F, value: 1}` while word 1 | error `XIP0001_NOT_SELF_CALL` |
| M4 | `eth_estimateGas {from: A, to: A, data: execute(...)}` while word 1 | normal estimate (≤ 2,000,000) |
| M5 | `eth_call {from: A, to: F, data: balanceOf(A)}` while word 2 | executes normally |
| M6 | pool holds V2 from `A` (word 1); a new head contains a `freeze()` that seals `A` | the pending tx is evicted |

---

## 11. Security considerations and residual risk

1. **Window before the lock.** The lock only protects from the moment it is enabled (and from `xip0001Block` at the
   protocol level). Wallets SHOULD enable it by default for 7702 accounts with the vault on.
2. **Bounded fee burn while LOCKED** (§5 c): at most 17.28 tXDS/day at the minimum base fee with 2 s blocks, scaling
   linearly with the base fee and the block rate. It ends when the account is frozen or recovered (SEALED) or emptied.
3. **Signatures verified with plain `ecrecover`.** Contracts that accept a raw ECDSA signature from `A` without
   checking for code / ERC-1271 (some EIP-2612 `permit(v,r,s)` and EIP-3009 implementations) can still be authorized
   by the EOA key off-chain. The protocol cannot see those. Contracts using OpenZeppelin `SignatureChecker` (code
   present ⇒ ERC-1271), Permit2 and USDC-style checks are safe, because the account's ERC-1271 refuses signatures
   that bypass the vault. The former instance in this repo, `XDRC20.permit(owner, spender, value, deadline, v, r, s)`
   (`contracts/src/periphery/token/XDRC20.sol`), now goes through `SignatureChecker` as well: owners with code are
   checked by ERC-1271 only (tests: `test_eip2612PermitEoaKeyCannotBypassLocked7702Account` in `XDRC20.t.sol`,
   `test_permitEoaKeyCannotBypassLocked7702Account` in `Permit2Expiring.t.sol`). Third-party tokens on the chain
   can still have the raw-`ecrecover` pattern.
4. **SEALED after a freeze expires by itself** stays SEALED until anyone calls `refreshEoaLock()`, because an expiry
   writes no storage. A legitimate EOA-key owner can trigger it through another signer, a UserOp, or any third party.
   It is never a loss of access (UserOps and signers still work).
5. **Guardians become safety-critical for 7702 accounts.** A guardian threshold can freeze (which seals) and then
   recover. That is the intended social-recovery trust model (see smart-accounts.md §8.1).
6. **Other chains.** The lock exists only on xDEVS Chain. The same key on other EVM chains is unaffected.

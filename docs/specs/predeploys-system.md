# System predeploys — Staking, Governance, Treasury, FeeConfig, WXDS

Status: implemented (Phase 3 scope) · Sources: `contracts/src/system/` · Tests: `contracts/test/system/`
Plan references: §11.4–11.8 (PoS), §15.1 (fees), §18 (system contracts), §20 (tokenomics), §21 (genesis),
§25.1 (threat model), §33.1 (parameters).

Build / test: `cd contracts && FOUNDRY_PROFILE=system forge test` (the `system` profile compiles only
`src/system` + `test/system`; artifacts land in `contracts/out/system/`).

## 0. Cross-cutting rules

| Rule | Detail |
|---|---|
| Addresses | Fixed, from `contracts/src/common/Predeploys.sol`: Staking `0x…1000`, Governance `0x…1001`, Treasury `0x…1002`, FeeConfig `0x…1003`, WXDS `0x…1009`. |
| System caller | `SYSTEM_ADDRESS = 0xffffFFFfFFffffffffffffffFfFFFfffFFFfFFfE`. Protocol calls use it as `msg.sender`, no signature, no gas charge, not counted in `gas_used` (§18.2). |
| Genesis | Contracts are written into genesis `alloc` as **runtime** bytecode (`deployedBytecode` in `contracts/out/system/<C>.sol/<C>.json`). Constructors never run at the final address and are empty. No `immutable`s, nothing caches `address(this)`. |
| Initialization | Each contract (except WXDS) has a one-time `initialize(...)`, callable **only by `SYSTEM_ADDRESS`**, and only once. The genesis builder executes it in its in-memory EVM with `caller = SYSTEM_ADDRESS` and dumps the resulting storage into `alloc[addr].storage`. Reference procedure: `contracts/script/system/SystemGenesis.s.sol`. |
| Init order | 1) set balances (Staking = Σ genesis stakes, Treasury = 200M tXDS) 2) `Staking.initialize` 3) `Governance.initialize` 4) `Treasury.initialize` 5) `FeeConfig.initialize`. WXDS needs code only. |
| Upgrades | **Not proxies.** Upgrades are done by replacing the code at the predeploy address at a fork height (§24.6), keeping the storage layout append-only. This deviates from the "Upgradeable only via Governance + timelock" sentence of §18: code changes are a hard fork (validators must upgrade binaries), while **every parameter change** is still gated by Governance (vote + timelock). |
| Reentrancy | Native value leaves contracts only through pull functions (`withdrawUnbonded`, `claimRewards`, `Treasury.grant`, `WXDS.withdraw`, `Governance.execute`) using checks-effects-interactions; Staking/Treasury/Governance also use OpenZeppelin `ReentrancyGuardTransient` (EIP-1153 transient storage, so it adds **no storage slot** and needs no initialization). |
| Solidity | `^0.8.24`, compiled with solc 0.8.38, `evm_version = prague`, optimizer 200 runs, `bytecode_hash = none`, `cbor_metadata = false` (deterministic runtime bytecode). |

---

## 1. Staking — `0x0000000000000000000000000000000000001000`

Validator registry, delegation, rewards, slashing and the consensus validator set (§11.4–11.6).
Interface: `contracts/src/system/interfaces/IStaking.sol` (the §18.1 sketch is kept verbatim, extensions are additive;
`recordParticipation` is `payable`, which does not change its selector).

### 1.1 Lifecycle (§11.4)

```
[*] ──register(key, commission) + selfStake ≥ minSelfStake──▶ Candidate
Candidate ──in top-N at onEpoch──▶ Active ──dropped from top-N at onEpoch──▶ Candidate
Active ──downtime (window end)──▶ Jailed ──unjail() after jailedUntil, selfStake ≥ min──▶ Candidate
any registered state ──double-sign evidence──▶ Tombstoned   (permanent, never eligible again)
Candidate / Active / Jailed ──operator undelegates ALL self-stake──▶ Exited  (= diagram "Unbonding → [*]")
```

* Validator id = operator address (`msg.sender` of `register`). One registration per address, ever.
* Consensus key = 32-byte Ed25519 public key, unique, permanently bound (even after Exit/Tombstone).
* Operator self-stake = value of the operator's own delegation shares. Partial self-undelegation that would leave
  `0 < selfStake < minSelfStake` reverts (`SelfStakeBelowMinimum`); undelegating everything exits the validator.
  A slash can push self-stake below the minimum: the validator is then not eligible at the next epoch and cannot
  `unjail()` until the operator tops up with `delegate(operator)`.
* `Exited` and `Tombstoned` validators accept no new delegations; delegators can always undelegate.
* The **active set changes at epoch boundaries** (`onEpoch`) **and** immediately when an active validator is jailed,
  tombstoned or exits (order of the remaining members is preserved; `setVersion` increments and
  `ValidatorSetUpdated` is emitted). If removing a tombstoned validator would empty the set, the set is refilled from
  eligible candidates on the spot. Safety guards: downtime never jails the last active validator, and the last
  active validator cannot exit (`LastActiveValidator`). The set can only become empty if the last validator is
  tombstoned and nobody is eligible — the chain then halts safely (§11.1).

### 1.2 Active-set selection (deterministic)

At `onEpoch` (and genesis): eligible = status `Candidate` or `Active`, `totalStake > 0`, `selfStake ≥ minSelfStake`.
Order = bonded stake **descending**, ties broken by operator address **ascending** (numeric). Top `maxValidators`
are kept (bounded partial insertion sort, O(candidates × N), `maxCandidates ≤ 2000`, `maxValidators ≤ 256`).
If nobody is eligible the current set is kept. Power = bonded stake in **wei** at selection time
(frozen until the next change). The node converts it to CometBFT/Malachite voting power as `power / 1e18`
(whole tXDS, fits in i64), which is what `genesis.json` `validators[].power` uses.

`validatorSetHash = keccak256(abi.encode(address[] operators, bytes32[] consensusKeys, uint256[] powers))`
(standard ABI encoding of three dynamic arrays, same order as `consensusValidatorSet()`).

### 1.3 System-call ABI (msg.sender = SYSTEM_ADDRESS)

Per block, the node executes system calls **before user transactions** in this order:

| # | Call | When | Selector |
|---|---|---|---|
| 1 | `recordParticipation(address proposer, uint256 signedBitmap)` | every block ≥ 1 | `0x671a8af4` |
| 2 | `slash(bytes evidence)` | once per evidence item included in the block | `0x5e47655f` |
| 3 | `onEpoch()` | when `block.number % epochLength == 0` (epochLength = 3600, a fork parameter of the node) | `0xc5242beb` |

Read-only helpers for the node: `consensusValidatorSet()` `0x3c32d66c` → `(address[] operators, bytes32[] keys,
uint256[] powers)`, `signingSet()` `0x39c47408`, `activeValidators()` `0xd2d49c4b`, `validatorSetHash()` `0xcdea2912`,
`setVersion()` `0x0e15932b`, `currentEpoch()` `0x76671808`.
Genesis: `initialize(address,(uint256,uint256,uint64,uint64,uint64,uint64,uint16,uint16,uint16,uint16,uint16,uint16,uint16,uint16),(address,bytes32,uint16,uint256)[])` `0xdd74dafd`.

* **`recordParticipation`** — must be called exactly once per block, first (reverts `AlreadyCalledThisBlock` on a
  second call in the same block). `proposer` = operator address of the block's proposer. Bit `i` of
  `signedBitmap` (LSB = index 0) = "member `i` of the **signing set** has a precommit in the `LastCommit` included in
  this block". The signing set is returned by `signingSet()` evaluated in the current block: it is the set that was in
  force when the previous block was signed, i.e. the pre-change set if the active set changed in the previous block
  (`lastSetChangeBlock == block.number - 1`), otherwise the current active set. The contract then:
  1. updates per-validator liveness counters for the current window,
  2. distributes rewards (below),
  3. at the end of each `downtimeWindow`-block window, jails + slashes every **active** validator that signed less
     than `minSignedBps` of the blocks it was expected to sign in that window (tumbling window; validators with no
     expected blocks in the window are skipped).
* **`slash(bytes evidence)`** — `evidence = abi.encode(uint256 kind, address validator, uint64 infractionHeight,
  bytes32 ref)` (exactly 128 bytes). `kind = 1` (double-sign) is the only kind. The **node verifies** the two
  conflicting signed votes (same height/round, different block hash, valid Ed25519 signatures by the validator's
  consensus key, within the evidence max-age which must be shorter than the unbonding period) before calling;
  `ref` is the hash of the evidence object for traceability. Malformed, duplicate (same bytes), unknown-kind,
  unknown-validator or already-tombstoned evidence is **ignored with `EvidenceIgnored(hash, code)`, never reverts**,
  so block execution cannot fail because of evidence. Effect: slash `doubleSignSlashBps` of the validator's bonded
  stake **and** of its unbonding pool, status → `Tombstoned`, removed from candidates and from the active set.
* **`onEpoch()`** — increments `currentEpoch`, recomputes the set, emits `EpochTransition(epoch, validatorSetHash)`.
  Reverts if called twice in one block. The new set is used for blocks `> block.number`
  (the consensus layer reads it via `ConsensusApp::validator_set(height)` after executing the epoch block).

A system call that reverts indicates a node bug (wrong order / duplicate call); the node should treat it as fatal.

### 1.4 How rewards are funded

Rewards = **native balance not accounted for**: `reward = address(Staking).balance − accountedBalance`, distributed
inside `recordParticipation`. The node funds the block reward (inflation: 7% year 1, −1%/year, floor 2%, §20.1,
computed by the node from the block height) in one of two equivalent ways, **before** call #1 of the block:

* **Preferred: direct balance credit** — increase the Staking account balance in the state by the block reward (the
  same mechanism as the 20% base-fee credit to the Treasury). No call, no event.
* Alternative: perform `recordParticipation` with `value = blockReward` (the node must first credit the reward to
  `SYSTEM_ADDRESS` so the value transfer succeeds).

Priority fees (tips) are **not** routed through Staking: they go to `block.coinbase` (the proposer's fee recipient)
as in Ethereum (§15.1). Any accidental donation to Staking (e.g. SELFDESTRUCT) is distributed as reward too.
If no rewardable validator signed (`signedPower == 0`) the reward stays unaccounted and is paid in a later block.

Split of `reward` R in a block:
1. Proposer bonus `R × proposerBonusBps / 10000` (5%) to the proposer — if it is a registered, non-tombstoned
   validator with stake; otherwise the bonus stays in the signer pool.
2. The rest is split among signers (bits set in the bitmap, excluding tombstoned validators) pro-rata to their
   **set power**.
3. For each credited validator: `commissionBps` goes to the operator (`commissionAccrued`), the remainder to all
   delegators of that validator (operator's self-delegation included) pro-rata to shares via an accumulator
   (`accRewardPerShare`, 1e30 precision; debt rounded up, accrual rounded down so payouts never exceed credits).
Rewards are claimed with `claimRewards(validator)` (the operator also receives its commission there).
They are **not** auto-compounded.

### 1.5 Delegation, unbonding, slashing accounting

* Delegations are **shares** of a validator's bonded stake; slashing reduces `totalStake` without touching shares
  (O(1), pro-rata).
* `undelegate(validator, amount)` (`amount = type(uint256).max` = everything) moves stake into the validator's
  **unbonding pool** (also share-based) and creates an entry maturing after `unbondingSeconds`
  (max 32 pending entries per delegator). A double-sign slash also cuts the unbonding pool, so stake that started
  unbonding before the evidence arrived is still slashable (all entries in the pool are cut pro-rata — documented
  over-approximation: entries created before the infraction height are slashed too).
* `withdrawUnbonded()` pays all matured entries.
* Slashed stake is **locked forever** in the contract (`totalSlashed`; economically burned).
* Accounting invariant (fuzzed): `accountedBalance == totalBonded + totalUnbonding + totalRewardsReserved + totalSlashed ≤ balance`.

### 1.6 Governance voting power

`getVotes(account)` / `getPastVotes(account, blockNumber)` / `getPastTotalVotes(blockNumber)` = bonded **principal**
(own stake + delegations), checkpointed per block (OpenZeppelin `Checkpoints.Trace208`). Undelegation removes
principal pro-rata to the shares burned. Limitation: slashing is not reflected in voting power until the account's
next stake change (principal ≠ current value after a slash).

### 1.7 Parameters (`setParams(Params)`, only Governance)

| Field | Testnet default | Bounds |
|---|---|---|
| minSelfStake | 10,000 tXDS | > 0 |
| minDelegation | 1 tXDS | > 0 |
| unbondingSeconds | 86,400 (1 day) | 1 h … 60 days |
| commissionChangeInterval | 86,400 | ≤ 30 days |
| downtimeWindow | 10,000 blocks | 1 … 1,000,000 |
| downtimeJailSeconds | 600 (10 min) | ≤ 30 days |
| maxValidators | 100 | 1 … 256 (bitmap width) |
| maxCandidates | 1,000 | maxValidators … 2,000 |
| maxCommissionBps | 2,000 (20%) | ≤ 10,000 |
| maxCommissionChangeBps | 100 (1% per interval) | ≤ 10,000 |
| proposerBonusBps | 500 (5%) | ≤ 10,000 |
| doubleSignSlashBps | 500 (5%) | ≤ 5,000 |
| downtimeSlashBps | 1 (0.01%) | ≤ 5,000 |
| minSignedBps | 5,000 (50%) | ≤ 10,000 |

Epoch length (3,600 blocks) is **not** stored in the contract: it is a fork parameter of the node (§33.1), which
decides when to call `onEpoch()`.

### 1.8 `initialize(address governance, Params params, GenesisValidator[] validators)`

`GenesisValidator = (address operator, bytes32 consensusKey, uint16 commissionBps, uint256 stake)`.
Requirements: caller `SYSTEM_ADDRESS`; ≥ 1 validator; unique non-zero operators and keys; `stake ≥ minSelfStake`;
commission ≤ max; Σ stake ≤ contract balance (excess balance = undistributed reward paid at block 1). Each genesis
validator is registered with its stake self-delegated; the epoch-0 set is computed with the same selection rule and
`EpochTransition(0, hash)` is emitted. Genesis `validators[]` in `genesis.json` must match `consensusValidatorSet()`
after initialization (the builder should assert this).

### 1.9 Node-facing reads

Recommended: `eth_call`-style local calls (the node embeds revm) to `consensusValidatorSet()`,
`signingSet()`, `validatorSetHash()`, `setVersion()`, `currentEpoch()`. Raw-slot alternative (pinned by
`test_storageLayout_nodeSlots`):

| Slot | Content |
|---|---|
| 0 | `address[] _activeOperators` — length at slot 0, element i at `keccak256(uint256(0)) + i` |
| 1 | `bytes32[] _activeKeys` — length at slot 1, element i at `keccak256(uint256(1)) + i` |
| 2 | `uint256[] _activePowers` — length at slot 2, element i at `keccak256(uint256(2)) + i` |
| 3 | packed: bits 0–63 `currentEpoch`, 64–127 `setVersion`, 128–191 `lastSetChangeBlock`, 192–255 `lastEpochBlock` |
| 4 | `validatorSetHash` |
| 5, 6 | previous set (`_prevOperators`, `_prevPowers`) — the signing set when `lastSetChangeBlock == block.number − 1` |

### 1.10 Storage layout (`forge inspect Staking storage-layout`)

| Slot | Offset | Bytes | Name | Type | Notes |
|---|---|---|---|---|---|
| 0 | 0 | 32 | `_activeOperators` | `address[]` |  |
| 1 | 0 | 32 | `_activeKeys` | `bytes32[]` |  |
| 2 | 0 | 32 | `_activePowers` | `uint256[]` |  |
| 3 | 0 | 8 | `currentEpoch` | `uint64` |  |
| 3 | 8 | 8 | `setVersion` | `uint64` |  |
| 3 | 16 | 8 | `lastSetChangeBlock` | `uint64` |  |
| 3 | 24 | 8 | `lastEpochBlock` | `uint64` |  |
| 4 | 0 | 32 | `validatorSetHash` | `bytes32` |  |
| 5 | 0 | 32 | `_prevOperators` | `address[]` |  |
| 6 | 0 | 32 | `_prevPowers` | `uint256[]` |  |
| 7 | 0 | 20 | `governance` | `address` |  |
| 7 | 20 | 1 | `initialized` | `bool` |  |
| 8 | 0 | 128 | `_params` | `struct IStaking.Params` | 4 slots: minSelfStake, minDelegation, packed 4×u64 (unbondingSeconds, commissionChangeInterval, downtimeWindow, downtimeJailSeconds), packed 8×u16 (maxValidators … minSignedBps) |
| 12 | 0 | 32 | `accountedBalance` | `uint256` |  |
| 13 | 0 | 32 | `totalBonded` | `uint256` |  |
| 14 | 0 | 32 | `totalUnbonding` | `uint256` |  |
| 15 | 0 | 32 | `totalRewardsReserved` | `uint256` |  |
| 16 | 0 | 32 | `totalSlashed` | `uint256` |  |
| 17 | 0 | 8 | `windowId` | `uint64` |  |
| 17 | 8 | 8 | `windowBlocks` | `uint64` |  |
| 17 | 16 | 8 | `lastParticipationBlock` | `uint64` |  |
| 18 | 0 | 32 | `_candidates` | `address[]` |  |
| 19 | 0 | 32 | `_validators` | `mapping(address => struct Staking.Validator)` | struct Validator (9 slots): +0 consensusKey, +1 totalStake, +2 totalShares, +3 unbondingStake, +4 unbondingShares, +5 accRewardPerShare, +6 commissionAccrued, +7 packed: status u8 @0, commissionBps u16 @1, jailedUntil u64 @3, lastCommissionChange u64 @11, candidateIndex u32 @19, windowId u64 @23; +8 packed: windowExpected u64 @0, windowSigned u64 @8 |
| 20 | 0 | 32 | `_delegations` | `mapping(address => mapping(address => struct Staking.Delegation))` | struct Delegation (4 slots): shares, principal, rewardDebt, rewardsPending |
| 21 | 0 | 32 | `_unbonding` | `mapping(address => struct IStaking.UnbondingEntry[])` | UnbondingEntry (2 slots): +0 packed: validator @0, completionTime u64 @20; +1 poolShares |
| 22 | 0 | 32 | `consensusKeyOwner` | `mapping(bytes32 => address)` |  |
| 23 | 0 | 32 | `evidenceProcessed` | `mapping(bytes32 => bool)` |  |
| 24 | 0 | 32 | `_votes` | `mapping(address => struct Checkpoints.Trace208)` | OZ Checkpoints.Trace208 (array of {uint48 block, uint208 value}) |
| 25 | 0 | 32 | `_totalVotes` | `struct Checkpoints.Trace208` |  |

---

## 2. Governance — `0x0000000000000000000000000000000000001001`

Stake-weighted proposals with a timelock. Governance is the **only** account allowed to change FeeConfig, Staking
parameters and to spend the Treasury.

* A proposal = up to 16 calls `(target, value, calldata)` + description. Proposer needs voting power
  ≥ `proposalThreshold` at the snapshot.
* Snapshot = `block.number − 1` at creation; voting power = `Staking.getPastVotes(voter, snapshot)`.
  `quorumVotes = getPastTotalVotes(snapshot) × quorumBps / 10000` and `thresholdBps` are frozen in the proposal.
* Voting starts immediately and lasts `votingPeriod`. Options: 0 Against, 1 For, 2 Abstain.
* Succeeded ⇔ `for > 0` ∧ `for + abstain ≥ quorumVotes` ∧ `for × 10000 > (for + against) × thresholdBps`.
* `queue(id)` (anyone, after voting) sets `eta = now + timelockDelay`; `execute(id)` (anyone, payable) only when
  `eta ≤ now ≤ eta + gracePeriod`; calls run in order, any failure reverts the whole execution (`CallFailed(i, ret)`),
  the proposal stays Queued and can be retried. After the grace period it is Expired.
* `cancel(id)`: proposer only, while Active / Succeeded / Queued.
* **Emergency pause (§25.1):** the `guardian` can `pause()` which blocks **only new proposals** for
  `pauseDuration`; voting, queueing, execution and cancel keep working. A new pause is only possible
  `pauseDuration` after the previous one ended (guaranteed window for proposals, e.g. to replace the guardian).
  `unpause()`: guardian or Governance itself. `guardian = 0` disables pausing.
* Self-administration through proposals only (`msg.sender == address(this)`): `setParams(Params)`,
  `setGuardian(address)`.

States: `Active, Defeated, Succeeded, Queued, Expired, Executed, Canceled`.

`initialize(address staking, address guardian, Params params)` — caller `SYSTEM_ADDRESS`, once.

| Param | Testnet default | Bounds |
|---|---|---|
| votingPeriod | 3 days | 1 h … 30 days |
| timelockDelay | 1 day | 1 h … 30 days |
| gracePeriod | 14 days | 1 … 60 days |
| pauseDuration | 3 days | ≤ 14 days |
| quorumBps | 3,340 (33.4% of total bonded principal) | 1 … 10,000 |
| thresholdBps | 5,000 (simple majority of for+against) | 5,000 … 9,999 |
| proposalThreshold | 10,000 tXDS | any |

### 2.1 Storage layout

| Slot | Offset | Bytes | Name | Type | Notes |
|---|---|---|---|---|---|
| 0 | 0 | 20 | `staking` | `address` |  |
| 0 | 20 | 1 | `initialized` | `bool` |  |
| 1 | 0 | 20 | `guardian` | `address` |  |
| 1 | 20 | 8 | `pausedUntil` | `uint64` |  |
| 2 | 0 | 8 | `nextPauseAllowedAt` | `uint64` |  |
| 3 | 0 | 96 | `_params` | `struct IGovernance.Params` | 3 slots: packed 4×u64 (votingPeriod, timelockDelay, gracePeriod, pauseDuration), packed quorumBps, thresholdBps (u16), proposalThreshold |
| 6 | 0 | 32 | `proposalCount` | `uint256` |  |
| 7 | 0 | 32 | `_proposals` | `mapping(uint256 => struct Governance.Proposal)` | struct Proposal: +0 packed: proposer @0, snapshotBlock u48 @20, thresholdBps u16 @26, executed @28, canceled @29; +1 packed: voteEnd u64 @0, eta u64 @8, +2 quorumVotes, +3 forVotes, +4 againstVotes, +5 abstainVotes, +6 targets[], +7 values[], +8 calldatas[] |
| 8 | 0 | 32 | `hasVoted` | `mapping(uint256 => mapping(address => bool))` |  |

---

## 3. Treasury — `0x0000000000000000000000000000000000001002`

Holds native tXDS (genesis 200M + 20% of every block's base fees). **The node credits the base-fee share by directly
increasing the account balance (no call, no event)**, so the contract never relies on `receive()`: the spendable
amount is always `address(this).balance`. `receive()` exists only for voluntary deposits (emits `Deposited`).

* `grant(address payable to, uint256 amount, string memo)` — only Governance; emits `Granted`; native transfer with
  `call` (non-reentrant); reverts on zero recipient, insufficient balance or failed transfer.
* `grantToken(IERC20 token, address to, uint256 amount, string memo)` — only Governance, for ERC-20s held by the
  Treasury (SafeERC20).
* `initialize(address governance)` — caller `SYSTEM_ADDRESS`, once. Before initialization nobody can spend.
* `totalGranted` counts native grants.

### 3.1 Storage layout

| Slot | Offset | Bytes | Name | Type | Notes |
|---|---|---|---|---|---|
| 0 | 0 | 20 | `governance` | `address` |  |
| 1 | 0 | 32 | `totalGranted` | `uint256` |  |

---

## 4. FeeConfig — `0x0000000000000000000000000000000000001003`

Governance-controlled fee parameters, read by the node **by raw storage slot** at the start of every block from the
parent state (a change executed in block N applies from block N+1). Every value is a full 32-byte big-endian
`uint256` in its own slot (governance address in the low 20 bytes). Layout is append-only.

| Slot | Name | Testnet default | Bounds (enforced on write) |
|---|---|---|---|
| 0 | governance | `0x…1001` | — |
| 1 | blockGasLimit | 30,000,000 | 1,000,000 … 1,000,000,000 |
| 2 | minBaseFee (wei) | 100,000,000 (0.1 gwei) | 1 … 1,000 gwei |
| 3 | burnBps | 8,000 (rest → Treasury) | ≤ 10,000 |
| 4 | baseFeeMaxChangeDenominator | 8 (±12.5%) | 1 … 1,000 |
| 5 | elasticityMultiplier | 2 | 1 … 100 |
| 6 | maxTxSizeBytes | 131,072 | 1,024 … 4 MiB |
| 7 | localFeeEnabled | 0 | 0 / 1 |
| 8 | localFeeThresholdBps | 2,500 | 1 … 10,000 |
| 9 | localFeeMaxChangeDenominator | 8 | 1 … 1,000 |
| 10 | version | 1 after init, +1 per change (0 = uninitialized → node uses `ChainConfig` defaults) | — |

* `initialize(address governance, FeeParams params)` — caller `SYSTEM_ADDRESS`, once (the builder may equally write
  the slots directly, as long as slot 10 ≠ 0).
* `setParams(FeeParams)` — only Governance; replaces all values, `version++`, emits `ParamsUpdated(version, params)`.
* Note: §33.1 lists "base fee max change" as a *fork* parameter. It is stored here as requested; the node may
  choose to honour slot 4/5 only from a fork height. `epochLength` and block time are not in FeeConfig.

### 4.1 Storage layout

| Slot | Offset | Bytes | Name | Type | Notes |
|---|---|---|---|---|---|
| 0 | 0 | 20 | `governance` | `address` |  |
| 1 | 0 | 32 | `blockGasLimit` | `uint256` |  |
| 2 | 0 | 32 | `minBaseFee` | `uint256` |  |
| 3 | 0 | 32 | `burnBps` | `uint256` |  |
| 4 | 0 | 32 | `baseFeeMaxChangeDenominator` | `uint256` |  |
| 5 | 0 | 32 | `elasticityMultiplier` | `uint256` |  |
| 6 | 0 | 32 | `maxTxSizeBytes` | `uint256` |  |
| 7 | 0 | 32 | `localFeeEnabled` | `uint256` |  |
| 8 | 0 | 32 | `localFeeThresholdBps` | `uint256` |  |
| 9 | 0 | 32 | `localFeeMaxChangeDenominator` | `uint256` |  |
| 10 | 0 | 32 | `version` | `uint256` |  |

---

## 5. WXDS — `0x0000000000000000000000000000000000001009`

WETH9-style wrapped native coin: name "Wrapped XDS", symbol "WXDS", 18 decimals (constants, no storage).
`deposit()` / `receive()` mint 1:1, `withdraw(wad)` burns and sends native via `call` (CEI; WETH9 used `transfer`,
which breaks smart-account recipients). `totalSupply() = address(this).balance` (as WETH9; force-sent coins are
unclaimable surplus). Allowance `type(uint256).max` is never decreased. No `initialize`: genesis needs only code.

### 5.1 Storage layout

| Slot | Offset | Bytes | Name | Type | Notes |
|---|---|---|---|---|---|
| 0 | 0 | 32 | `balanceOf` | `mapping(address => uint256)` |  |
| 1 | 0 | 32 | `allowance` | `mapping(address => mapping(address => uint256))` |  |

---

## 6. Test & verification summary

`FOUNDRY_PROFILE=system forge test` → **124 tests passed, 0 failed** (13 suites; Foundry counts each invariant
contract as one entry). Coverage (`FOUNDRY_PROFILE=system forge coverage`): **100% lines** on Staking, Governance,
Treasury, FeeConfig and WXDS; Staking branches 97.6% (3 uncovered branches are defensive guards that cannot be
reached: zero-share delegation, zero-amount full undelegation, removal of a validator absent from the set).

| File | What it covers |
|---|---|
| `test/system/Staking.t.sol` | every Staking function and revert path, genesis init, rewards math, commission rules, jail/unjail, tombstone, set refill, signing-set switch after a set change, raw node slots 0–4 |
| `test/system/StakingFuzz.t.sol` | onlySystem for any caller, delegate/undelegate round trip, reward conservation, top-N sorted + complete, commission bounds, slash amounts |
| `test/system/StakingInvariant.t.sol` (+ `handlers/StakingHandler.sol`) | accounting identity; total staked == Σ validator stakes and total unbonding == Σ pools; Σ shares == totalShares; rewards solvent (Σ pending ≤ reserved); votes == Σ principal; tombstoned never active/candidate; active set ≤ maxValidators, strictly sorted, statuses/hash consistent; set never emptied while someone is eligible; only SYSTEM_ADDRESS can call system functions |
| `test/system/Governance.t.sol` | full lifecycle, quorum/threshold, snapshot semantics, timelock, grace/expiry, failing calls, cancel, pause/cooldown, self-admin, end-to-end FeeConfig change / Treasury grant / Staking params |
| `test/system/GovernanceInvariant.t.sol` (+ `handlers/GovernanceHandler.sol`) | an action never runs before `eta`, never twice, never without queue; votes ≤ snapshot total power |
| `test/system/TreasuryFeeConfigWXDS.t.sol` | Treasury (incl. balance credit without call), FeeConfig (incl. raw slot layout pin), WXDS (incl. reentrancy) |

The invariant suites were also run with 512 runs × depth 100 (and fuzz at 2,000 runs): all pass. That run found and
led to fixing a 1-wei reward-rounding overshoot (see §1.4).

## 7. Known limitations / open items

1. **Not proxies / code upgrades are hard forks.** §18 says "upgradeable only via Governance + timelock"; the current
   decision is fork-height code replacement. Storage layouts must stay append-only across such replacements.
2. **Evidence is trusted from the node.** The EVM cannot verify Ed25519, so `slash()` trusts `SYSTEM_ADDRESS`; the
   node must verify both votes, the key binding (`consensusKeyOwner`) and the evidence max-age (< unbonding period).
   Evidence older than the unbonding period can no longer reach already-withdrawn stake.
3. **Unbonding-pool slashing over-approximates:** all entries in a validator's pool are cut, including entries created
   before the infraction height.
4. **Downtime uses tumbling windows** of `downtimeWindow` blocks evaluated at window end, not a sliding
   "last 10,000 blocks" window; validators that joined late in a window are judged on the blocks they were
   expected to sign (a validator with 1 expected block that misses it is jailed).
5. **Voting power = bonded principal**, not current value: slashing does not reduce voting power until the account's
   next delegate/undelegate. No vote delegation separate from stake delegation.
6. **Inflation schedule lives in the node** (7% → −1%/yr → 2% floor). The contract only distributes what it receives.
7. **Gas of system calls:** `recordParticipation` writes one slot per signing-set member per block (≤ 256) and
   `onEpoch` sorts up to `maxCandidates` (≤ 2,000) in memory; system calls are gas-free but this is real CPU work —
   benchmark before raising `maxValidators`/`maxCandidates`.
8. **Staking runtime size is 24,099 B** (EIP-170 limit 24,576 B). Irrelevant for genesis (no size limit for `alloc`
   code) but new features may need a split (e.g. a library) to keep CREATE-based tooling working.
9. **Governance:** no vote changes, no delegation of votes, no proposal-threshold re-check after creation
   (anyone-can-cancel rule not implemented), guardian cannot cancel queued proposals (by design, §25.1).
10. **FeeConfig** stores base-fee change denominator/elasticity, which §33.1 calls fork parameters — the node decides
    whether to honour them.
11. Static analysis (Slither/Aderyn, §18.3) has **not** been run on this machine; `forge lint` notes (mostly
    `block-timestamp`, `unsafe-typecast` on `uint64(block.number)`, `arbitrary-send-eth` on pull payments) were
    reviewed and are intentional.

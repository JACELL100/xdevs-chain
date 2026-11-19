# Local fee markets

Status: **built, off on every network** (Phase 5, plan §15.2). A consensus rule behind the
genesis activation `localFeeMarketsBlock` (absent = never); proposer and validators apply
it identically from that block on. Code: `crates/primitives/src/local_fee.rs` (math and
table layout), `crates/execution/src/local_fee.rs` (execution), mempool
`best_txs_with_local_fees`, producer `best_candidates`, RPC `xdevs_localFee`.

## Problem

With a single EIP-1559 base fee, one popular mint or airdrop claim fills blocks and raises
the price for **everyone**. Local fee markets price the congested contract instead: the
global base fee is computed exactly as before from total gas, and transactions to the hot
contract pay an extra **local base fee** on top.

## Rule

* **Target** of a transaction: its top-level `to` (any account; contract creations have
  no target). Gas used per target is summed over the block's included transactions
  (successful or not: the gas was used).
* **Congestion**: a target is congested in a block if it used more than 25% of the block
  gas limit (`LOCAL_FEE_TARGET_BPS = 2500`).
* **Update** at the end of every block, for each target with gas in it (`next_local_fee`):
  * congested and no local fee yet → `LOCAL_FEE_MIN` (0.1 gwei, the global floor);
  * congested → `+ fee × (used − target) / target / 8` (at least 1 wei): +12.5% per
    target of excess, up to +37.5% for a full block;
  * not congested → `− fee × (target − used) / target / 8`, i.e. −12.5% at zero use;
    below `LOCAL_FEE_MIN` it drops to 0;
  * capped at `LOCAL_FEE_MAX` (10,000 gwei).
  Targets without gas in a block decay lazily by the zero-use rule for every idle block
  (after 256 idle blocks the fee is 0 whatever its level).
* **Payment**: a transaction to target `t` in block `n` has base fee
  `base_fee(n) + local_fee(t, n)`: it needs `max_fee_per_gas ≥ base + local`, pays
  `min(max_fee, base + local + max_priority)` per gas, its tip (to the proposer) is what
  is above `base + local`, and the whole base part is split like the global base fee
  (80% burned, 20% treasury, plan §10.2). Transactions to other targets pay the global
  base fee only; the global base fee itself is unchanged.
* **Determinism**: the local fees of block `n` depend only on the parent state, because
  the table is rewritten after all transactions of a block. Execution order inside the
  block (including Block-STM) cannot change them.

Example (30M gas blocks, target 7.5M): a contract taking 24M gas per block gets 0.1 gwei
after the first such block, then +27.5% per block: ~1 gwei after 10 blocks, ~11 gwei after
20. When demand stops it decays 12.5% per block (to zero in ~20 blocks from 1 gwei).

## State

The table is the storage of `0x0000000000000000000000000000000000001101` (an account with
nonce 1 and no code, created at the first update; nonce 1 so that EIP-161 never removes
it): Solidity layout `mapping(address => uint256)` at slot 0, word
`fee << 64 | last_block` = the local fee that applies in block `last_block + 1`. It is part
of the state root, readable with `eth_getStorageAt`, and reproducible by any client.

## Implementation notes

* Execution sets revm's block base fee per transaction to `base + local` and passes the
  same value to the fee split and the XIP-0001 checks. Consequence: the `BASEFEE` opcode
  inside a transaction to a congested target returns `base + local` (the base fee *that
  transaction* pays). Contracts comparing `tx.gasprice` with `block.basefee` stay
  consistent.
* Block building: a candidate whose fee cap is below `base + local` is skipped as "not
  now" (it stays in the pool), like a candidate below the global base fee.
* Mempool ordering (`best_txs_with_local_fees`): a sender's next transaction is eligible
  if its fee cap covers `base + local(to)` and is ranked by its tip above that; the
  producer reads `local(to)` from the head state (cached per target per block).
* `xdevs_localFee(contract, block?)` (alias `xdevs_getLocalFee`) returns
  `{ active, contract, globalBaseFee, localBaseFee, totalBaseFee }` for the block after
  `block` (default `latest`), the shape the TypeScript SDK's `getLocalFee` expects.
  Wallets should set `maxFeePerGas ≥ totalBaseFee` (plus headroom) for transactions to
  that contract.

## Tests

* `crates/primitives/src/local_fee.rs`: the update rule (rise, minimum, decay, drop,
  cap), lazy decay equal to per-block decay, table words and the Solidity slot.
* `crates/execution/src/tests/local_fee.rs`: a contract taking 10M of 30M gas per block
  and an unrelated transfer in the same blocks: the hot sender pays exactly
  `gas × (base + local + tip)`, the transfer exactly `21000 × (base + tip)`, the treasury
  gets 20% of the whole base part, the local fee goes 0 → 0.1 gwei → rising, decays when
  idle; every block is also executed with Block-STM and must match. Nothing changes
  before the activation block.
* `crates/mempool`: ordering and eligibility with a local fee on one target only.
* `crates/testutils/tests/phase5_local_fee_markets.rs` (LocalNet, 4 validators, 2 of them
  with Block-STM, `localFeeMarketsBlock = 1`): a burst of 12M-gas transactions to a
  gas-burning contract; for every block the hot sender's balance drop equals
  `gas × (globalBaseFee + localBaseFee + tip)` with the fee reported by `xdevs_localFee`
  for that block, the unrelated transfers pay exactly the global price, the local fee
  starts at 0 and rises, and all nodes agree on every block hash and state root.

## Limits

* Keyed by top-level `to` (as the plan says): a hot contract called through a router or
  an aggregator is not priced separately, and a router used by many dapps is priced as
  one target. Per-account or per-slot pricing needs the access list or the execution
  trace before the transaction runs; out of scope for this version.
* Parameters (25%, 1/8, min, max) are constants, not yet read from the `FeeConfig`
  predeploy (plan §18 lists "local fee params" there); changing them is a fork.
* Receipts' `effectiveGasPrice` and `eth_feeHistory` report the global price only; the
  local part is visible through `xdevs_localFee` at the parent block and in balances.
  `eth_gasPrice` / `eth_maxPriorityFeePerGas` are global suggestions; `eth_estimateGas`
  is unaffected (it estimates gas units, not price).
* The `XDEVS_CONTEXT` precompile that would expose the local fee to contracts (plan §10.3)
  is not implemented.
* Splitting demand across many contracts or senders evades a per-target price by design;
  the global base fee still rises with total demand.

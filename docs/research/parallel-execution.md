# Parallel execution (Block-STM)

Status: **built, off by default** (Phase 5, plan §10.5). A node-local execution strategy,
not a consensus rule: a validator that enables it computes exactly the same receipts,
logs, state diff and state root as one that does not, so operators can switch it on
independently, with no activation height.

```toml
[execution]
parallel = true   # default false
threads = 0       # worker threads; 0 = one per CPU core
```

Code: `crates/execution/src/parallel/` (`mod.rs` driver, `mv.rs` multi-version memory,
`scheduler.rs`), entry points `EvmExecutor::execute_parallel` and `EvmExecutor::execute_with`
(which dispatches on the node's `[execution]` section).

## What runs in parallel

Only **strict execution of a received block** (what every validator does for every
proposal, and what full nodes do while syncing). Building a block stays sequential: the
builder decides which candidate transactions to include one at a time (gas left, nonce
gaps, invalid transactions), which is inherently ordered. In a network every block is
built sequentially once by its proposer and executed by the other validators and the
full nodes, in parallel where enabled.

## Algorithm

1. **System calls first, sequentially.** From `stakingValidatorsBlock` on, the Staking
   calls (reward, `recordParticipation`, `slash`, `onEpoch`) run before the transactions,
   exactly as in sequential execution. Their changes form the base state of step 3.
2. **Sender recovery** (ECDSA) is split across the worker threads.
3. **Block-STM** (Gelashvili et al., 2022) with the paper's collaborative scheduler:
   * every transaction is executed optimistically; reads go through a **multi-version
     memory** that returns the newest write of a *lower* transaction (or the base state)
     and are recorded with the value seen;
   * an incarnation's writes are installed in the multi-version memory; a **validation**
     task re-resolves every recorded read (value-based) and, if anything changed, aborts
     the transaction: its writes become `ESTIMATE` markers and it is re-executed;
   * a read that hits an `ESTIMATE` suspends the reader until the writer has run again
     (dependency), instead of executing on data that is known to be stale;
   * execution and validation indices only move down when a transaction is aborted or
     resumed, so the scheduler is done exactly when the last incarnation of every
     transaction has been validated against the final writes of all lower transactions.
4. **In-order fold.** The final outcomes are folded into one state overlay in block order,
   applying the same per-transaction checks as sequential execution (signature, blob
   type, cumulative gas vs. the block gas limit, invalid / XIP-0001-invalid transactions),
   so even the error returned for an invalid block is identical.

Each incarnation runs the **same code path** as sequential execution: `apply_tx` (revm,
XIP-0001 checks, the commit rules of `StateOverlay`) on a `StateOverlay` whose base is a
view of the multi-version memory. The commit rules (EIP-161 removal, EIP-6780
self-destruct, storage wipe on re-creation) therefore cannot drift between the two paths.

### State locations

| Location | Meaning |
|---|---|
| `Account(a)` | nonce, balance, code hash (or "does not exist") |
| `Slot(a, k)` | one storage slot |
| `Wipe(a)` | all storage of `a` cleared (destroyed or re-created); a slot read sees zero if the newest wipe below the reader is newer than the newest write of that slot |

Bytecode is content-addressed and immutable, so it is kept in a plain map.

### Fees without serialisation

Every transaction pays the block beneficiary (tip) and the treasury (20% of the base fee).
Done inside each transaction, that is a read-modify-write of two accounts by *every*
transaction, i.e. a fully serial block. The parallel path runs revm with a handler whose
only difference is that the beneficiary is not paid inside the transaction (the amount
revm would pay is recorded instead), and records the tip and the treasury share as
**balance deltas** in the multi-version memory. A reader of these accounts sees the base
value plus all lower deltas; a transaction that does not read them does not depend on
them. The in-order fold credits beneficiary then treasury right after each transaction,
exactly where sequential execution credits them.

Two corner cases where a deferred tip would not be equivalent are executed sequentially
instead (`ParallelStats::fallback`): a beneficiary account that exists but is empty
(sequential execution deletes it under EIP-161 when a zero tip touches it), and a
transaction that destroys the beneficiary (revm then drops that transaction's tip). A
worker panic also falls back. All three are reported in the stats and logged at debug.

### Ordering-sensitive parts

* Staking system calls: before the transactions, sequential (step 1).
* Encrypted-mempool transactions: their order is fixed by the block body (the decrypted
  prefix is checked before execution, `crates/node/src/app.rs`); Block-STM preserves the
  body order by construction.
* Local fee markets: the fees of a block come from its parent state and the table is
  updated after all transactions, so they never create dependencies between transactions.

## Determinism and equivalence

* Results never depend on thread timing: the outcome of every transaction is the outcome
  of an incarnation whose recorded reads equal, value for value, what sequential execution
  reads at that position; revm is deterministic in its inputs.
* No `HashMap` is iterated (they are only used for lookups); the write sets come from
  `BTreeMap`-based diffs, and the fold is in block order (plan §10.6).
* Tests (`crates/execution/tests/parallel.rs`, run on every PR):
  * a property test over 24 seeds × 3 chained blocks × 3 thread counts of a generated mix
    of transfers, a hot counter, a self-calling contract, an ERC-20-like token, keyed
    storage collisions, reverts, out-of-gas calls, deployments called later in the same
    block, constructors that self-destruct to `COINBASE`, value forwarded to `COINBASE`,
    contracts reading `BALANCE(COINBASE)` and `BALANCE(treasury)`, legacy and EIP-1559
    transactions with random tips (including zero): `ExecutionOutcome` (receipts, logs,
    blooms, gas, state diff, state root) must be equal;
  * hot-spot, ERC-20 and independent workloads (120 transactions, chained), with the hot
    counter checked to hold every increment;
  * the same block on 1–12 threads, repeatedly;
  * both fallbacks, and invalid blocks (replayed nonce, block gas exceeded) failing with
    the identical error;
  * the local fee market tests also run every block through Block-STM.
* LocalNet (`crates/testutils/tests/phase5_parallel_execution.rs`): 4 validators, 2 with
  Block-STM and 2 sequential, ~20 blocks of transfers to shared recipients and calls of a
  hot contract. A block needs 3 of 4 prevotes and a validator only prevotes a block whose
  execution reproduces the header's state root, so any divergence would stall the chain;
  the test also checks identical hashes and roots on every node, the exact counter value,
  and that Block-STM actually ran.

## Benchmark

`crates/execution/examples/parallel_bench.rs` builds one block per workload and times
sequential vs. Block-STM execution (median of 5, outcomes asserted equal). CI runs it on
every PR touching Rust (`cargo run --release -p xdevs-execution --example parallel_bench
-- 2000`) on a GitHub-hosted `ubuntu-latest` runner (4 vCPUs):

CI run of 20 Nov 2026 (PR #20, 2,000 transactions per block, median of 5, ms):

| Workload | sequential | 1 thread | 2 threads | 4 threads | 8 threads |
|---|---|---|---|---|---|
| Independent transfers | 289.4 | 297.1 (0.97×) | 156.7 (1.85×) | 138.2 (**2.09×**) | 143.6 (2.02×) |
| ERC-20, half to one hot receiver | 288.7 | 300.4 (0.96×) | 161.3 (1.79×) | 151.7 (1.90×) | 180.3 (1.60×) |
| One hot counter slot | 287.8 | 297.4 (0.97×) | 165.1 (1.74×) | 157.9 (1.82×) | 201.6 (1.43×) |
| Mixed (differential workload) | 288.6 | 297.8 (0.97×) | 157.3 (1.83×) | 139.7 (2.07×) | 162.3 (1.78×) |

Incarnations / validation aborts at 4 threads: independent 2,000 / 0, ERC-20 7,589 /
1,018, hot counter 10,513 / 1,749, mixed 3,062 / 537 (at 8 threads on 4 vCPUs the hot
workloads re-execute 10–13× more, which is why 8 threads is slower than 4).

Reading the numbers:

* The time includes sender recovery (ECDSA, ~half of the per-transaction cost of these
  small transactions), the EVM work and the state root. Recovery is split across the
  threads in the parallel path, which is why even the fully serial **hot counter**
  workload gets ~1.8×: there the EVM part is effectively sequential (every transaction
  waits for the previous one) and the gain comes from recovery and from overlapping
  re-executions. The same split could be applied to the sequential path; the speed-up
  attributable to Block-STM itself is therefore the gap between the hot-spot and the
  independent rows.
* **Independent transfers** are the best case (no conflicts): ~2.1× on 4 vCPUs. The state
  root (sequential, after execution) and the shared-memory traffic of a 4-vCPU runner
  cap it well below 4×.
* **Mixed** (the differential-test workload) behaves like the independent case: most of
  its conflicts (hot counter, token, map keys) resolve with few re-executions.
* With 1 thread Block-STM costs ~3% over sequential execution; on a 1-core VM it should
  stay off.
* These are small, cheap transactions; heavier contract calls make the EVM share (and
  the potential gain) larger. Measurements on reference hardware with ≥ 8 cores are still
  to be published (Phase 5 exit criterion).

On a 1-core VM parallel execution can only add overhead; leave it off there (plan §10.5:
"only enable when validators have ≥ 4 cores").

## Limits and next steps

* Building stays sequential (see above). A proposer could pre-execute candidates in
  parallel and feed the builder; not done.
* The multi-version memory is sharded `Mutex<HashMap>` (std only, no new dependency);
  a lock-free structure would scale further on many cores.
* Each incarnation builds a fresh revm instance; pooling them would cut overhead for
  small transactions.
* The plan's exit criterion ("identical roots across ≥ 1M devnet blocks") is a long-run
  check for the devnet; the CI evidence is the property test, the LocalNet test and the
  parallel-execution validators of a network whose blocks would otherwise not finalise.

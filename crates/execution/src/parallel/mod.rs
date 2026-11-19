//! Block-STM parallel block execution (Phase 5, plan §10.5).
//!
//! A **local execution strategy**, not a consensus rule: [`EvmExecutor::execute_parallel`]
//! returns exactly the [`ExecutionOutcome`] of [`EvmExecutor::execute_sequential`]
//! (receipts, logs, state diff, roots, errors), whatever the thread count or scheduling.
//! Validators may enable it independently (`[execution] parallel = true`).
//!
//! # Algorithm
//!
//! 1. The pre-transaction Staking system calls run sequentially (they must come first).
//! 2. Senders are recovered in parallel.
//! 3. Block-STM ([`scheduler`], [`mv`]): every transaction is executed optimistically
//!    against a multi-version memory holding the writes of lower transactions; each
//!    incarnation records its read set; a validation re-resolves every read and aborts
//!    the transaction (its writes become `ESTIMATE`s) if any value changed; a read of an
//!    `ESTIMATE` suspends the reader until the writer re-executed. When the scheduler is
//!    done, the last incarnation of every transaction has been validated against the
//!    final writes of all lower transactions, i.e. it read exactly what sequential
//!    execution would have read.
//! 4. The outcomes are folded **in block order** into one overlay, applying the same
//!    per-transaction checks as sequential execution (signature, blob type, block gas
//!    limit, invalid transactions), so even the error of an invalid block is identical.
//!
//! Each incarnation runs the ordinary sequential code path (`apply_tx` on a
//! `StateOverlay`) whose base state is a [`mv::View`], so EVM semantics, XIP-0001 checks
//! and the commit rules are shared and cannot drift apart.
//!
//! **Fees.** Every transaction pays the block beneficiary (tip) and the treasury (base-fee
//! share); done inside the transaction, that would make every pair of transactions
//! conflict. The parallel path therefore runs revm with the beneficiary payment deferred
//! and records both payments as balance *deltas* in the multi-version memory (credited
//! right after the transaction, exactly where sequential execution credits them). Only a
//! transaction that actually reads those accounts depends on earlier fees.
//!
//! **Sequential fallback** (same result, just not parallel): a worker panic, a
//! beneficiary account that exists but is empty (sequential execution would delete it
//! via EIP-161 when paying a zero tip), or a transaction that destroys the beneficiary
//! (revm then drops the tip). These are rare corner cases; the fallback keeps them
//! trivially correct.

mod mv;
mod scheduler;

use crate::{
    db::StateOverlay,
    env::{block_env, cfg_env, BLOB_TX_TYPE},
    executor::{make_receipt, new_evm, AppliedTx, FeeCredits, TxFailure},
    EvmExecutor, ExecutionError, ExecutionOutcome, StakingInputs,
};
use mv::{lock, Base, Loc, MvMemory, Val, View};
use scheduler::{Scheduler, Task, Version};
use std::{
    collections::BTreeSet,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst},
        Mutex,
    },
};
use xdevs_primitives::{Block, Header, RecoveredTx, Transaction, Typed2718};
use xdevs_state::{StateDiff, StateProvider};

/// Blocks with fewer transactions are executed sequentially by
/// [`EvmExecutor::execute_with`] (nothing to parallelise).
pub const MIN_PARALLEL_TXS: usize = 2;

static PARALLEL_BLOCKS: AtomicU64 = AtomicU64::new(0);

/// Blocks executed by Block-STM in this process (without falling back). For tests and
/// metrics.
pub fn parallel_blocks_executed() -> u64 {
    PARALLEL_BLOCKS.load(SeqCst)
}

/// What a parallel execution did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParallelStats {
    /// Transactions in the block.
    pub transactions: usize,
    /// Worker threads used.
    pub threads: usize,
    /// Transaction incarnations run (≥ `transactions`; the excess is re-execution).
    pub executions: usize,
    /// Incarnations suspended on a read of an `ESTIMATE`.
    pub dependency_waits: usize,
    /// Incarnations aborted by validation.
    pub validation_aborts: usize,
    /// Why the block was executed sequentially instead (`None` = Block-STM was used).
    pub fallback: Option<&'static str>,
}

/// One transaction of the block, prepared for execution.
struct Prepared {
    /// Recovered transaction, or the recovery error.
    tx: Result<RecoveredTx, String>,
    /// XIP-0001: `Some(sender already in the block)` when active.
    xip: Option<bool>,
}

impl Prepared {
    fn executable(&self) -> Option<&RecoveredTx> {
        self.tx.as_ref().ok().filter(|t| t.ty() != BLOB_TX_TYPE)
    }
}

/// Output of the last incarnation of a transaction.
struct Payload {
    diff: StateDiff,
    credits: FeeCredits,
    /// `None` for a transaction that is not executed (bad signature / blob).
    result: Option<Result<AppliedTx, TxFailure>>,
}

enum Incarnation {
    Blocked(usize),
    Done(Payload, Vec<(Loc, Val)>),
}

/// State shared by the workers of one block.
struct Shared<'s, 'a> {
    exec: &'s EvmExecutor,
    header: &'s Header,
    base_fee: u64,
    txs: &'s [Prepared],
    base: Base<'a>,
    mv: MvMemory,
    sched: Scheduler,
    outputs: Vec<Mutex<Option<Payload>>>,
    abort: AtomicBool,
    executions: AtomicUsize,
    waits: AtomicUsize,
    aborts: AtomicUsize,
}

impl Shared<'_, '_> {
    fn run_incarnation(&self, idx: usize) -> Incarnation {
        let view = View::new(&self.mv, &self.base, idx);
        let mut overlay = StateOverlay::new(&view);
        let mut credits = FeeCredits::default();
        let result = self.txs[idx].executable().map(|tx| {
            let cfg = self.exec.chain_config();
            let mut evm = new_evm(
                &mut overlay,
                cfg_env(cfg.chain_id),
                block_env(self.header),
                cfg,
            );
            self.exec.apply_tx(
                &mut evm,
                tx,
                self.base_fee,
                self.txs[idx].xip,
                Some(&mut credits),
            )
        });
        let diff = overlay.into_diff();
        if let Some(on) = view.blocked() {
            return Incarnation::Blocked(on);
        }
        Incarnation::Done(
            Payload {
                diff,
                credits,
                result,
            },
            view.into_reads(),
        )
    }

    fn try_execute(&self, (idx, incarnation): Version) -> Option<Task> {
        loop {
            self.executions.fetch_add(1, SeqCst);
            match self.run_incarnation(idx) {
                Incarnation::Blocked(on) => {
                    self.waits.fetch_add(1, SeqCst);
                    if self.sched.add_dependency(idx, on) {
                        return None;
                    }
                    // `on` finished meanwhile: run again right away.
                }
                Incarnation::Done(payload, reads) => {
                    let writes = mv::write_set(
                        &payload.diff,
                        &payload.credits,
                        self.header.beneficiary,
                        self.exec.chain_config().treasury,
                    );
                    let wrote_new = self.mv.record(idx, writes, reads, &payload.diff.codes);
                    *lock(&self.outputs[idx]) = Some(payload);
                    return self.sched.finish_execution(idx, incarnation, wrote_new);
                }
            }
        }
    }

    fn validate(&self, (idx, incarnation): Version) -> Option<Task> {
        let valid = self.mv.validate(idx, &self.base);
        let aborted = !valid && self.sched.try_validation_abort(idx, incarnation);
        if aborted {
            self.aborts.fetch_add(1, SeqCst);
            self.mv.convert_writes_to_estimates(idx);
        }
        self.sched.finish_validation(idx, aborted)
    }

    /// One worker: take tasks until the scheduler is done (or a worker panicked).
    fn work(&self) {
        let mut task: Option<Task> = None;
        let mut idle = 0u32;
        while !self.sched.done() && !self.abort.load(SeqCst) {
            task = match task.take() {
                Some(t) => {
                    idle = 0;
                    let run = catch_unwind(AssertUnwindSafe(|| match t {
                        Task::Execute(v) => self.try_execute(v),
                        Task::Validate(v) => self.validate(v),
                    }));
                    run.unwrap_or_else(|_| {
                        self.abort.store(true, SeqCst);
                        None
                    })
                }
                None => {
                    let next = self.sched.next_task();
                    if next.is_none() {
                        idle = idle.wrapping_add(1);
                        if idle.is_multiple_of(64) {
                            std::thread::yield_now();
                        } else {
                            std::hint::spin_loop();
                        }
                    }
                    next
                }
            };
        }
    }
}

/// Recover all senders on `threads` threads (in block order).
fn prepare(block: &Block, threads: usize, xip0001_active: bool) -> Vec<Prepared> {
    let n = block.body.len();
    let mut recovered: Vec<Option<Result<RecoveredTx, String>>> = (0..n).map(|_| None).collect();
    let chunk = n.div_ceil(threads.max(1)).max(1);
    std::thread::scope(|s| {
        for (txs, out) in block.body.chunks(chunk).zip(recovered.chunks_mut(chunk)) {
            s.spawn(move || {
                for (tx, slot) in txs.iter().zip(out.iter_mut()) {
                    *slot = Some(RecoveredTx::recover(tx.clone()).map_err(|e| e.to_string()));
                }
            });
        }
    });
    let mut seen = BTreeSet::new();
    recovered
        .into_iter()
        .map(|r| {
            let tx = r.unwrap_or_else(|| Err("not recovered".into()));
            let xip = match &tx {
                Ok(t) if xip0001_active => Some(!seen.insert(t.sender)),
                _ => None,
            };
            Prepared { tx, xip }
        })
        .collect()
}

fn state_err(e: crate::StateDbError) -> ExecutionError {
    ExecutionError::State(e.0)
}

impl EvmExecutor {
    /// Execute `block` with Block-STM on `threads` worker threads. The outcome (or the
    /// error) is identical to [`EvmExecutor::execute_sequential`]; corner cases listed
    /// in the [module docs](crate::parallel) are executed sequentially
    /// ([`ParallelStats::fallback`]).
    pub fn execute_parallel(
        &self,
        parent: &Header,
        block: &Block,
        state: &(dyn StateProvider + Sync),
        staking: &StakingInputs,
        threads: usize,
    ) -> Result<(ExecutionOutcome, ParallelStats), ExecutionError> {
        let header = &block.header;
        self.validate_child(parent, header)?;
        let cfg = self.chain_config();
        let base_fee = header.base_fee_per_gas.unwrap_or_default();
        let n = block.body.len();
        let threads = threads.clamp(1, n.max(1));
        let mut stats = ParallelStats {
            transactions: n,
            threads,
            ..Default::default()
        };
        let fallback = |stats: ParallelStats, why: &'static str| {
            tracing::debug!(block = header.number, why, "parallel execution fell back");
            self.execute_sequential(parent, block, state, staking)
                .map(|o| {
                    (
                        o,
                        ParallelStats {
                            fallback: Some(why),
                            ..stats
                        },
                    )
                })
        };

        // 1. System calls before the transactions (sequential).
        let mut db = StateOverlay::new(state);
        let mut next_validators = None;
        if cfg.staking_active(header.number) {
            let mut evm = new_evm(&mut db, cfg_env(cfg.chain_id), block_env(header), cfg);
            next_validators = self.run_staking(&mut evm, header, staking)?;
        }
        if let Some(b) = db.account(header.beneficiary).map_err(state_err)? {
            if b.nonce == 0 && b.balance.is_zero() && !b.has_code() {
                return fallback(stats, "beneficiary exists but is empty");
            }
        }

        // 2. Senders, 3. Block-STM.
        let txs = prepare(block, threads, cfg.xip0001_active(header.number));
        let shared = Shared {
            exec: self,
            header,
            base_fee,
            txs: &txs,
            base: Base::new(state, db.diff().clone()),
            mv: MvMemory::new(n),
            sched: Scheduler::new(n),
            outputs: (0..n).map(|_| Mutex::new(None)).collect(),
            abort: AtomicBool::new(false),
            executions: AtomicUsize::new(0),
            waits: AtomicUsize::new(0),
            aborts: AtomicUsize::new(0),
        };
        std::thread::scope(|s| {
            for _ in 1..threads {
                s.spawn(|| shared.work());
            }
            shared.work();
        });
        stats.executions = shared.executions.load(SeqCst);
        stats.dependency_waits = shared.waits.load(SeqCst);
        stats.validation_aborts = shared.aborts.load(SeqCst);
        if shared.abort.load(SeqCst) {
            return fallback(stats, "a worker panicked");
        }
        let mut outputs: Vec<Option<Payload>> = shared
            .outputs
            .into_iter()
            .map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()))
            .collect();

        // 4. Fold in block order with the sequential per-transaction checks.
        let mut receipts = Vec::with_capacity(n);
        let mut cumulative = 0u64;
        for (i, tx) in block.body.iter().enumerate() {
            let hash = *tx.tx_hash();
            let rtx = match &txs[i].tx {
                Ok(rtx) => rtx,
                Err(e) => {
                    return Err(ExecutionError::InvalidTransaction {
                        hash,
                        reason: format!("invalid signature: {e}"),
                    })
                }
            };
            if rtx.ty() == BLOB_TX_TYPE {
                return Err(ExecutionError::UnsupportedTxType(BLOB_TX_TYPE));
            }
            let needed = cumulative.saturating_add(rtx.gas_limit());
            if needed > header.gas_limit {
                return Err(ExecutionError::BlockGasLimitExceeded {
                    used: needed,
                    limit: header.gas_limit,
                });
            }
            let Some(Payload {
                diff,
                credits,
                result: Some(result),
            }) = outputs[i].take()
            else {
                return fallback(stats, "missing transaction output");
            };
            let applied = match result {
                Ok(a) => a,
                Err(TxFailure::Invalid(e)) => {
                    return Err(ExecutionError::InvalidTransaction {
                        hash,
                        reason: e.to_string(),
                    })
                }
                Err(TxFailure::Xip0001(error)) => {
                    return Err(ExecutionError::Xip0001 {
                        hash: Some(hash),
                        error: Box::new(error),
                    })
                }
                Err(TxFailure::Fatal(e)) => return Err(e),
            };
            if diff.accounts.get(&header.beneficiary) == Some(&None) {
                return fallback(stats, "a transaction destroyed the beneficiary");
            }
            db.merge_diff(diff);
            db.credit(header.beneficiary, credits.tip)
                .map_err(state_err)?;
            db.credit(cfg.treasury, credits.treasury)
                .map_err(state_err)?;
            cumulative += applied.gas_used;
            receipts.push(make_receipt(rtx, applied, cumulative));
        }
        PARALLEL_BLOCKS.fetch_add(1, SeqCst);
        Self::finish(state, db.into_diff(), receipts, cumulative, next_validators)
            .map(|o| (o, stats))
    }
}

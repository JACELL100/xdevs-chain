//! [`ChainApp`]: the node's [`ConsensusApp`] (research doc §6.3, ADR-0003
//! execute-before-vote).
//!
//! * **propose** builds AND executes the next block on the committed head (the dev
//!   producer's build step without the commit) and caches the execution outcome.
//! * **validate** re-executes a proposal on the committed head and compares roots, bloom
//!   and gas with the header. Verdicts are cached by block hash, so the repeated calls the
//!   engine makes (proposal received, `StartedRound`, sync) cost one execution.
//! * **commit** writes block + commit certificate + state diff + history in **one**
//!   `WriteBatch`, from the cached outcome (re-executing once on a cache miss), then
//!   updates the pool. It is idempotent: a second call for a committed block only
//!   replaces the certificate when the new one carries more signatures.
//!
//! Every storage / EVM step runs on a blocking thread.

use crate::{
    chain::{self, Db},
    producer::{after_commit, build_next_block, drop_rejected, unix_now, NextBlock},
};
use anyhow::{bail, ensure, Context};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Instant};
use xdevs_consensus::{
    BlockOrigin, CommitCertificate, ConsensusApp, Evidence, ValidatorSet, Validity,
};
use xdevs_execution::{
    next_base_fee, BlockExecutor, EvmExecutor, ExecutionError, ExecutionOutcome,
};
use xdevs_mempool::Pool;
use xdevs_primitives::{
    constants::MAX_EXTRA_DATA_SIZE, set_compat_header_fields, Address, Block, Bytes, ChainConfig,
    Encodable2718, Header, Transaction, Typed2718, B256,
};
use xdevs_rpc::ChainEvents;
use xdevs_state::DbStateProvider;

pub use crate::producer::MAX_BLOCK_BYTES;

/// `extraData` of blocks proposed through consensus.
pub const PROPOSAL_EXTRA_DATA: &[u8] = b"xdevs";
/// Execution verdicts kept (by block hash); the lowest heights are evicted first.
const VERDICT_CACHE_CAPACITY: usize = 64;
const LOG: &str = "xdevs::app";

/// Settings of a [`ChainApp`].
#[derive(Clone, Debug)]
pub struct ChainAppConfig {
    /// Beneficiary of the blocks this node proposes (priority fees).
    pub fee_recipient: Address,
    /// `extraData` of proposed blocks (≤ 32 bytes).
    pub extra_data: Bytes,
    /// Past blocks whose state stays readable (see [`chain::commit_block`]).
    pub state_history: u64,
    /// Max seconds a live proposal may run ahead of the local clock (rule 3).
    pub max_timestamp_drift_secs: u64,
}

impl Default for ChainAppConfig {
    fn default() -> Self {
        Self {
            fee_recipient: Address::ZERO,
            extra_data: Bytes::from_static(PROPOSAL_EXTRA_DATA),
            state_history: xdevs_state::history::DEFAULT_STATE_HISTORY_BLOCKS,
            max_timestamp_drift_secs: crate::config::DEFAULT_MAX_TIMESTAMP_DRIFT_SECS,
        }
    }
}

/// Cached verdict of a block (by hash).
#[derive(Clone, Debug)]
enum Verdict {
    /// Executes on its parent; the outcome is what commit writes.
    Valid(Arc<ExecutionOutcome>),
    /// Deterministically invalid (the reason is logged).
    Invalid(Arc<str>),
}

impl Verdict {
    fn invalid(reason: impl std::fmt::Display) -> Self {
        Self::Invalid(reason.to_string().into())
    }

    fn validity(&self) -> Validity {
        Validity::from_bool(matches!(self, Self::Valid(_)))
    }
}

struct Inner {
    db: Db,
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
    cfg: ChainConfig,
    set: ValidatorSet,
    app: ChainAppConfig,
    /// block hash -> (block number, verdict).
    verdicts: Mutex<HashMap<B256, (u64, Verdict)>>,
    /// Serialises commits (the only chain writer in consensus modes).
    commit_lock: Mutex<()>,
    /// Committed blocks are published here (`eth_subscribe`); set once.
    events: std::sync::OnceLock<Arc<ChainEvents>>,
}

/// The node's [`ConsensusApp`] (see the module docs). Cheap to clone.
#[derive(Clone)]
pub struct ChainApp {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ChainApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainApp")
            .field("chain_id", &self.inner.cfg.chain_id)
            .field("validators", &self.inner.set.len())
            .finish_non_exhaustive()
    }
}

impl ChainApp {
    /// A chain app over an initialised database. `set` is the static genesis validator
    /// set (Phase 2: active at every height).
    pub fn new(
        db: Db,
        pool: Arc<Pool>,
        executor: Arc<EvmExecutor>,
        cfg: ChainConfig,
        set: ValidatorSet,
        app: ChainAppConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                pool,
                executor,
                cfg,
                set,
                app,
                verdicts: Mutex::new(HashMap::new()),
                commit_lock: Mutex::new(()),
                events: std::sync::OnceLock::new(),
            }),
        }
    }

    /// Publish every newly committed block to `events` (`newHeads` / `logs`
    /// subscriptions). Only the first call has an effect.
    pub fn with_events(self, events: Arc<ChainEvents>) -> Self {
        let _ = self.inner.events.set(events);
        self
    }

    /// Run `f` on a blocking thread.
    async fn blocking<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> anyhow::Result<T> + Send + 'static,
    {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || f(&inner))
            .await
            .context("chain app task failed")?
    }
}

impl Inner {
    fn cached(&self, hash: &B256) -> Option<Verdict> {
        self.verdicts.lock().get(hash).map(|(_, v)| v.clone())
    }

    fn remember(&self, number: u64, hash: B256, verdict: Verdict) {
        let mut cache = self.verdicts.lock();
        if cache.len() >= VERDICT_CACHE_CAPACITY && !cache.contains_key(&hash) {
            if let Some(evict) = cache.iter().min_by_key(|(_, (n, _))| *n).map(|(h, _)| *h) {
                cache.remove(&evict);
            }
        }
        cache.insert(hash, (number, verdict));
    }

    /// Build and execute block `height` on the committed head (no commit).
    fn build(&self, height: u64, with_txs: bool) -> anyhow::Result<Block> {
        let snapshot = self.db.snapshot()?;
        let (head, _) = chain::latest_header(&*snapshot)?;
        ensure!(
            height == head + 1,
            "cannot propose height {height}: committed head is {head}"
        );
        let (_, built) = build_next_block(
            &*snapshot,
            &self.pool,
            &self.executor,
            &self.cfg,
            &NextBlock {
                now_secs: unix_now(),
                beneficiary: self.app.fee_recipient,
                extra_data: self.app.extra_data.clone(),
                with_txs,
            },
        )?;
        drop(snapshot);
        drop_rejected(&self.pool, &built);
        let hash = built.block.hash();
        self.remember(height, hash, Verdict::Valid(Arc::new(built.outcome)));
        Ok(built.block)
    }

    /// The §6.3 validation rules against the committed head. `Err` = local failure
    /// (wrong height for this node, storage error): no verdict.
    fn verdict(
        &self,
        height: u64,
        block: &Block,
        origin: BlockOrigin,
        now_secs: u64,
    ) -> anyhow::Result<Verdict> {
        let snapshot = self.db.snapshot()?;
        let (head, parent) = chain::latest_header(&*snapshot)?;
        ensure!(
            height == head + 1,
            "cannot validate height {height}: committed head is {head}"
        );
        let header = &block.header;
        // Rule 3, live proposals only. Not cached: it depends on the clock. A proposer
        // must use at least `parent + 1`, so that is always timely.
        if origin == BlockOrigin::Consensus {
            let limit = now_secs
                .saturating_add(self.app.max_timestamp_drift_secs)
                .max(parent.timestamp + 1);
            if header.timestamp > limit {
                return Ok(Verdict::invalid(format!(
                    "timestamp {} is ahead of the local clock (limit {limit})",
                    header.timestamp
                )));
            }
        }
        // The cache is keyed by the header hash: the body must belong to that header
        // first (a proposer could pair a known header with another body).
        if !body_matches_header(block) {
            return Ok(Verdict::invalid(
                "transactions root does not match the body",
            ));
        }
        let hash = block.hash();
        if let Some(v) = self.cached(&hash) {
            return Ok(v);
        }
        let verdict = match self.check_shape(height, &parent, block) {
            Err(reason) => Verdict::invalid(reason),
            Ok(()) => {
                let state = DbStateProvider::new(&*snapshot);
                match self.executor.execute(&parent, block, &state) {
                    Ok(outcome) => match compare_outcome(header, &outcome) {
                        Ok(()) => Verdict::Valid(Arc::new(outcome)),
                        Err(reason) => Verdict::invalid(reason),
                    },
                    // A storage failure says nothing about the block.
                    Err(ExecutionError::State(e)) => bail!("state read failed: {e}"),
                    Err(e) => Verdict::invalid(e),
                }
            }
        };
        self.remember(header.number, hash, verdict.clone());
        Ok(verdict)
    }

    /// Rules 1, 2 and 4 (everything but the clock and execution).
    fn check_shape(&self, height: u64, parent: &Header, block: &Block) -> Result<(), String> {
        let h = &block.header;
        let parent_hash = parent.hash_slow();
        if h.number != height {
            return Err(format!("number {} at height {height}", h.number));
        }
        if h.parent_hash != parent_hash {
            return Err(format!(
                "parent hash {} is not the head {parent_hash}",
                h.parent_hash
            ));
        }
        if h.gas_limit != self.cfg.block_gas_limit {
            return Err(format!(
                "gas limit {}, expected {}",
                h.gas_limit, self.cfg.block_gas_limit
            ));
        }
        let base_fee = next_base_fee(parent, &self.cfg);
        if h.base_fee_per_gas != Some(base_fee) {
            return Err(format!(
                "base fee {:?}, expected {base_fee}",
                h.base_fee_per_gas
            ));
        }
        if h.timestamp <= parent.timestamp {
            return Err(format!(
                "timestamp {} not after parent timestamp {}",
                h.timestamp, parent.timestamp
            ));
        }
        if h.extra_data.len() > MAX_EXTRA_DATA_SIZE {
            return Err(format!("extraData of {} bytes", h.extra_data.len()));
        }
        let mut compat = h.clone();
        set_compat_header_fields(&mut compat);
        if compat != *h {
            return Err("compat header fields differ from the protocol values".into());
        }
        if h.mix_hash != parent_hash {
            return Err("prevRandao is not the parent hash".into());
        }
        let mut bytes = 0usize;
        for tx in &block.body {
            if tx.ty() == 0x03 {
                return Err(format!("blob transaction {}", tx.tx_hash()));
            }
            if tx.chain_id() != Some(self.cfg.chain_id) {
                return Err(format!(
                    "transaction {} has chain id {:?}",
                    tx.tx_hash(),
                    tx.chain_id()
                ));
            }
            bytes += tx.encode_2718_len();
        }
        if bytes > MAX_BLOCK_BYTES {
            return Err(format!("body of {bytes} bytes exceeds {MAX_BLOCK_BYTES}"));
        }
        Ok(())
    }

    /// Commit (or re-commit) a decided block with its certificate.
    fn commit(&self, block: Block, cert: CommitCertificate) -> anyhow::Result<()> {
        let _guard = self.commit_lock.lock();
        let number = block.number();
        let hash = block.hash();
        ensure!(
            cert.block_hash == hash && cert.height == number,
            "certificate (height {}, {}) is not for block {number} {hash}",
            cert.height,
            cert.block_hash
        );
        cert.verify_at(self.cfg.chain_id, number, &self.set)
            .map_err(|e| anyhow::anyhow!("invalid commit certificate for block {number}: {e:?}"))?;
        let (head, head_header) = chain::latest_header(&*self.db)?;

        if number <= head {
            // Idempotent re-commit (Decided, then Finalized): same block, maybe a
            // certificate with more signatures.
            let stored = chain::header(&*self.db, number)?
                .with_context(|| format!("missing header of committed block {number}"))?;
            let stored_hash = stored.hash_slow();
            ensure!(
                stored_hash == hash,
                "FORK: block {number} is committed as {stored_hash}, consensus decided {hash}"
            );
            let stored_sigs = chain::commit_certificate(&*self.db, number)?
                .and_then(|raw| CommitCertificate::from_bytes(&raw).ok())
                .map_or(0, |c| c.signatures.len());
            if cert.signatures.len() > stored_sigs {
                chain::write_commit_certificate(&*self.db, number, &cert.to_bytes())?;
                tracing::debug!(target: LOG, number, signatures = cert.signatures.len(),
                                "commit certificate upgraded");
            }
            return Ok(());
        }

        ensure!(
            number == head + 1,
            "cannot commit block {number}: committed head is {head}"
        );
        let head_hash = head_header.hash_slow();
        ensure!(
            block.header.parent_hash == head_hash,
            "decided block {number} does not extend the head {head_hash}"
        );
        ensure!(
            body_matches_header(&block),
            "decided block {number} {hash}: transactions root does not match the body"
        );
        let outcome = match self.cached(&hash) {
            Some(Verdict::Valid(outcome)) => outcome,
            Some(Verdict::Invalid(reason)) => {
                bail!("decided block {number} {hash} is invalid here: {reason}")
            }
            // Not seen before (e.g. after a restart): re-execute once.
            None => match self.verdict(number, &block, BlockOrigin::Sync, unix_now())? {
                Verdict::Valid(outcome) => outcome,
                Verdict::Invalid(reason) => {
                    bail!("decided block {number} {hash} is invalid here: {reason}")
                }
            },
        };
        chain::commit_executed_block(
            &*self.db,
            &block,
            &outcome,
            Some(&cert.to_bytes()),
            self.app.state_history,
        )?;
        crate::events::publish_block(
            self.events.get().map(|e| &**e),
            &block,
            hash,
            &outcome.receipts,
        );
        after_commit(&self.pool, &self.cfg, &block, &outcome.receipts);
        self.verdicts.lock().retain(|_, (n, _)| *n > number);
        tracing::info!(
            target: LOG,
            number,
            %hash,
            txs = block.body.len(),
            gas_used = block.header.gas_used,
            round = cert.round,
            signatures = cert.signatures.len(),
            "committed block"
        );
        Ok(())
    }
}

/// The body hashes to the header's `transactions_root`.
fn body_matches_header(block: &Block) -> bool {
    block.header.transactions_root == block.compute_transactions_root()
}

/// Rule 5: the re-execution results must equal the header.
fn compare_outcome(header: &Header, outcome: &ExecutionOutcome) -> Result<(), String> {
    if outcome.state_root != header.state_root {
        return Err(format!(
            "state root {} != header {}",
            outcome.state_root, header.state_root
        ));
    }
    if outcome.receipts_root != header.receipts_root {
        return Err(format!(
            "receipts root {} != header {}",
            outcome.receipts_root, header.receipts_root
        ));
    }
    if outcome.logs_bloom != header.logs_bloom {
        return Err("logs bloom differs from the header".into());
    }
    if outcome.gas_used != header.gas_used {
        return Err(format!(
            "gas used {} != header {}",
            outcome.gas_used, header.gas_used
        ));
    }
    Ok(())
}

#[async_trait::async_trait]
impl ConsensusApp for ChainApp {
    async fn head(&self) -> anyhow::Result<(u64, B256)> {
        self.blocking(|s| {
            let (number, header) = chain::latest_header(&*s.db)?;
            Ok((number, header.hash_slow()))
        })
        .await
    }

    async fn validator_set(&self, _height: u64) -> anyhow::Result<ValidatorSet> {
        Ok(self.inner.set.clone())
    }

    async fn propose(&self, height: u64, round: u32, deadline: Instant) -> anyhow::Result<Block> {
        let full = self.blocking(move |s| s.build(height, true));
        match tokio::time::timeout_at(deadline.into(), full).await {
            Ok(block) => block,
            Err(_) => {
                // A full build would miss the deadline: propose an empty block so the
                // height can still be decided this round.
                tracing::warn!(target: LOG, height, round, "block build missed the propose deadline; proposing an empty block");
                self.blocking(move |s| s.build(height, false)).await
            }
        }
    }

    async fn validate(
        &self,
        height: u64,
        block: &Block,
        origin: BlockOrigin,
    ) -> anyhow::Result<Validity> {
        let block = block.clone();
        let hash = block.hash();
        let verdict = self
            .blocking(move |s| s.verdict(height, &block, origin, unix_now()))
            .await?;
        if let Verdict::Invalid(reason) = &verdict {
            tracing::warn!(target: LOG, height, %hash, ?origin, %reason, "rejecting invalid block");
        }
        Ok(verdict.validity())
    }

    async fn commit(&self, block: Block, cert: CommitCertificate) -> anyhow::Result<()> {
        self.blocking(move |s| s.commit(block, cert)).await
    }

    async fn decided(&self, height: u64) -> anyhow::Result<Option<(Block, CommitCertificate)>> {
        self.blocking(move |s| {
            let snapshot = s.db.snapshot()?;
            let (Some(block), Some(raw)) = (
                chain::block(&*snapshot, height)?,
                chain::commit_certificate(&*snapshot, height)?,
            ) else {
                return Ok(None);
            };
            let cert = CommitCertificate::from_bytes(&raw)
                .map_err(|e| anyhow::anyhow!("stored certificate of block {height}: {e:?}"))?;
            Ok(Some((block, cert)))
        })
        .await
    }

    async fn earliest_height(&self) -> anyhow::Result<u64> {
        // Every block is kept in Phase 2.
        Ok(1)
    }

    async fn report_evidence(&self, e: Evidence) -> anyhow::Result<()> {
        tracing::warn!(target: LOG, validator = %e.validator(), height = e.height(), "equivocation evidence: {e:?}");
        Ok(())
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

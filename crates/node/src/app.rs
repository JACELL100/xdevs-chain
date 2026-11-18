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
//! * **Phase 3** (from `stakingValidatorsBlock`, see [`crate::staking`]): proposals carry
//!   consensus data (proposer, parent commit certificate, double-sign evidence) that
//!   validators verify before executing the Staking system calls; an epoch-boundary
//!   commit stores the next epoch's validator set; `validator_set(h)` follows the epoch
//!   sets; reported / submitted evidence is verified and pooled for inclusion.
//! * **Phase 5 prototype** (from `encryptedMempoolBlock`, see [`crate::encrypted`]): a
//!   committee member releases its decryption shares for the envelopes of every block it
//!   commits; the next block must start with their decryption (shares carried in the
//!   block, verified and re-executed by validators). The proposer waits (bounded) for
//!   enough shares before building.
//!
//! Every storage / EVM step runs on the app's own [`WorkerPool`] threads, never on
//! tokio's shared blocking pool: RPC requests (`eth_call`, `eth_estimateGas`, ...) use
//! that pool, and however many are queued, consensus work starts at once (security
//! review: RPC load starving consensus).
//!
//! **Timestamp rule (rule 3) and the verdict cache.** A live proposal
//! ([`BlockOrigin::Consensus`]) whose timestamp is more than `max_timestamp_drift_secs`
//! ahead of the local clock is invalid *now*. That rejection depends on the local clock,
//! so it is **never cached**: the same block is judged again on every call and becomes
//! valid once the local clock catches up. Only deterministic verdicts (shape and
//! execution) are cached. A block with a cached *valid* verdict is not re-checked against
//! the clock: it was timely when first judged (or proposed by us, or decided), and a
//! re-proposal of a value that already gathered a polka must not turn invalid because
//! of clock skew or a clock step backwards, which could split validators and stall the
//! height.

use crate::{
    chain::{self, Db},
    encrypted::{self, BlockEnvelope, DecryptionData, Encryption},
    producer::{after_commit, build_next_block, drop_rejected, unix_now, NextBlock},
    staking::{self, EvidencePool, MAX_EVIDENCE_AGE_BLOCKS, MAX_EVIDENCE_PER_BLOCK},
    workers::WorkerPool,
};
use anyhow::{bail, ensure, Context};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use xdevs_consensus::{
    BlockConsensusData, BlockOrigin, CommitCertificate, ConflictingVotes, ConsensusApp, Evidence,
    ValidatorAddress, ValidatorSet, Validity,
};
use xdevs_execution::{
    next_base_fee, BlockAttributes, EvmExecutor, ExecutionError, ExecutionOutcome, SlashEvidence,
    StakingInputs,
};
use xdevs_mempool::Pool;
use xdevs_primitives::{
    consensus_data_hash, constants::MAX_EXTRA_DATA_SIZE, set_compat_header_fields, Address, Block,
    Bytes, ChainConfig, Encodable2718, Header, RecoveredTx, Transaction, Typed2718, B256,
};
use xdevs_rpc::ChainEvents;
use xdevs_state::DbStateProvider;
use xdevs_storage::DbRead;

pub use crate::producer::MAX_BLOCK_BYTES;

/// `extraData` of blocks proposed through consensus.
pub const PROPOSAL_EXTRA_DATA: &[u8] = b"xdevs";
/// Execution verdicts kept (by block hash); the lowest heights are evicted first.
const VERDICT_CACHE_CAPACITY: usize = 64;
/// Threads of the consensus worker pool. More than one: a full block build that misses
/// the propose deadline keeps running while the empty fallback block is built.
const CONSENSUS_THREADS: usize = 4;
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
    /// This node's consensus address (validators): named as the proposer in the
    /// consensus data of the blocks it builds (Phase 3). `None` on full nodes.
    pub validator_address: Option<ValidatorAddress>,
}

impl Default for ChainAppConfig {
    fn default() -> Self {
        Self {
            fee_recipient: Address::ZERO,
            extra_data: Bytes::from_static(PROPOSAL_EXTRA_DATA),
            state_history: xdevs_state::history::DEFAULT_STATE_HISTORY_BLOCKS,
            max_timestamp_drift_secs: crate::config::DEFAULT_MAX_TIMESTAMP_DRIFT_SECS,
            validator_address: None,
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
    /// The genesis validator set (in force until the first epoch boundary).
    set: ValidatorSet,
    /// Epoch sets read so far, by boundary block (immutable once stored).
    epoch_sets: Mutex<BTreeMap<u64, ValidatorSet>>,
    /// Verified double-sign evidence waiting for a block.
    evidence: EvidencePool,
    app: ChainAppConfig,
    /// block hash -> (block number, verdict).
    verdicts: Mutex<HashMap<B256, (u64, Verdict)>>,
    /// Serialises commits (the only chain writer in consensus modes).
    commit_lock: Mutex<()>,
    /// Committed blocks are published here (`eth_subscribe`); set once.
    events: OnceLock<Arc<ChainEvents>>,
    /// Encrypted mempool (Phase 5 prototype); set once.
    encryption: OnceLock<Arc<Encryption>>,
}

/// The node's [`ConsensusApp`] (see the module docs). Cheap to clone.
#[derive(Clone)]
pub struct ChainApp {
    inner: Arc<Inner>,
    /// Dedicated threads for every blocking step (see the module docs).
    workers: Arc<WorkerPool>,
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
    /// A chain app over an initialised database. `set` is the genesis validator set: in
    /// force at every height (Phase 2), or until the first epoch boundary once the
    /// staking-driven set is scheduled (Phase 3).
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
                epoch_sets: Mutex::new(BTreeMap::new()),
                evidence: EvidencePool::default(),
                app,
                verdicts: Mutex::new(HashMap::new()),
                commit_lock: Mutex::new(()),
                events: OnceLock::new(),
                encryption: OnceLock::new(),
            }),
            workers: Arc::new(
                WorkerPool::new("xdevs-consensus", CONSENSUS_THREADS)
                    .expect("failed to spawn the consensus worker threads"),
            ),
        }
    }

    /// Publish every newly committed block to `events` (`newHeads` / `logs`
    /// subscriptions). Only the first call has an effect.
    pub fn with_events(self, events: Arc<ChainEvents>) -> Self {
        let _ = self.inner.events.set(events);
        self
    }

    /// Enable the encrypted mempool (share pool, own key share, gossip). Only the first
    /// call has an effect. Required when the chain config schedules
    /// `encryptedMempoolBlock`.
    pub fn with_encryption(self, encryption: Arc<Encryption>) -> Self {
        let _ = self.inner.encryption.set(encryption);
        self
    }

    /// The validator set in force at `height` (see [`crate::staking`]).
    pub async fn validator_set_at(&self, height: u64) -> anyhow::Result<ValidatorSet> {
        self.blocking(move |s| s.set_at(height)).await
    }

    /// Verify double-sign evidence (`xdevs_consensus::ConflictingVotes` RLP) and queue it
    /// for inclusion in a block. Returns the evidence hash.
    pub async fn submit_evidence(&self, raw: &[u8]) -> anyhow::Result<B256> {
        let evidence = ConflictingVotes::from_bytes(raw)
            .map_err(|e| anyhow::anyhow!("undecodable evidence: {e}"))?;
        self.blocking(move |s| s.add_evidence(evidence)).await
    }

    /// Double-sign evidence waiting for inclusion.
    pub fn pending_evidence(&self) -> usize {
        self.inner.evidence.len()
    }

    /// Run `f` on a consensus worker thread.
    async fn blocking<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> anyhow::Result<T> + Send + 'static,
    {
        let inner = self.inner.clone();
        self.workers
            .run(move || f(&inner))
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

    /// The validator set in force at `height` (cached per epoch).
    fn set_at(&self, height: u64) -> anyhow::Result<ValidatorSet> {
        let Some(boundary) = self.cfg.validator_set_boundary(height) else {
            return Ok(self.set.clone());
        };
        if let Some(set) = self.epoch_sets.lock().get(&boundary) {
            return Ok(set.clone());
        }
        let set = staking::validator_set_at(&*self.db, &self.cfg, &self.set, height)?;
        let mut cache = self.epoch_sets.lock();
        // A handful of epochs is plenty (current, previous, evidence heights).
        while cache.len() >= 8 {
            cache.pop_first();
        }
        cache.insert(boundary, set.clone());
        Ok(set)
    }

    /// Verify `evidence` against the validator set of its height and pool it.
    fn add_evidence(&self, evidence: ConflictingVotes) -> anyhow::Result<B256> {
        let evidence = evidence.canonical();
        let height = evidence.first.height;
        let (head, _) = chain::latest_header(&*self.db)?;
        ensure!(
            (1..=head + 1).contains(&height),
            "evidence height {height} is not at or below the next height {}",
            head + 1
        );
        ensure!(
            head + 1 - height <= MAX_EVIDENCE_AGE_BLOCKS,
            "evidence height {height} is older than {MAX_EVIDENCE_AGE_BLOCKS} blocks"
        );
        let set = self.set_at(height)?;
        ensure!(
            evidence.is_valid(self.cfg.chain_id, &set),
            "invalid double-sign evidence: the votes do not conflict or the signatures of \
             a validator at height {height} do not verify"
        );
        let hash = evidence.hash();
        if self.cfg.staking_validators_block.is_none() {
            tracing::warn!(target: LOG, validator = %evidence.first.validator, height, %hash,
                           "double-sign evidence verified; not slashable (staking-driven set not scheduled)");
            return Ok(hash);
        }
        if self.evidence.add(evidence) {
            tracing::warn!(target: LOG, validator = %evidence.first.validator, height, %hash,
                           "double-sign evidence verified and queued for slashing");
        }
        Ok(hash)
    }

    /// Consensus data of the block this node proposes at `height` (Phase 3).
    fn consensus_data(
        &self,
        snapshot: &dyn DbRead,
        height: u64,
    ) -> anyhow::Result<BlockConsensusData> {
        let proposer = self
            .app
            .validator_address
            .context("only a validator proposes blocks")?;
        let last_commit = if height >= 2 {
            let raw = chain::commit_certificate(snapshot, height - 1)?
                .with_context(|| format!("no commit certificate for block {}", height - 1))?;
            vec![CommitCertificate::from_bytes(&raw).map_err(|e| {
                anyhow::anyhow!("stored certificate of block {}: {e:?}", height - 1)
            })?]
        } else {
            Vec::new()
        };
        let oldest = height.saturating_sub(MAX_EVIDENCE_AGE_BLOCKS).max(1);
        let evidence = self.evidence.select(MAX_EVIDENCE_PER_BLOCK, |e| {
            (oldest..height).contains(&e.first.height)
        });
        Ok(BlockConsensusData {
            proposer,
            last_commit,
            evidence,
        })
    }

    /// The consensus-data rules of block `height` (child of `parent_hash`) and the Staking
    /// inputs they yield. `Ok(Err(reason))` = the block is invalid; `Err` = local failure.
    fn check_consensus_data(
        &self,
        height: u64,
        parent_hash: B256,
        data: &BlockConsensusData,
    ) -> anyhow::Result<Result<StakingInputs, String>> {
        // The parent's commit certificate: its signers are the liveness bits.
        let mut signers = Vec::new();
        match (height, data.last_commit.as_slice()) {
            (1, []) => {}
            (1, _) => return Ok(Err("block 1 cannot carry a last commit".into())),
            (_, [cert]) => {
                if cert.height != height - 1 || cert.block_hash != parent_hash {
                    return Ok(Err(format!(
                        "last commit is for block {} {}, parent is {} {parent_hash}",
                        cert.height,
                        cert.block_hash,
                        height - 1
                    )));
                }
                let set = self.set_at(height - 1)?;
                if let Err(e) = cert.verify(self.cfg.chain_id, &set) {
                    return Ok(Err(format!("last commit does not verify: {e}")));
                }
                signers = cert
                    .signatures
                    .iter()
                    .filter_map(|s| set.get(&s.validator))
                    .map(|v| B256::from(v.public_key.0))
                    .collect();
            }
            (_, list) => {
                return Ok(Err(format!(
                    "{} last commits, expected exactly one",
                    list.len()
                )))
            }
        }
        // The proposer must be a member of the set of this height.
        let set = self.set_at(height)?;
        let Some(proposer) = set.get(&data.proposer) else {
            return Ok(Err(format!(
                "proposer {} is not a validator at height {height}",
                data.proposer
            )));
        };
        let proposer = B256::from(proposer.public_key.0);
        // Evidence: bounded, canonical, unique, recent and valid.
        if data.evidence.len() > MAX_EVIDENCE_PER_BLOCK {
            return Ok(Err(format!(
                "{} evidence items, at most {MAX_EVIDENCE_PER_BLOCK}",
                data.evidence.len()
            )));
        }
        let mut seen = BTreeSet::new();
        let mut evidence = Vec::with_capacity(data.evidence.len());
        for ev in &data.evidence {
            let h = ev.first.height;
            if !(1..height).contains(&h) || height - h > MAX_EVIDENCE_AGE_BLOCKS {
                return Ok(Err(format!("evidence height {h} out of range")));
            }
            if *ev != ev.canonical() {
                return Ok(Err("evidence votes not in canonical order".into()));
            }
            let hash = ev.hash();
            if !seen.insert(hash) {
                return Ok(Err(format!("duplicate evidence {hash}")));
            }
            let ev_set = self.set_at(h)?;
            let Some(offender) = ev_set
                .get(&ev.first.validator)
                .filter(|_| ev.is_valid(self.cfg.chain_id, &ev_set))
            else {
                return Ok(Err(format!("invalid evidence {hash}")));
            };
            evidence.push(SlashEvidence {
                consensus_key: B256::from(offender.public_key.0),
                height: h,
                reference: hash,
            });
        }
        Ok(Ok(StakingInputs {
            proposer: Some(proposer),
            signers,
            evidence,
        }))
    }

    fn encryption(&self) -> anyhow::Result<&Encryption> {
        self.encryption.get().map(|e| &**e).context(
            "the encrypted mempool is active but this node has no committee key configured",
        )
    }

    /// The decryptable envelopes of block `height - 1` with its hash, `None` when there
    /// is nothing to decrypt. Registers the block with the share pool and releases this
    /// node's own shares if that has not happened yet (e.g. after a restart).
    fn parent_envelopes(
        &self,
        snapshot: &dyn DbRead,
        height: u64,
    ) -> anyhow::Result<Option<(B256, Vec<BlockEnvelope>)>> {
        if height < 2 || !self.cfg.encrypted_mempool_active(height - 1) {
            return Ok(None);
        }
        let enc = self.encryption()?;
        let parent = chain::block(snapshot, height - 1)?
            .with_context(|| format!("missing block {}", height - 1))?;
        let hash = parent.hash();
        enc.on_committed(&self.cfg, &parent, hash);
        let envs = encrypted::decryptable_envelopes(&self.cfg, enc.pool.key(), &parent);
        Ok((!envs.is_empty()).then_some((hash, envs)))
    }

    /// `true` once this node holds `t` shares for every envelope of the parent of
    /// `height` (or there is nothing to decrypt).
    fn decryption_ready(&self, height: u64) -> anyhow::Result<bool> {
        if height < 2 || !self.cfg.encrypted_mempool_active(height - 1) {
            return Ok(true);
        }
        let snapshot = self.db.snapshot()?;
        let Some((hash, _)) = self.parent_envelopes(&*snapshot, height)? else {
            return Ok(true);
        };
        Ok(self
            .encryption()?
            .pool
            .selection(height - 1, hash)
            .is_some())
    }

    /// The decryption data and decrypted transactions of the block this node proposes at
    /// `height` (`None` = the parent has no decryptable envelope).
    fn own_decryption(
        &self,
        snapshot: &dyn DbRead,
        height: u64,
    ) -> anyhow::Result<Option<(DecryptionData, Vec<RecoveredTx>)>> {
        let Some((hash, envs)) = self.parent_envelopes(snapshot, height)? else {
            return Ok(None);
        };
        let enc = self.encryption()?;
        let envelopes = enc.pool.selection(height - 1, hash).with_context(|| {
            format!(
                "not enough decryption shares yet for the {} envelopes of block {}",
                envs.len(),
                height - 1
            )
        })?;
        let data = DecryptionData {
            block_hash: hash,
            envelopes,
        };
        let txs = encrypted::decrypted_candidates(&self.cfg, enc.pool.key(), &envs, &data)
            .map_err(|r| anyhow::anyhow!("own decryption data is invalid: {r}"))?;
        tracing::info!(target: LOG, height, envelopes = envs.len(), decrypted = txs.len(),
                       "decrypting the parent's envelopes first");
        Ok(Some((data, txs)))
    }

    /// The encrypted-mempool rule of block `height`: it carries the decryption of every
    /// decryptable envelope of its parent (and nothing otherwise), and its body starts
    /// with exactly the decrypted transactions the executor includes, in committed order.
    /// `Ok(Err(reason))` = invalid block; `Err` = local failure.
    fn check_decryption(
        &self,
        height: u64,
        parent: &Header,
        block: &Block,
        snapshot: &dyn DbRead,
        staking: &StakingInputs,
    ) -> anyhow::Result<Result<(), String>> {
        if !self.cfg.encrypted_mempool_active(height) {
            return Ok(Ok(()));
        }
        let decryption = match encrypted::split_block_data(&self.cfg, height, &block.consensus) {
            Ok((_, d)) => d,
            Err(reason) => return Ok(Err(reason)),
        };
        let expected = if height >= 2 && self.cfg.encrypted_mempool_active(height - 1) {
            let enc = self.encryption()?;
            let parent_block = chain::block(snapshot, height - 1)?
                .with_context(|| format!("missing block {}", height - 1))?;
            encrypted::decryptable_envelopes(&self.cfg, enc.pool.key(), &parent_block)
        } else {
            Vec::new()
        };
        let prefix = match (expected.is_empty(), decryption) {
            (true, None) => return Ok(Ok(())),
            (true, Some(_)) => {
                return Ok(Err(
                    "decryption data but the parent has no decryptable envelope".into(),
                ))
            }
            (false, None) => {
                return Ok(Err(format!(
                    "missing the decryption of the {} envelopes of block {}",
                    expected.len(),
                    height - 1
                )))
            }
            (false, Some(data)) => {
                if data.block_hash != parent.hash_slow() {
                    return Ok(Err("decryption data for another block".into()));
                }
                let key = self.encryption()?.pool.key();
                match encrypted::decrypted_candidates(&self.cfg, key, &expected, &data) {
                    Ok(txs) => txs,
                    Err(reason) => return Ok(Err(reason)),
                }
            }
        };
        if prefix.is_empty() {
            return Ok(Ok(()));
        }
        // Rebuild the prefix with the executor's skip rules on the same state and header.
        let h = &block.header;
        let attrs = BlockAttributes {
            timestamp: h.timestamp,
            beneficiary: h.beneficiary,
            prev_randao: h.mix_hash,
            extra_data: h.extra_data.clone(),
            gas_limit: h.gas_limit,
        };
        let state = DbStateProvider::new(snapshot);
        let built = match self
            .executor
            .build_with(parent, attrs, prefix, &state, staking)
        {
            Ok(b) => b,
            Err(ExecutionError::State(e)) => bail!("state read failed: {e}"),
            Err(e) => return Ok(Err(format!("decrypted prefix does not build: {e}"))),
        };
        let want: Vec<B256> = built.block.tx_hashes().collect();
        let have: Vec<B256> = block.tx_hashes().take(want.len()).collect();
        if have != want {
            return Ok(Err(format!(
                "the body does not start with the {} decrypted transactions of block {} in \
                 committed order",
                want.len(),
                height - 1
            )));
        }
        Ok(Ok(()))
    }

    /// Build and execute block `height` on the committed head (no commit).
    fn build(&self, height: u64, with_txs: bool) -> anyhow::Result<Block> {
        let snapshot = self.db.snapshot()?;
        let (head, parent) = chain::latest_header(&*snapshot)?;
        ensure!(
            height == head + 1,
            "cannot propose height {height}: committed head is {head}"
        );
        // Phase 3: consensus data and the Staking inputs it yields (checked like any
        // other proposal, so a bad local certificate or evidence never reaches a block).
        let consensus = if self.cfg.staking_active(height) {
            let data = self.consensus_data(&*snapshot, height)?;
            let inputs = match self.check_consensus_data(height, parent.hash_slow(), &data)? {
                Ok(inputs) => inputs,
                Err(reason) => bail!("own consensus data for block {height} is invalid: {reason}"),
            };
            Some((data, inputs))
        } else {
            None
        };
        let inputs = consensus
            .as_ref()
            .map(|(_, i)| i.clone())
            .unwrap_or_default();
        // Phase 5: the parent's envelopes, decrypted, go first.
        let decryption = self.own_decryption(&*snapshot, height)?;
        let prefix = decryption
            .as_ref()
            .map(|(_, txs)| txs.clone())
            .unwrap_or_default();
        let (_, mut built) = build_next_block(
            &*snapshot,
            &self.pool,
            &self.executor,
            &self.cfg,
            &NextBlock {
                now_secs: unix_now(),
                beneficiary: self.app.fee_recipient,
                extra_data: self.app.extra_data.clone(),
                with_txs,
                prefix,
            },
            &inputs,
        )?;
        drop(snapshot);
        let staking_data = consensus
            .map(|(data, _)| Bytes::from(data.to_bytes()))
            .unwrap_or_default();
        let bytes = encrypted::join_block_data(
            &self.cfg,
            height,
            staking_data,
            decryption.as_ref().map(|(d, _)| d),
        );
        if !bytes.is_empty() {
            built.block.header.requests_hash = Some(consensus_data_hash(&bytes));
            built.block.consensus = bytes;
        }
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
        // The cache is keyed by the header hash: the body (and the consensus data) must
        // belong to that header first (a proposer could pair a known header with another
        // body).
        if !body_matches_header(block) {
            return Ok(Verdict::invalid(
                "transactions root does not match the body (or requests hash the consensus data)",
            ));
        }
        let hash = block.hash();
        // Cached verdicts are deterministic (shape / execution). A cached valid block is
        // not re-checked against the clock (module docs: polka'd re-proposals).
        if let Some(v) = self.cached(&hash) {
            return Ok(v);
        }
        // Rule 3, live proposals only. NEVER cached: it depends on the local clock. A
        // proposer must use at least `parent + 1`, so that is always timely.
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
        let inputs = match self.check_shape(height, &parent, block) {
            Err(reason) => Err(reason),
            Ok(()) => self.staking_inputs(height, &parent, block)?,
        };
        let verdict = match inputs {
            Err(reason) => Verdict::invalid(reason),
            Ok(inputs) => {
                match self.check_decryption(height, &parent, block, &*snapshot, &inputs)? {
                    Err(reason) => Verdict::invalid(reason),
                    Ok(()) => {
                        let state = DbStateProvider::new(&*snapshot);
                        match self.executor.execute_with(&parent, block, &state, &inputs) {
                            Ok(outcome) => match compare_outcome(header, &outcome) {
                                Ok(()) => Verdict::Valid(Arc::new(outcome)),
                                Err(reason) => Verdict::invalid(reason),
                            },
                            // A storage failure says nothing about the block.
                            Err(ExecutionError::State(e)) => bail!("state read failed: {e}"),
                            // Never happens for blocks (no limits); not a verdict either way.
                            Err(e @ ExecutionError::Aborted(_)) => bail!("block execution {e}"),
                            Err(e) => Verdict::invalid(e),
                        }
                    }
                }
            }
        };
        self.remember(header.number, hash, verdict.clone());
        Ok(verdict)
    }

    /// The Staking inputs of `block` (Phase 3): its consensus data is required from the
    /// activation on and forbidden before. `Ok(Err(reason))` = invalid block.
    fn staking_inputs(
        &self,
        height: u64,
        parent: &Header,
        block: &Block,
    ) -> anyhow::Result<Result<StakingInputs, String>> {
        // From the encrypted-mempool activation on, `Block::consensus` also carries the
        // decryption data (checked by `check_decryption`).
        let staking_data = match encrypted::split_block_data(&self.cfg, height, &block.consensus) {
            Ok((s, _)) => s,
            Err(reason) => return Ok(Err(reason)),
        };
        if !self.cfg.staking_active(height) {
            if !staking_data.is_empty() {
                return Ok(Err("consensus data before the staking activation".into()));
            }
            return Ok(Ok(StakingInputs::default()));
        }
        if staking_data.is_empty() {
            return Ok(Err(
                "missing consensus data (staking-driven validator set active)".into(),
            ));
        }
        let data = match BlockConsensusData::from_bytes(&staking_data) {
            Ok(data) => data,
            Err(e) => return Ok(Err(format!("undecodable consensus data: {e}"))),
        };
        self.check_consensus_data(height, parent.hash_slow(), &data)
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
        if !block.consensus.is_empty() {
            // Bound to the consensus data instead (checked with the body).
            compat.requests_hash = h.requests_hash;
        }
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
        cert.verify_at(self.cfg.chain_id, number, &self.set_at(number)?)
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
        // Phase 3: an epoch boundary fixes the set of the next epoch (the current set is
        // kept if the contract has no usable member: the chain must not lose its set).
        let next_set = if self.cfg.is_epoch_boundary(number) {
            let set = match outcome
                .next_validators
                .as_deref()
                .and_then(staking::set_from_staking)
            {
                Some(set) => set,
                None => {
                    tracing::error!(target: LOG, number,
                                    "Staking returned no usable validator; keeping the current set");
                    self.set_at(number)?
                }
            };
            Some(set)
        } else {
            None
        };
        chain::commit_consensus_block(
            &*self.db,
            &block,
            &outcome,
            Some(&cert.to_bytes()),
            next_set.as_ref().map(staking::encode_set).as_deref(),
            self.app.state_history,
        )?;
        if let Some(set) = next_set {
            tracing::info!(
                target: LOG,
                epoch_block = number,
                first_height = number + 1,
                validators = set.len(),
                total_power = set.total_voting_power(),
                "epoch boundary: validator set of the next epoch fixed"
            );
            self.epoch_sets.lock().insert(number, set);
        }
        let staking_data = encrypted::split_block_data(&self.cfg, number, &block.consensus)
            .map(|(s, _)| s)
            .unwrap_or_default();
        if !staking_data.is_empty() {
            let included: Vec<B256> = BlockConsensusData::from_bytes(&staking_data)
                .map(|d| d.evidence.iter().map(ConflictingVotes::hash).collect())
                .unwrap_or_default();
            self.evidence
                .prune(&included, number.saturating_sub(MAX_EVIDENCE_AGE_BLOCKS));
        }
        crate::events::publish_block(
            self.events.get().map(|e| &**e),
            &block,
            hash,
            &outcome.receipts,
        );
        after_commit(&self.pool, &self.cfg, &block, &outcome.receipts);
        // Phase 5: release this node's decryption shares for the block's envelopes.
        if let Some(enc) = self.encryption.get() {
            enc.on_committed(&self.cfg, &block, hash);
        }
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

/// The body hashes to the header's `transactions_root` and the consensus data to its
/// `requests_hash`.
fn body_matches_header(block: &Block) -> bool {
    block.header.transactions_root == block.compute_transactions_root()
        && block.consensus_matches_header()
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

    async fn validator_set(&self, height: u64) -> anyhow::Result<ValidatorSet> {
        if self.inner.cfg.staking_validators_block.is_none() {
            return Ok(self.inner.set.clone());
        }
        self.validator_set_at(height).await
    }

    async fn propose(&self, height: u64, round: u32, deadline: Instant) -> anyhow::Result<Block> {
        // Phase 5: the block must decrypt its parent's envelopes; wait (at most half the
        // time to the deadline) for the committee's shares to arrive by gossip.
        let wait_until = Instant::now() + deadline.saturating_duration_since(Instant::now()) / 2;
        loop {
            let ready = self
                .blocking(move |s| s.decryption_ready(height))
                .await
                .unwrap_or(true);
            if ready || Instant::now() >= wait_until {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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
        if let Evidence::DoubleVote(votes) = e {
            // Slashable from the staking activation on: verify and queue for inclusion.
            self.blocking(move |s| s.add_evidence(votes)).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "app_staking_tests.rs"]
mod staking_tests;

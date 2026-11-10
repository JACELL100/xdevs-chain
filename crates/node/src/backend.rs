//! [`RpcBackend`] over storage + state + mempool + executor (plan §19.1).
//!
//! Every request runs on a blocking thread against one database snapshot, so all
//! reads of a request see the same committed block even while the producer commits.
//!
//! DoS limits (security review "RPC load can starve consensus"): at most
//! `max_concurrent` requests do blocking work at once (default 2 x CPUs; others wait up
//! to [`RPC_QUEUE_TIMEOUT`] for a slot, then fail with -32005), and `eth_call` /
//! `eth_estimateGas` stop after `evm_timeout` of EVM time (default 5 s, like geth) or as
//! soon as the request is dropped (client gone), failing with "execution aborted".
//! Consensus never shares these threads (see `ChainApp`).
//! State queries at past blocks read the reverse state diffs of the recent blocks
//! (`[chain] pruning` / `state_history_blocks`, default 10,000 blocks); older blocks
//! fail with a geth-style "-32000" error.

use crate::chain::{self, Db};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use xdevs_consensus::{CommitCertificate, EngineStatus};
use xdevs_crypto::decode_and_recover;
use xdevs_execution::{next_base_fee, CallLimits, CallResult, EvmExecutor, ExecutionError};
use xdevs_mempool::{Pool, PoolError, TxOrigin, TxPool};
use xdevs_network::NetworkHandle;
use xdevs_primitives::reexports::alloy_primitives::U64;
use xdevs_primitives::{
    xip0001::{Xip0001Error, LOCK_SLOT},
    Address, Block, Bytes, ChainConfig, Header, ReceiptEnvelope, RecoveredTx, Transaction,
    TxEnvelope, TxKind, B256, U256,
};
use xdevs_rpc::{
    check_logs_limit,
    convert::{self, ReceiptMeta, TxMeta},
    types::*,
    BackendResult, BlockEvent, ChainEvents, RpcBackend, RpcBackendError,
};
use xdevs_state::{
    history::oldest_available_state, DbStateProvider, HistoricalStateProvider, StateProvider,
};
use xdevs_storage::{DbError, DbRead};

/// Fallback suggested priority fee when recent blocks carry no transactions
/// (0.001 gwei, geth's default minimum suggestion).
pub const DEFAULT_PRIORITY_FEE: u128 = 1_000_000;
/// Blocks sampled by the priority-fee oracle.
pub const TIP_ORACLE_BLOCKS: u64 = 20;

fn internal(e: impl std::fmt::Display) -> RpcBackendError {
    RpcBackendError::Internal(e.to_string())
}

/// Shorthand: run a storage read and map its error.
fn db<T>(r: Result<T, DbError>) -> BackendResult<T> {
    r.map_err(internal)
}

type Snapshot<'a> = dyn DbRead + Send + Sync + 'a;

/// Default EVM time limit of `eth_call` / `eth_estimateGas` (geth's `rpc.evmtimeout`).
pub const DEFAULT_EVM_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a request waits for a blocking-work slot before failing with -32005.
pub const RPC_QUEUE_TIMEOUT: Duration = Duration::from_secs(10);

/// Default cap on concurrent blocking RPC work: 2 x available CPUs (at least 4).
pub fn default_max_concurrent_requests() -> usize {
    std::thread::available_parallelism()
        .map_or(4, |n| n.get() * 2)
        .max(4)
}

/// Sets the cancel flag of a simulation when the request future is dropped (the client
/// disconnected, or the server gave up on it), so the EVM stops instead of running on.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A node is reported as syncing (`eth_syncing`) while peers are at least this many
/// blocks ahead of its head.
pub const SYNC_LAG_THRESHOLD: u64 = 2;

/// The node's RPC backend.
pub struct NodeBackend {
    db: Db,
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
    cfg: ChainConfig,
    gas_cap: u64,
    /// Tx gossip (validator / full-node mode): accepted txs are published.
    gossip: Option<Arc<NetworkHandle>>,
    /// Consensus engine status (validator / full-node mode).
    consensus: Option<EngineStatus>,
    /// Head when the node started (`eth_syncing.startingBlock`).
    starting_block: u64,
    /// Chain events for `eth_subscribe`; accepted txs are published as pending.
    events: Option<Arc<ChainEvents>>,
    /// Slots for concurrent blocking work (shared by every RPC server of the node).
    slots: Arc<Semaphore>,
    max_concurrent: usize,
    /// EVM time limit of `eth_call` / `eth_estimateGas` (`None` = unlimited).
    evm_timeout: Option<Duration>,
}

impl std::fmt::Debug for NodeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeBackend")
            .field("chain_id", &self.cfg.chain_id)
            .field("gas_cap", &self.gas_cap)
            .finish_non_exhaustive()
    }
}

/// Everything a blocking request handler needs (cheap to clone).
#[derive(Clone)]
struct Ctx {
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
    cfg: ChainConfig,
    gas_cap: u64,
    evm_timeout: Option<Duration>,
}

impl Ctx {
    /// Limits of one simulation: the EVM time limit from now, plus `cancel`.
    fn call_limits(&self, cancel: Arc<AtomicBool>) -> CallLimits {
        CallLimits {
            deadline: self.evm_timeout.map(|t| std::time::Instant::now() + t),
            cancel: Some(cancel),
        }
    }

    /// Execution error -> RPC error, with geth's wording for an aborted simulation.
    fn exec_err(&self, e: ExecutionError) -> RpcBackendError {
        match e {
            ExecutionError::Aborted(reason) => {
                RpcBackendError::Rejected(match (reason, self.evm_timeout) {
                    ("timeout", Some(t)) => {
                        format!("execution aborted (timeout = {}s)", t.as_secs_f64())
                    }
                    (reason, _) => format!("execution aborted ({reason})"),
                })
            }
            other => exec_err(other),
        }
    }
}

impl NodeBackend {
    /// New backend; `gas_cap` bounds `eth_call` / `eth_estimateGas` (0 = unlimited).
    pub fn new(
        db: Db,
        pool: Arc<Pool>,
        executor: Arc<EvmExecutor>,
        cfg: ChainConfig,
        gas_cap: u64,
    ) -> Self {
        Self {
            db,
            pool,
            executor,
            cfg,
            gas_cap,
            gossip: None,
            consensus: None,
            starting_block: 0,
            events: None,
            slots: Arc::new(Semaphore::new(default_max_concurrent_requests())),
            max_concurrent: default_max_concurrent_requests(),
            evm_timeout: Some(DEFAULT_EVM_TIMEOUT),
        }
    }

    /// DoS limits: at most `max_concurrent` requests doing blocking work at once (0 =
    /// [`default_max_concurrent_requests`]) and an EVM time limit for `eth_call` /
    /// `eth_estimateGas` (`None` or zero = unlimited).
    pub fn with_limits(mut self, max_concurrent: usize, evm_timeout: Option<Duration>) -> Self {
        let max = match max_concurrent {
            0 => default_max_concurrent_requests(),
            n => n,
        };
        self.slots = Arc::new(Semaphore::new(max));
        self.max_concurrent = max;
        self.evm_timeout = evm_timeout.filter(|t| !t.is_zero());
        self
    }

    /// Serve `eth_subscribe` from `events` and publish the hash of every
    /// `eth_sendRawTransaction` tx admitted to the pool.
    pub fn with_events(mut self, events: Arc<ChainEvents>) -> Self {
        self.events = Some(events);
        self
    }

    /// Publish accepted `eth_sendRawTransaction` txs on the tx-gossip network and count
    /// its peers in `net_peerCount`.
    pub fn with_gossip(mut self, gossip: Arc<NetworkHandle>) -> Self {
        self.gossip = Some(gossip);
        self
    }

    /// Serve `eth_syncing` and `net_peerCount` from a running consensus engine;
    /// `starting_block` = head at start-up.
    pub fn with_consensus(mut self, status: EngineStatus, starting_block: u64) -> Self {
        self.consensus = Some(status);
        self.starting_block = starting_block;
        self
    }

    /// A blocking-work slot; fails with -32005 after [`RPC_QUEUE_TIMEOUT`].
    async fn slot(&self) -> BackendResult<OwnedSemaphorePermit> {
        match tokio::time::timeout(RPC_QUEUE_TIMEOUT, self.slots.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(internal("request slots closed")),
            Err(_) => Err(RpcBackendError::LimitExceeded(format!(
                "server busy: {} requests already running, try again later",
                self.max_concurrent
            ))),
        }
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            pool: self.pool.clone(),
            executor: self.executor.clone(),
            cfg: self.cfg.clone(),
            gas_cap: self.gas_cap,
            evm_timeout: self.evm_timeout,
        }
    }

    /// Run `f` on a blocking thread (once a work slot is free) with a consistent
    /// database snapshot.
    async fn read<T, F>(&self, f: F) -> BackendResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Snapshot<'_>, &Ctx) -> BackendResult<T> + Send + 'static,
    {
        let permit = self.slot().await?;
        let db = self.db.clone();
        let ctx = self.ctx();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let snapshot = db.snapshot().map_err(internal)?;
            f(&*snapshot, &ctx)
        })
        .await
        .map_err(|e| internal(format!("request task failed: {e}")))?
    }

    /// [`NodeBackend::read`] for an EVM simulation: `f` gets the [`CallLimits`] (the EVM
    /// time limit from when it starts; cancelled if this future is dropped).
    async fn simulate<T, F>(&self, f: F) -> BackendResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Snapshot<'_>, &Ctx, &CallLimits) -> BackendResult<T> + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(cancel.clone());
        self.read(move |s, ctx| f(s, ctx, &ctx.call_limits(cancel)))
            .await
    }
}

// ---------------------------------------------------------------------------
// Block resolution
// ---------------------------------------------------------------------------

/// Block number for a tag (`None` = beyond the head / unknown).
fn resolve_number(snap: &Snapshot<'_>, tag: BlockNumberOrTag) -> BackendResult<Option<u64>> {
    let head = db(chain::latest_header(snap))?.0;
    Ok(match tag {
        BlockNumberOrTag::Earliest => Some(0),
        BlockNumberOrTag::Number(n) => (n <= head).then_some(n),
        // latest / safe / finalized / pending: BFT finality, no pending block.
        _ => Some(head),
    })
}

/// Block number for a [`BlockId`] (`None` = unknown).
fn resolve_block_id(snap: &Snapshot<'_>, id: BlockId) -> BackendResult<Option<u64>> {
    match id {
        BlockId::Number(tag) => resolve_number(snap, tag),
        BlockId::Hash(h) => db(chain::number_by_hash(snap, h.block_hash)),
    }
}

/// State after block `id` and that block's header. The latest block reads the flat
/// state directly; recent past blocks go through the reverse state diffs; blocks
/// older than the kept history fail with -32000.
fn state_at<'a>(
    snap: &'a Snapshot<'a>,
    id: BlockId,
) -> BackendResult<(Header, Box<dyn StateProvider + 'a>)> {
    let (head_number, head) = db(chain::latest_header(snap))?;
    let number = resolve_block_id(snap, id)?.ok_or(RpcBackendError::UnknownBlock)?;
    if number == head_number {
        return Ok((head, Box::new(DbStateProvider::new(snap))));
    }
    let oldest = db(oldest_available_state(snap, head_number))?;
    if number < oldest {
        return Err(RpcBackendError::Rejected(format!(
            "historical state not available: block {number} (this node keeps the state \
             of blocks {oldest}..={head_number}; raise [chain] state_history_blocks or use \
             pruning = \"archive\")"
        )));
    }
    let header = db(chain::header(snap, number))?
        .ok_or_else(|| internal(format!("missing header of block {number}")))?;
    Ok((header, Box::new(HistoricalStateProvider::new(snap, number))))
}

fn state_err(e: xdevs_state::StateError) -> RpcBackendError {
    internal(e)
}

// ---------------------------------------------------------------------------
// Execution errors
// ---------------------------------------------------------------------------

/// Decode an ABI `Error(string)` revert payload.
pub fn decode_revert_reason(data: &[u8]) -> Option<String> {
    const SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
    let body = data.strip_prefix(&SELECTOR)?;
    let word = |i: usize| -> Option<usize> {
        let w = body.get(i..i + 32)?;
        if w[..24].iter().any(|b| *b != 0) {
            return None;
        }
        Some(u64::from_be_bytes(w[24..].try_into().ok()?) as usize)
    };
    let offset = word(0)?;
    let len = word(offset)?;
    let start = offset.checked_add(32)?;
    let bytes = body.get(start..start.checked_add(len)?)?;
    String::from_utf8(bytes.to_vec()).ok()
}

fn reverted(output: Bytes) -> RpcBackendError {
    RpcBackendError::Reverted {
        reason: decode_revert_reason(&output),
        data: output,
    }
}

/// XIP-0001 rejection: JSON-RPC -32003 with `data.reason` (spec §5 f).
fn xip0001_rejected(e: &Xip0001Error) -> RpcBackendError {
    RpcBackendError::TxRejected {
        reason: e.violation.reason().to_string(),
        message: e.to_string(),
    }
}

fn pool_err(e: PoolError) -> RpcBackendError {
    match e {
        PoolError::Xip0001(e) => xip0001_rejected(&e),
        other => RpcBackendError::Rejected(other.to_string()),
    }
}

fn exec_err(e: ExecutionError) -> RpcBackendError {
    match e {
        ExecutionError::Reverted { output } => reverted(output),
        ExecutionError::Xip0001 { error, .. } => xip0001_rejected(&error),
        ExecutionError::State(s) => RpcBackendError::Internal(s),
        ExecutionError::InvalidRequest(s) | ExecutionError::Evm(s) => RpcBackendError::Rejected(s),
        other => RpcBackendError::Rejected(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Block / tx / receipt assembly
// ---------------------------------------------------------------------------

fn tx_meta(header: &Header, hash: B256, index: u64) -> TxMeta {
    TxMeta {
        block_hash: hash,
        block_number: header.number,
        index,
        base_fee: header.base_fee_per_gas,
        timestamp: header.timestamp,
    }
}

fn rpc_block_at(snap: &Snapshot<'_>, number: u64, full: bool) -> BackendResult<Option<RpcBlock>> {
    let Some(block) = db(chain::block(snap, number))? else {
        return Ok(None);
    };
    let hash = block.header.hash_slow();
    let senders = if full {
        db(chain::recover_senders(&block.body))?
    } else {
        Vec::new()
    };
    Ok(Some(convert::rpc_block(&block, hash, &senders, full)))
}

/// RPC receipts of block `number` (all of them, or only `only` index).
fn rpc_receipts_at(
    snap: &Snapshot<'_>,
    number: u64,
    only: Option<u64>,
) -> BackendResult<Option<Vec<RpcReceipt>>> {
    let Some(block) = db(chain::block(snap, number))? else {
        return Ok(None);
    };
    let receipts = db(chain::receipts(snap, number))?;
    if receipts.len() != block.body.len() {
        return Err(internal(format!(
            "block {number}: {} receipts for {} txs",
            receipts.len(),
            block.body.len()
        )));
    }
    let hash = block.header.hash_slow();
    let mut out = Vec::new();
    let mut prev_cumulative = 0u64;
    let mut log_index = 0u64;
    for (i, (tx, receipt)) in block.body.iter().zip(&receipts).enumerate() {
        let i = i as u64;
        let gas_used = receipt.cumulative_gas_used() - prev_cumulative;
        prev_cumulative = receipt.cumulative_gas_used();
        let first_log_index = log_index;
        log_index += receipt.logs().len() as u64;
        if only.is_some_and(|o| o != i) {
            continue;
        }
        let rtx = RecoveredTx::recover(tx.clone()).map_err(internal)?;
        let contract_address = match tx.kind() {
            TxKind::Create => Some(rtx.sender.create(tx.nonce())),
            TxKind::Call(_) => None,
        };
        out.push(convert::rpc_receipt(
            &rtx,
            receipt,
            tx_meta(&block.header, hash, i),
            ReceiptMeta {
                gas_used,
                first_log_index,
                contract_address,
            },
        ));
    }
    Ok(Some(out))
}

/// Append the logs of block `number` matching `filter` to `out`. Fails with
/// `LimitExceeded` as soon as `out` would hold more than `limit` logs, so a huge
/// range never buffers more than `limit + 1` results (plan §19.3).
fn logs_in_block(
    snap: &Snapshot<'_>,
    number: u64,
    filter: &Filter,
    limit: Option<usize>,
    out: &mut Vec<RpcLog>,
) -> BackendResult<()> {
    let Some(header) = db(chain::header(snap, number))? else {
        return Ok(());
    };
    if !filter.matches_bloom(header.logs_bloom) {
        return Ok(());
    }
    let body = db(chain::body(snap, number))?.unwrap_or_default();
    let receipts = db(chain::receipts(snap, number))?;
    let hash = header.hash_slow();
    let mut log_index = 0u64;
    for (i, (tx, receipt)) in body.iter().zip(&receipts).enumerate() {
        let meta = tx_meta(&header, hash, i as u64);
        for rpc_log in convert::rpc_logs(receipt, *tx.tx_hash(), meta, log_index) {
            if filter.matches(&rpc_log.inner) {
                check_logs_limit(out.len() + 1, limit)?;
                out.push(rpc_log);
            }
        }
        log_index += receipt.logs().len() as u64;
    }
    Ok(())
}

/// Effective tips paid in `block` with each tx's gas used, sorted by tip.
fn block_tips(block: &Block, receipts: &[ReceiptEnvelope]) -> Vec<(u128, u64)> {
    let base_fee = block.header.base_fee_per_gas.unwrap_or_default();
    let mut prev = 0u64;
    let mut tips: Vec<(u128, u64)> = block
        .body
        .iter()
        .zip(receipts)
        .map(|(tx, r)| {
            let used = r.cumulative_gas_used() - prev;
            prev = r.cumulative_gas_used();
            (tx.effective_tip_per_gas(base_fee).unwrap_or_default(), used)
        })
        .collect();
    tips.sort_unstable();
    tips
}

/// geth's reward percentiles: tip of the tx at which `p`% of the block gas is reached.
fn rewards(tips: &[(u128, u64)], gas_used: u64, percentiles: &[f64]) -> Vec<u128> {
    if tips.is_empty() {
        return vec![0; percentiles.len()];
    }
    percentiles
        .iter()
        .map(|p| {
            // RPC-only (not consensus): float percentile like geth.
            let threshold = (gas_used as f64 * p / 100.0) as u64;
            let mut sum = 0u64;
            for (tip, used) in tips {
                sum += used;
                if sum >= threshold {
                    return *tip;
                }
            }
            tips.last().map(|t| t.0).unwrap_or_default()
        })
        .collect()
}

/// Median tip of the txs in the last [`TIP_ORACLE_BLOCKS`] blocks.
fn suggested_tip(snap: &Snapshot<'_>) -> BackendResult<u128> {
    let head = db(chain::latest_header(snap))?.0;
    let mut tips = Vec::new();
    for n in head.saturating_sub(TIP_ORACLE_BLOCKS - 1)..=head {
        if let Some(block) = db(chain::block(snap, n))? {
            let base_fee = block.header.base_fee_per_gas.unwrap_or_default();
            tips.extend(
                block
                    .body
                    .iter()
                    .filter_map(|tx| tx.effective_tip_per_gas(base_fee)),
            );
        }
    }
    if tips.is_empty() {
        return Ok(DEFAULT_PRIORITY_FEE);
    }
    tips.sort_unstable();
    Ok(tips[tips.len() / 2].max(DEFAULT_PRIORITY_FEE))
}

fn find_tx(snap: &Snapshot<'_>, hash: B256) -> BackendResult<Option<(Header, TxEnvelope, u64)>> {
    let Some(loc) = db(chain::tx_location(snap, hash))? else {
        return Ok(None);
    };
    let header = db(chain::header(snap, loc.block_number))?
        .ok_or_else(|| internal(format!("tx {hash}: missing block {}", loc.block_number)))?;
    let body = db(chain::body(snap, loc.block_number))?.unwrap_or_default();
    let tx = body
        .get(loc.index as usize)
        .cloned()
        .ok_or_else(|| internal(format!("tx {hash}: bad index {}", loc.index)))?;
    Ok(Some((header, tx, loc.index)))
}

#[async_trait::async_trait]
impl RpcBackend for NodeBackend {
    fn chain_id(&self) -> u64 {
        self.cfg.chain_id
    }

    async fn block_number(&self) -> BackendResult<u64> {
        self.read(|s, _| Ok(db(chain::latest_header(s))?.0)).await
    }

    async fn balance(&self, address: Address, block: BlockId) -> BackendResult<U256> {
        self.read(move |s, _| {
            let (_, state) = state_at(s, block)?;
            Ok(state
                .basic_account(address)
                .map_err(state_err)?
                .map(|a| a.balance)
                .unwrap_or_default())
        })
        .await
    }

    async fn transaction_count(&self, address: Address, block: BlockId) -> BackendResult<u64> {
        let pending = matches!(block, BlockId::Number(BlockNumberOrTag::Pending));
        self.read(move |s, ctx| {
            let (_, state) = state_at(s, block)?;
            let nonce = state
                .basic_account(address)
                .map_err(state_err)?
                .map(|a| a.nonce)
                .unwrap_or_default();
            let pool_nonce = pending
                .then(|| ctx.pool.pending_nonce(&address))
                .flatten()
                .unwrap_or_default();
            Ok(nonce.max(pool_nonce))
        })
        .await
    }

    async fn code(&self, address: Address, block: BlockId) -> BackendResult<Bytes> {
        self.read(move |s, _| {
            let (_, state) = state_at(s, block)?;
            state.code(address).map_err(state_err)
        })
        .await
    }

    async fn storage_at(
        &self,
        address: Address,
        slot: B256,
        block: BlockId,
    ) -> BackendResult<B256> {
        self.read(move |s, _| {
            let (_, state) = state_at(s, block)?;
            let v = state.storage(address, slot).map_err(state_err)?;
            Ok(B256::from(v.to_be_bytes::<32>()))
        })
        .await
    }

    async fn call(&self, request: TransactionRequest, block: BlockId) -> BackendResult<Bytes> {
        self.simulate(move |s, ctx, limits| {
            let (header, state) = state_at(s, block)?;
            match ctx
                .executor
                .call_with_limits(&*state, &header, &request, ctx.gas_cap, limits)
                .map_err(|e| ctx.exec_err(e))?
            {
                CallResult::Success { output, .. } => Ok(output),
                CallResult::Revert { output, .. } => Err(reverted(output)),
                CallResult::Halt { reason, .. } => Err(RpcBackendError::Rejected(reason)),
            }
        })
        .await
    }

    async fn estimate_gas(
        &self,
        request: TransactionRequest,
        block: BlockId,
    ) -> BackendResult<u64> {
        self.simulate(move |s, ctx, limits| {
            let (header, state) = state_at(s, block)?;
            ctx.executor
                .estimate_gas_with_limits(&*state, &header, &request, ctx.gas_cap, limits)
                .map_err(|e| ctx.exec_err(e))
        })
        .await
    }

    async fn send_raw_transaction(&self, raw: Bytes) -> BackendResult<B256> {
        let chain_id = self.cfg.chain_id;
        let pool = self.pool.clone();
        let gossip = self.gossip.clone();
        let events = self.events.clone();
        let permit = self.slot().await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let tx = decode_and_recover(&raw, chain_id)
                .map_err(|e| RpcBackendError::Rejected(e.to_string()))?;
            let hash = tx.hash();
            pool.add(tx, TxOrigin::External).map_err(pool_err)?;
            if let Some(events) = events {
                events.publish_pending_tx(hash);
            }
            tracing::debug!(target: "xdevs::rpc", %hash, "transaction added to pool");
            if let Some(gossip) = gossip {
                // Best effort: the tx is in the local pool either way.
                if let Err(e) = gossip.publish_tx(raw) {
                    tracing::warn!(target: "xdevs::rpc", %hash, error = %e, "tx gossip publish failed");
                }
            }
            Ok(hash)
        })
        .await
        .map_err(internal)?
    }

    fn subscribe_blocks(&self) -> Option<broadcast::Receiver<Arc<BlockEvent>>> {
        self.events.as_ref().map(|e| e.subscribe_blocks())
    }

    fn subscribe_pending_txs(&self) -> Option<broadcast::Receiver<B256>> {
        self.events.as_ref().map(|e| e.subscribe_pending_txs())
    }

    async fn syncing(&self) -> BackendResult<SyncStatus> {
        let Some(status) = &self.consensus else {
            return Ok(SyncStatus::None);
        };
        let head = self.block_number().await?;
        // Peers voting at height h have decided h - 1.
        let highest = status.highest_seen_height().saturating_sub(1);
        if highest < head.saturating_add(SYNC_LAG_THRESHOLD) {
            return Ok(SyncStatus::None);
        }
        Ok(SyncStatus::Info(Box::new(SyncInfo {
            starting_block: U256::from(self.starting_block.min(head)),
            current_block: U256::from(head),
            highest_block: U256::from(highest),
            warp_chunks_amount: None,
            warp_chunks_processed: None,
            stages: None,
        })))
    }

    async fn peer_count(&self) -> BackendResult<u64> {
        let gossip = self.gossip.as_ref().map_or(0, |g| g.peer_count());
        let consensus = self.consensus.as_ref().map_or(0, |s| s.peer_count());
        Ok((gossip + consensus) as u64)
    }

    async fn commit_certificate(
        &self,
        block: BlockId,
    ) -> BackendResult<Option<CommitCertificateInfo>> {
        self.read(move |s, _| {
            let Some(number) = resolve_block_id(s, block)? else {
                return Ok(None);
            };
            let Some(raw) = db(chain::commit_certificate(s, number))? else {
                return Ok(None);
            };
            let cert = CommitCertificate::from_bytes(&raw)
                .map_err(|e| internal(format!("stored certificate of block {number}: {e:?}")))?;
            Ok(Some(CommitCertificateInfo {
                height: U64::from(cert.height),
                round: U64::from(cert.round),
                block_hash: cert.block_hash,
                signatures: cert
                    .signatures
                    .iter()
                    .map(|sig| CommitSigInfo {
                        validator: Bytes::copy_from_slice(&sig.validator.0),
                        signature: Bytes::copy_from_slice(&sig.signature),
                    })
                    .collect(),
                raw,
            }))
        })
        .await
    }

    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> BackendResult<Option<RpcBlock>> {
        self.read(move |s, _| match resolve_number(s, number)? {
            Some(n) => rpc_block_at(s, n, full),
            None => Ok(None),
        })
        .await
    }

    async fn block_by_hash(&self, hash: B256, full: bool) -> BackendResult<Option<RpcBlock>> {
        self.read(move |s, _| match db(chain::number_by_hash(s, hash))? {
            Some(n) => rpc_block_at(s, n, full),
            None => Ok(None),
        })
        .await
    }

    async fn block_transaction_count(&self, block: BlockId) -> BackendResult<Option<u64>> {
        self.read(move |s, _| match resolve_block_id(s, block)? {
            Some(n) => Ok(db(chain::body(s, n))?.map(|b| b.len() as u64)),
            None => Ok(None),
        })
        .await
    }

    async fn transaction_by_hash(&self, hash: B256) -> BackendResult<Option<RpcTransaction>> {
        self.read(move |s, ctx| {
            if let Some((header, tx, index)) = find_tx(s, hash)? {
                let rtx = RecoveredTx::recover(tx).map_err(internal)?;
                let meta = tx_meta(&header, header.hash_slow(), index);
                return Ok(Some(convert::rpc_transaction(&rtx, Some(meta))));
            }
            Ok(ctx
                .pool
                .get(&hash)
                .map(|tx| convert::rpc_transaction(&tx, None)))
        })
        .await
    }

    async fn transaction_receipt(&self, hash: B256) -> BackendResult<Option<RpcReceipt>> {
        self.read(move |s, _| {
            let Some(loc) = db(chain::tx_location(s, hash))? else {
                return Ok(None);
            };
            Ok(rpc_receipts_at(s, loc.block_number, Some(loc.index))?.and_then(|mut v| v.pop()))
        })
        .await
    }

    async fn block_receipts(&self, block: BlockId) -> BackendResult<Option<Vec<RpcReceipt>>> {
        self.read(move |s, _| match resolve_block_id(s, block)? {
            Some(n) => rpc_receipts_at(s, n, None),
            None => Ok(None),
        })
        .await
    }

    async fn logs(&self, filter: Filter, limit: Option<usize>) -> BackendResult<Vec<RpcLog>> {
        self.read(move |s, _| {
            let mut out = Vec::new();
            match filter.block_option {
                FilterBlockOption::AtBlockHash(hash) => {
                    let n =
                        db(chain::number_by_hash(s, hash))?.ok_or(RpcBackendError::UnknownBlock)?;
                    logs_in_block(s, n, &filter, limit, &mut out)?;
                }
                FilterBlockOption::Range {
                    from_block,
                    to_block,
                } => {
                    let head = db(chain::latest_header(s))?.0;
                    let resolve = |t: Option<BlockNumberOrTag>| match t {
                        Some(BlockNumberOrTag::Number(n)) => n,
                        Some(BlockNumberOrTag::Earliest) => 0,
                        _ => head,
                    };
                    let (from, to) = (resolve(from_block), resolve(to_block).min(head));
                    for n in from..=to {
                        logs_in_block(s, n, &filter, limit, &mut out)?;
                    }
                }
            }
            Ok(out)
        })
        .await
    }

    async fn gas_price(&self) -> BackendResult<U256> {
        self.read(|s, ctx| {
            let (_, head) = db(chain::latest_header(s))?;
            let base = u128::from(next_base_fee(&head, &ctx.cfg));
            Ok(U256::from(base + suggested_tip(s)?))
        })
        .await
    }

    async fn max_priority_fee_per_gas(&self) -> BackendResult<U256> {
        self.read(|s, _| Ok(U256::from(suggested_tip(s)?))).await
    }

    async fn next_base_fee(&self) -> BackendResult<u64> {
        self.read(|s, ctx| {
            let (_, head) = db(chain::latest_header(s))?;
            Ok(next_base_fee(&head, &ctx.cfg))
        })
        .await
    }

    async fn eoa_lock(&self, address: Address, block: BlockId) -> BackendResult<EoaLockInfo> {
        self.read(move |s, ctx| {
            let (header, state) = state_at(s, block)?;
            let status = state.eoa_lock_status(address).map_err(state_err)?;
            let word = state.storage(address, LOCK_SLOT).map_err(state_err)?;
            let code = state.code(address).map_err(state_err)?;
            let delegated_to = code
                .strip_prefix(&[0xef, 0x01, 0x00])
                .filter(|target| target.len() == 20)
                .map(Address::from_slice);
            Ok(EoaLockInfo {
                active: ctx.cfg.xip0001_active(header.number + 1),
                status,
                word: B256::from(word),
                delegated_to,
            })
        })
        .await
    }

    async fn fee_history(
        &self,
        block_count: u64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> BackendResult<FeeHistory> {
        self.read(move |s, ctx| {
            let head = db(chain::latest_header(s))?.0;
            let newest = match newest_block {
                BlockNumberOrTag::Number(n) if n > head => {
                    return Err(RpcBackendError::Rejected(format!(
                        "request beyond head block: requested {n}, head {head}"
                    )))
                }
                tag => resolve_number(s, tag)?.unwrap_or(head),
            };
            let count = block_count.min(newest + 1);
            if count == 0 {
                return Ok(FeeHistory::default());
            }
            let oldest = newest + 1 - count;
            let mut history = FeeHistory {
                oldest_block: oldest,
                reward: reward_percentiles.as_ref().map(|_| Vec::new()),
                ..Default::default()
            };
            let mut last = None;
            for n in oldest..=newest {
                let block = db(chain::block(s, n))?
                    .ok_or_else(|| internal(format!("missing block {n}")))?;
                let h = &block.header;
                history
                    .base_fee_per_gas
                    .push(u128::from(h.base_fee_per_gas.unwrap_or_default()));
                // RPC-only ratio (floats never reach consensus code).
                history.gas_used_ratio.push(if h.gas_limit == 0 {
                    0.0
                } else {
                    h.gas_used as f64 / h.gas_limit as f64
                });
                if let (Some(p), Some(rows)) = (&reward_percentiles, &mut history.reward) {
                    let receipts = db(chain::receipts(s, n))?;
                    rows.push(rewards(&block_tips(&block, &receipts), h.gas_used, p));
                }
                last = Some(block.header);
            }
            if let Some(h) = last {
                history
                    .base_fee_per_gas
                    .push(u128::from(next_base_fee(&h, &ctx.cfg)));
            }
            Ok(history)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::hex;

    #[test]
    fn decodes_error_string() {
        // Error("not enough")
        let data = hex::decode(
            "08c379a0\
             0000000000000000000000000000000000000000000000000000000000000020\
             000000000000000000000000000000000000000000000000000000000000000a\
             6e6f7420656e6f75676800000000000000000000000000000000000000000000",
        )
        .unwrap();
        assert_eq!(decode_revert_reason(&data).as_deref(), Some("not enough"));
        assert_eq!(decode_revert_reason(&[0xde, 0xad]), None);
        assert_eq!(decode_revert_reason(&data[..40]), None);
    }

    #[test]
    fn reward_percentiles_follow_gas_weighting() {
        let tips = vec![(1u128, 21_000u64), (5, 21_000), (9, 58_000)];
        let used = 100_000;
        assert_eq!(
            rewards(&tips, used, &[0.0, 25.0, 50.0, 100.0]),
            vec![1, 5, 9, 9]
        );
        assert_eq!(rewards(&[], 0, &[10.0, 90.0]), vec![0, 0]);
    }

    use crate::producer::BlockProducer;
    use alloy_rpc_types_eth::TransactionInput;
    use xdevs_crypto::{sign_tx_envelope, signer_from_hex};
    use xdevs_genesis::{Genesis, DEV_ACCOUNTS, DEV_ACCOUNT_BALANCE, DEV_PRIVATE_KEYS};
    use xdevs_mempool::{FromState, PoolConfig, PoolParams};
    use xdevs_primitives::{
        constants::{DEV_CHAIN_ID, GWEI},
        Log, LogData, Receipt, ReceiptWithBloom, TxEip1559,
    };
    use xdevs_storage::{meta_keys, tables, MemDb, WriteBatch};

    /// Dev chain on a [`MemDb`] with a producer keeping `history` blocks of state.
    fn dev_chain(history: u64) -> (Db, Arc<Pool>, BlockProducer, NodeBackend) {
        let db: Db = Arc::new(MemDb::new());
        let genesis = Genesis::dev(DEV_CHAIN_ID);
        chain::init_genesis(&*db, &genesis).unwrap();
        let cfg = genesis.chain_config();
        let (_, head) = chain::latest_header(&*db).unwrap();
        let pool = Arc::new(Pool::new(
            PoolParams::from_chain_config(&cfg),
            PoolConfig::default(),
            Arc::new(FromState(DbStateProvider::new(db.clone()))),
            next_base_fee(&head, &cfg),
        ));
        let executor = Arc::new(EvmExecutor::new(cfg.clone()));
        let producer = BlockProducer::new(
            db.clone(),
            pool.clone(),
            executor.clone(),
            cfg.clone(),
            Address::repeat_byte(0xfe),
            Bytes::from_static(b"xdevs"),
        )
        .with_state_history(history);
        let backend = NodeBackend::new(db.clone(), pool.clone(), executor, cfg, 50_000_000);
        (db, pool, producer, backend)
    }

    fn transfer(nonce: u64, to: Address, value: u64) -> RecoveredTx {
        let signer = signer_from_hex(DEV_PRIVATE_KEYS[0]).unwrap();
        let tx = sign_tx_envelope(
            &signer,
            TxEip1559 {
                chain_id: DEV_CHAIN_ID,
                nonce,
                gas_limit: 21_000,
                max_fee_per_gas: 10 * u128::from(GWEI),
                max_priority_fee_per_gas: u128::from(GWEI),
                to: TxKind::Call(to),
                value: U256::from(value),
                input: Bytes::new(),
                access_list: Default::default(),
            },
        )
        .unwrap();
        RecoveredTx::recover(tx).unwrap()
    }

    fn at(n: u64) -> BlockId {
        BlockId::Number(BlockNumberOrTag::Number(n))
    }

    /// Regression: state queries pinned to a recent past block (Foundry fork mode,
    /// `cast --block`, viem `blockNumber`) read that block's state instead of failing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_queries_at_past_blocks() {
        let (db, pool, producer, backend) = dev_chain(3);
        let to = Address::repeat_byte(0x77);
        let from = DEV_ACCOUNTS[0];
        pool.add(transfer(0, to, 5), TxOrigin::Local).unwrap();
        assert_eq!(producer.produce(0).unwrap().tx_hashes.len(), 1); // block 1
        pool.add(transfer(1, to, 7), TxOrigin::Local).unwrap();
        assert_eq!(producer.produce(0).unwrap().tx_hashes.len(), 1); // block 2
        producer.produce(0).unwrap(); // block 3 (empty)

        let balance = |n| backend.balance(to, at(n));
        assert_eq!(balance(0).await.unwrap(), U256::ZERO);
        assert_eq!(balance(1).await.unwrap(), U256::from(5u64));
        assert_eq!(balance(2).await.unwrap(), U256::from(12u64));
        assert_eq!(balance(3).await.unwrap(), U256::from(12u64));
        assert_eq!(
            backend.balance(from, at(0)).await.unwrap(),
            DEV_ACCOUNT_BALANCE
        );
        assert_eq!(backend.transaction_count(from, at(1)).await.unwrap(), 1);
        let hash1 = chain::header(&*db, 1).unwrap().unwrap().hash_slow();
        assert_eq!(
            backend.balance(to, BlockId::from(hash1)).await.unwrap(),
            U256::from(5u64)
        );

        // eth_call / estimateGas run on the historical state: BALANCE(to) at block 1.
        let mut code = vec![0x73];
        code.extend_from_slice(to.as_slice());
        code.extend_from_slice(&[0x31, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
        let request = TransactionRequest {
            from: Some(from),
            input: TransactionInput::new(Bytes::from(code)),
            ..Default::default()
        };
        let out = backend.call(request.clone(), at(1)).await.unwrap();
        assert_eq!(U256::from_be_slice(&out), U256::from(5u64));
        let out = backend.call(request.clone(), at(2)).await.unwrap();
        assert_eq!(U256::from_be_slice(&out), U256::from(12u64));
        assert!(backend.estimate_gas(request, at(1)).await.unwrap() > 21_000);
        assert_eq!(backend.code(to, at(1)).await.unwrap(), Bytes::new());
        assert_eq!(
            backend.storage_at(to, B256::ZERO, at(1)).await.unwrap(),
            B256::ZERO
        );

        // Window of 3 blocks: once the head is 5, block 1 is pruned, block 2 is kept.
        producer.produce(0).unwrap();
        producer.produce(0).unwrap();
        assert_eq!(balance(2).await.unwrap(), U256::from(12u64));
        let err = balance(1).await.unwrap_err();
        assert!(
            matches!(&err, RpcBackendError::Rejected(m) if m.contains("historical state not available")),
            "{err:?}"
        );
        assert_eq!(
            balance(99).await.unwrap_err(),
            RpcBackendError::UnknownBlock
        );
    }

    /// Regression: eth_getLogs stops collecting once the result cap is exceeded,
    /// instead of buffering every log of the range first.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn logs_stop_collecting_at_the_limit() {
        let (db, _pool, _producer, backend) = dev_chain(10);
        let tx = transfer(0, Address::repeat_byte(1), 1).tx;
        let log = Log {
            address: Address::repeat_byte(0xcc),
            data: LogData::new_unchecked(vec![], Bytes::new()),
        };
        let receipt = ReceiptWithBloom::from(Receipt::<Log> {
            status: true.into(),
            cumulative_gas_used: 21_000,
            logs: vec![log; 5],
        });
        let (_, mut header) = chain::latest_header(&*db).unwrap();
        header.number = 1;
        header.logs_bloom = receipt.logs_bloom;
        let mut batch = WriteBatch::new();
        batch.put::<tables::Headers>(&1, &header);
        batch.put::<tables::Bodies>(&1, &vec![tx]);
        batch.put::<tables::Receipts>(&(1, 0), &ReceiptEnvelope::Eip1559(receipt));
        batch.put::<tables::Meta>(
            &meta_keys::LATEST_BLOCK.to_string(),
            &1u64.to_be_bytes().to_vec(),
        );
        db.write(batch).unwrap();

        let snap = db.snapshot().unwrap();
        let mut out = Vec::new();
        let err = logs_in_block(&*snap, 1, &Filter::default(), Some(2), &mut out).unwrap_err();
        assert!(matches!(err, RpcBackendError::LimitExceeded(_)), "{err:?}");
        assert_eq!(out.len(), 2, "collection stopped at the limit");
        out.clear();
        logs_in_block(&*snap, 1, &Filter::default(), Some(5), &mut out).unwrap();
        assert_eq!(out.len(), 5);

        let range = Filter::default().from_block(0u64).to_block(1u64);
        let err = backend.logs(range.clone(), Some(4)).await.unwrap_err();
        assert_eq!(err.code(), -32005);
        assert_eq!(backend.logs(range.clone(), Some(5)).await.unwrap().len(), 5);
        assert_eq!(backend.logs(range, None).await.unwrap().len(), 5);
    }
}

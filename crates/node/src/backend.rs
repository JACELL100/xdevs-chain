//! [`RpcBackend`] over storage + state + mempool + executor (plan §19.1).
//!
//! Every request runs on a blocking thread against one database snapshot, so all
//! reads of a request see the same committed block even while the producer commits.
//! Only the latest state is kept (Phase 1): state queries at older blocks fail with
//! a geth-style "-32000" error.

use crate::chain::{self, Db};
use std::sync::Arc;
use xdevs_crypto::decode_and_recover;
use xdevs_execution::{next_base_fee, CallResult, EvmExecutor, ExecutionError};
use xdevs_mempool::{Pool, TxOrigin, TxPool};
use xdevs_primitives::{
    Address, Block, Bytes, ChainConfig, Header, ReceiptEnvelope, RecoveredTx, Transaction,
    TxEnvelope, TxKind, B256, U256,
};
use xdevs_rpc::{
    convert::{self, ReceiptMeta, TxMeta},
    types::*,
    BackendResult, RpcBackend, RpcBackendError,
};
use xdevs_state::{DbStateProvider, StateProvider};
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

/// The node's RPC backend.
pub struct NodeBackend {
    db: Db,
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
    cfg: ChainConfig,
    gas_cap: u64,
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
        }
    }

    /// Run `f` on a blocking thread with a consistent database snapshot.
    async fn read<T, F>(&self, f: F) -> BackendResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Snapshot<'_>, &Ctx) -> BackendResult<T> + Send + 'static,
    {
        let db = self.db.clone();
        let ctx = Ctx {
            pool: self.pool.clone(),
            executor: self.executor.clone(),
            cfg: self.cfg.clone(),
            gas_cap: self.gas_cap,
        };
        tokio::task::spawn_blocking(move || {
            let snapshot = db.snapshot().map_err(internal)?;
            f(&*snapshot, &ctx)
        })
        .await
        .map_err(|e| internal(format!("request task failed: {e}")))?
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

/// Header whose post-state is queried. Only the latest state exists in Phase 1.
fn state_header(snap: &Snapshot<'_>, id: BlockId) -> BackendResult<Header> {
    let (head_number, head) = db(chain::latest_header(snap))?;
    let number = resolve_block_id(snap, id)?.ok_or(RpcBackendError::UnknownBlock)?;
    if number != head_number {
        return Err(RpcBackendError::Rejected(format!(
            "historical state not available: block {number} (this node keeps only the \
             state of the latest block {head_number})"
        )));
    }
    Ok(head)
}

fn state<'a>(snap: &'a Snapshot<'a>) -> DbStateProvider<&'a Snapshot<'a>> {
    DbStateProvider::new(snap)
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

fn exec_err(e: ExecutionError) -> RpcBackendError {
    match e {
        ExecutionError::Reverted { output } => reverted(output),
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

fn logs_in_block(
    snap: &Snapshot<'_>,
    number: u64,
    filter: &Filter,
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
            state_header(s, block)?;
            Ok(state(s)
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
            state_header(s, block)?;
            let nonce = state(s)
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
            state_header(s, block)?;
            state(s).code(address).map_err(state_err)
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
            state_header(s, block)?;
            let v = state(s).storage(address, slot).map_err(state_err)?;
            Ok(B256::from(v.to_be_bytes::<32>()))
        })
        .await
    }

    async fn call(&self, request: TransactionRequest, block: BlockId) -> BackendResult<Bytes> {
        self.read(move |s, ctx| {
            let header = state_header(s, block)?;
            match ctx
                .executor
                .call(&state(s), &header, &request, ctx.gas_cap)
                .map_err(exec_err)?
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
        self.read(move |s, ctx| {
            let header = state_header(s, block)?;
            ctx.executor
                .estimate_gas(&state(s), &header, &request, ctx.gas_cap)
                .map_err(exec_err)
        })
        .await
    }

    async fn send_raw_transaction(&self, raw: Bytes) -> BackendResult<B256> {
        let chain_id = self.cfg.chain_id;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let tx = decode_and_recover(&raw, chain_id)
                .map_err(|e| RpcBackendError::Rejected(e.to_string()))?;
            let hash = tx.hash();
            pool.add(tx, TxOrigin::External)
                .map_err(|e| RpcBackendError::Rejected(e.to_string()))?;
            tracing::debug!(target: "xdevs::rpc", %hash, "transaction added to pool");
            Ok(hash)
        })
        .await
        .map_err(internal)?
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

    async fn logs(&self, filter: Filter) -> BackendResult<Vec<RpcLog>> {
        self.read(move |s, _| {
            let mut out = Vec::new();
            match filter.block_option {
                FilterBlockOption::AtBlockHash(hash) => {
                    let n =
                        db(chain::number_by_hash(s, hash))?.ok_or(RpcBackendError::UnknownBlock)?;
                    logs_in_block(s, n, &filter, &mut out)?;
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
                        logs_in_block(s, n, &filter, &mut out)?;
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
}

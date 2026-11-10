//! JSON-RPC method table (plan §19.1) and request-side hardening (§19.3).
//!
//! Every method parses its positional params, normalises block tags, applies the
//! server limits from [`RpcServerConfig`] and forwards to the [`RpcBackend`].
//!
//! Tag mapping (Phase 1, BFT finality): `latest` = `safe` = `finalized` = `pending` =
//! latest committed block (except `eth_getTransactionCount`, where `pending` is kept so
//! the backend can include pool nonces); `earliest` = block 0. A missing block param
//! means `latest`.

use crate::{types::*, RpcBackend, RpcBackendError, RpcServerConfig};
use jsonrpsee::{
    types::{ErrorObjectOwned, Params},
    RpcModule,
};
use std::sync::Arc;
use xdevs_primitives::{keccak256, reexports::alloy_primitives::U64, Address, Bytes, B256, U256};

type RpcResult<T> = Result<T, ErrorObjectOwned>;

/// `eth_feeHistory` block-count cap (same as geth).
pub(crate) const MAX_FEE_HISTORY_BLOCKS: u64 = 1024;
/// `eth_feeHistory` max number of reward percentiles.
pub(crate) const MAX_REWARD_PERCENTILES: usize = 100;

/// Shared state of every method handler.
pub struct RpcContext {
    backend: Arc<dyn RpcBackend>,
    config: RpcServerConfig,
}

impl std::fmt::Debug for RpcContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcContext")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Build the jsonrpsee module with every Phase-1 method (useful to embed the methods
/// in another server; [`crate::start_server`] uses it).
pub fn build_module(
    config: RpcServerConfig,
    backend: Arc<dyn RpcBackend>,
) -> anyhow::Result<RpcModule<RpcContext>> {
    let mut m = RpcModule::new(RpcContext { backend, config });

    macro_rules! method {
        ($name:literal, $handler:path) => {
            m.register_async_method($name, |params, ctx, _| $handler(params, ctx))?;
        };
    }

    method!("web3_clientVersion", web3_client_version);
    method!("web3_sha3", web3_sha3);
    method!("net_version", net_version);
    method!("net_listening", net_listening);
    method!("net_peerCount", net_peer_count);
    method!("eth_chainId", eth_chain_id);
    method!("eth_blockNumber", eth_block_number);
    method!("eth_syncing", eth_syncing);
    method!("eth_accounts", eth_accounts);
    method!("eth_gasPrice", eth_gas_price);
    method!("eth_maxPriorityFeePerGas", eth_max_priority_fee_per_gas);
    method!("eth_feeHistory", eth_fee_history);
    method!("eth_getBalance", eth_get_balance);
    method!("eth_getTransactionCount", eth_get_transaction_count);
    method!("eth_getCode", eth_get_code);
    method!("eth_getStorageAt", eth_get_storage_at);
    method!("eth_call", eth_call);
    method!("eth_estimateGas", eth_estimate_gas);
    method!("eth_sendRawTransaction", eth_send_raw_transaction);
    method!("eth_getBlockByNumber", eth_get_block_by_number);
    method!("eth_getBlockByHash", eth_get_block_by_hash);
    method!(
        "eth_getBlockTransactionCountByNumber",
        eth_get_block_transaction_count_by_number
    );
    method!(
        "eth_getBlockTransactionCountByHash",
        eth_get_block_transaction_count_by_hash
    );
    method!("eth_getTransactionByHash", eth_get_transaction_by_hash);
    method!("eth_getTransactionReceipt", eth_get_transaction_receipt);
    method!("eth_getBlockReceipts", eth_get_block_receipts);
    method!("eth_getLogs", eth_get_logs);
    Ok(m)
}

// ---------------------------------------------------------------------------
// Parameter helpers
// ---------------------------------------------------------------------------

fn invalid(msg: impl Into<String>) -> ErrorObjectOwned {
    RpcBackendError::InvalidParams(msg.into()).into()
}

/// Phase-1 tag mapping: safe/finalized/pending -> latest, earliest -> 0.
pub(crate) fn normalize_tag(tag: BlockNumberOrTag) -> BlockNumberOrTag {
    match tag {
        BlockNumberOrTag::Safe | BlockNumberOrTag::Finalized | BlockNumberOrTag::Pending => {
            BlockNumberOrTag::Latest
        }
        BlockNumberOrTag::Earliest => BlockNumberOrTag::Number(0),
        other => other,
    }
}

/// Default (`None`) = latest; tags normalised with [`normalize_tag`].
pub(crate) fn normalize_block_id(id: Option<BlockId>) -> BlockId {
    match id.unwrap_or(BlockId::Number(BlockNumberOrTag::Latest)) {
        BlockId::Number(tag) => BlockId::Number(normalize_tag(tag)),
        hash => hash,
    }
}

/// Clamp a caller-supplied gas limit to the server gas cap (geth behaviour).
pub(crate) fn cap_gas(request: &mut TransactionRequest, cap: u64) {
    if cap > 0 && request.gas.is_some_and(|g| g > cap) {
        request.gas = Some(cap);
    }
}

/// State / block overrides (3rd/4th `eth_call` param) are not supported in Phase 1.
fn reject_overrides(value: Option<serde_json::Value>) -> RpcResult<()> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(serde_json::Value::Object(map)) if map.is_empty() => Ok(()),
        Some(_) => Err(invalid("state and block overrides are not supported")),
    }
}

/// `eth_feeHistory` block count: JSON number, decimal string or `0x` hex string.
pub(crate) fn parse_block_count(value: &serde_json::Value) -> RpcResult<u64> {
    let parsed = match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
        {
            Some(hex) if !hex.is_empty() => u64::from_str_radix(hex, 16).ok(),
            Some(_) => None,
            None => s.parse::<u64>().ok(),
        },
        _ => None,
    };
    parsed.ok_or_else(|| invalid(format!("invalid block count: {value}")))
}

/// Reward percentiles must be within `[0, 100]` and strictly increasing (geth rule).
pub(crate) fn validate_percentiles(percentiles: &[f64]) -> RpcResult<()> {
    if percentiles.len() > MAX_REWARD_PERCENTILES {
        return Err(invalid(format!(
            "too many reward percentiles: {} > {MAX_REWARD_PERCENTILES}",
            percentiles.len()
        )));
    }
    for (i, p) in percentiles.iter().enumerate() {
        if !(0.0..=100.0).contains(p) {
            return Err(invalid(format!("invalid reward percentile: {p}")));
        }
        if i > 0 && *p <= percentiles[i - 1] {
            return Err(invalid(format!(
                "invalid reward percentile: #{}:{} >= #{i}:{p}",
                i - 1,
                percentiles[i - 1]
            )));
        }
    }
    Ok(())
}

/// Outcome of resolving an `eth_getLogs` block range against the head.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LogRange {
    /// Query the inclusive range `from..=to`.
    Range(u64, u64),
    /// Range starts after the head: the answer is `[]`.
    Empty,
}

/// Resolve `fromBlock` / `toBlock` (missing = latest) and enforce the range limit
/// (`max_range = 0` disables it).
pub(crate) fn resolve_log_range(
    from: Option<BlockNumberOrTag>,
    to: Option<BlockNumberOrTag>,
    head: u64,
    max_range: u64,
) -> Result<LogRange, RpcBackendError> {
    let resolve = |tag: Option<BlockNumberOrTag>| match tag {
        None => head,
        Some(BlockNumberOrTag::Number(n)) => n,
        Some(BlockNumberOrTag::Earliest) => 0,
        Some(_) => head,
    };
    let from = resolve(from);
    let to = resolve(to).min(head);
    if from > head {
        return Ok(LogRange::Empty);
    }
    if from > to {
        return Err(RpcBackendError::InvalidParams(
            "invalid block range params".into(),
        ));
    }
    if max_range > 0 && to - from >= max_range {
        return Err(RpcBackendError::LimitExceeded(format!(
            "exceed maximum block range: {max_range}"
        )));
    }
    Ok(LogRange::Range(from, to))
}

// ---------------------------------------------------------------------------
// web3_ / net_
// ---------------------------------------------------------------------------

async fn web3_client_version(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<String> {
    Ok(ctx.backend.client_version())
}

async fn web3_sha3(params: Params<'static>, _: Arc<RpcContext>) -> RpcResult<B256> {
    let data: Bytes = params.one()?;
    Ok(keccak256(&data))
}

async fn net_version(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<String> {
    Ok(ctx.backend.chain_id().to_string())
}

async fn net_listening(_: Params<'static>, _: Arc<RpcContext>) -> RpcResult<bool> {
    Ok(true)
}

async fn net_peer_count(_: Params<'static>, _: Arc<RpcContext>) -> RpcResult<U64> {
    Ok(U64::ZERO)
}

// ---------------------------------------------------------------------------
// eth_: chain / fees
// ---------------------------------------------------------------------------

async fn eth_chain_id(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<U64> {
    Ok(U64::from(ctx.backend.chain_id()))
}

async fn eth_block_number(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<U64> {
    Ok(U64::from(ctx.backend.block_number().await?))
}

async fn eth_syncing(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<SyncStatus> {
    Ok(ctx.backend.syncing().await?)
}

async fn eth_accounts(_: Params<'static>, _: Arc<RpcContext>) -> RpcResult<Vec<Address>> {
    Ok(Vec::new())
}

async fn eth_gas_price(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<U256> {
    Ok(ctx.backend.gas_price().await?)
}

async fn eth_max_priority_fee_per_gas(_: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<U256> {
    Ok(ctx.backend.max_priority_fee_per_gas().await?)
}

async fn eth_fee_history(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<FeeHistory> {
    let mut seq = params.sequence();
    let count: serde_json::Value = seq.next()?;
    let count = parse_block_count(&count)?;
    let newest: BlockNumberOrTag = seq.next()?;
    let percentiles: Option<Vec<f64>> = seq.optional_next()?;
    if let Some(p) = &percentiles {
        validate_percentiles(p)?;
    }
    if count == 0 {
        return Ok(FeeHistory::default());
    }
    let count = count.min(MAX_FEE_HISTORY_BLOCKS);
    Ok(ctx
        .backend
        .fee_history(count, normalize_tag(newest), percentiles)
        .await?)
}

// ---------------------------------------------------------------------------
// eth_: state
// ---------------------------------------------------------------------------

async fn eth_get_balance(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<U256> {
    let mut seq = params.sequence();
    let address: Address = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    Ok(ctx.backend.balance(address, block).await?)
}

async fn eth_get_transaction_count(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<U64> {
    let mut seq = params.sequence();
    let address: Address = seq.next()?;
    let block: Option<BlockId> = seq.optional_next()?;
    // `pending` is meaningful here: the backend adds pool nonces.
    let block = match block {
        Some(BlockId::Number(BlockNumberOrTag::Pending)) => {
            BlockId::Number(BlockNumberOrTag::Pending)
        }
        other => normalize_block_id(other),
    };
    Ok(U64::from(
        ctx.backend.transaction_count(address, block).await?,
    ))
}

async fn eth_get_code(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<Bytes> {
    let mut seq = params.sequence();
    let address: Address = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    Ok(ctx.backend.code(address, block).await?)
}

async fn eth_get_storage_at(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<B256> {
    let mut seq = params.sequence();
    let address: Address = seq.next()?;
    // Slots are QUANTITY-like: "0x0" and 32-byte padded hex are both accepted.
    let slot: U256 = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    Ok(ctx
        .backend
        .storage_at(address, B256::from(slot), block)
        .await?)
}

async fn eth_call(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<Bytes> {
    let mut seq = params.sequence();
    let mut request: TransactionRequest = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    reject_overrides(seq.optional_next()?)?;
    reject_overrides(seq.optional_next()?)?;
    cap_gas(&mut request, ctx.config.gas_cap);
    Ok(ctx.backend.call(request, block).await?)
}

async fn eth_estimate_gas(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<U64> {
    let mut seq = params.sequence();
    let mut request: TransactionRequest = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    reject_overrides(seq.optional_next()?)?;
    cap_gas(&mut request, ctx.config.gas_cap);
    Ok(U64::from(ctx.backend.estimate_gas(request, block).await?))
}

async fn eth_send_raw_transaction(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<B256> {
    let raw: Bytes = params.one()?;
    if raw.is_empty() {
        return Err(invalid("empty raw transaction"));
    }
    Ok(ctx.backend.send_raw_transaction(raw).await?)
}

// ---------------------------------------------------------------------------
// eth_: blocks / transactions / receipts / logs
// ---------------------------------------------------------------------------

async fn eth_get_block_by_number(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<RpcBlock>> {
    let mut seq = params.sequence();
    let number: BlockNumberOrTag = seq.next()?;
    let full: Option<bool> = seq.optional_next()?;
    Ok(ctx
        .backend
        .block_by_number(normalize_tag(number), full.unwrap_or(false))
        .await?)
}

async fn eth_get_block_by_hash(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<RpcBlock>> {
    let mut seq = params.sequence();
    let hash: B256 = seq.next()?;
    let full: Option<bool> = seq.optional_next()?;
    Ok(ctx
        .backend
        .block_by_hash(hash, full.unwrap_or(false))
        .await?)
}

async fn eth_get_block_transaction_count_by_number(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<U64>> {
    let number: BlockNumberOrTag = params.one()?;
    let block = BlockId::Number(normalize_tag(number));
    Ok(ctx
        .backend
        .block_transaction_count(block)
        .await?
        .map(U64::from))
}

async fn eth_get_block_transaction_count_by_hash(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<U64>> {
    let hash: B256 = params.one()?;
    Ok(ctx
        .backend
        .block_transaction_count(BlockId::from(hash))
        .await?
        .map(U64::from))
}

async fn eth_get_transaction_by_hash(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<RpcTransaction>> {
    let hash: B256 = params.one()?;
    Ok(ctx.backend.transaction_by_hash(hash).await?)
}

async fn eth_get_transaction_receipt(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<RpcReceipt>> {
    let hash: B256 = params.one()?;
    Ok(ctx.backend.transaction_receipt(hash).await?)
}

async fn eth_get_block_receipts(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<Vec<RpcReceipt>>> {
    let block: BlockId = params.one()?;
    Ok(ctx
        .backend
        .block_receipts(normalize_block_id(Some(block)))
        .await?)
}

async fn eth_get_logs(params: Params<'static>, ctx: Arc<RpcContext>) -> RpcResult<Vec<RpcLog>> {
    let mut filter: Filter = params.one()?;
    if let FilterBlockOption::Range {
        from_block,
        to_block,
    } = filter.block_option
    {
        let head = ctx.backend.block_number().await?;
        match resolve_log_range(from_block, to_block, head, ctx.config.max_logs_range)? {
            LogRange::Empty => return Ok(Vec::new()),
            LogRange::Range(from, to) => {
                filter.block_option = FilterBlockOption::Range {
                    from_block: Some(BlockNumberOrTag::Number(from)),
                    to_block: Some(BlockNumberOrTag::Number(to)),
                };
            }
        }
    }
    let max = ctx.config.max_logs_results;
    let limit = (max > 0).then_some(max);
    let logs = ctx.backend.logs(filter, limit).await?;
    // Backstop: backends should already stop at `limit` (see `RpcBackend::logs`).
    crate::check_logs_limit(logs.len(), limit)?;
    Ok(logs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tag_normalisation() {
        use BlockNumberOrTag::*;
        assert_eq!(normalize_tag(Safe), Latest);
        assert_eq!(normalize_tag(Finalized), Latest);
        assert_eq!(normalize_tag(Pending), Latest);
        assert_eq!(normalize_tag(Earliest), Number(0));
        assert_eq!(normalize_tag(Number(7)), Number(7));
        assert_eq!(normalize_block_id(None), BlockId::Number(Latest));
        let h = B256::repeat_byte(1);
        assert_eq!(normalize_block_id(Some(h.into())), BlockId::from(h));
    }

    #[test]
    fn gas_cap_clamps_only_above_cap() {
        let mut r = TransactionRequest::default();
        cap_gas(&mut r, 50);
        assert_eq!(r.gas, None);
        r.gas = Some(60);
        cap_gas(&mut r, 50);
        assert_eq!(r.gas, Some(50));
        r.gas = Some(40);
        cap_gas(&mut r, 50);
        assert_eq!(r.gas, Some(40));
    }

    #[test]
    fn block_count_forms() {
        assert_eq!(parse_block_count(&json!(5)).unwrap(), 5);
        assert_eq!(parse_block_count(&json!("0x5")).unwrap(), 5);
        assert_eq!(parse_block_count(&json!("10")).unwrap(), 10);
        assert!(parse_block_count(&json!("0x")).is_err());
        assert!(parse_block_count(&json!(-1)).is_err());
        assert!(parse_block_count(&json!(null)).is_err());
    }

    #[test]
    fn percentile_rules() {
        assert!(validate_percentiles(&[]).is_ok());
        assert!(validate_percentiles(&[10.0, 20.0, 30.0]).is_ok());
        assert!(validate_percentiles(&[0.0, 100.0]).is_ok());
        assert!(validate_percentiles(&[20.0, 10.0]).is_err());
        assert!(validate_percentiles(&[10.0, 10.0]).is_err());
        assert!(validate_percentiles(&[101.0]).is_err());
        assert!(validate_percentiles(&[-1.0]).is_err());
        assert!(validate_percentiles(&[f64::NAN]).is_err());
        assert_eq!(
            validate_percentiles(&[1.0; 101]).unwrap_err().code(),
            -32602
        );
    }

    #[test]
    fn log_range_resolution() {
        use BlockNumberOrTag::*;
        assert_eq!(
            resolve_log_range(None, None, 100, 10_000),
            Ok(LogRange::Range(100, 100))
        );
        assert_eq!(
            resolve_log_range(Some(Earliest), Some(Latest), 100, 10_000),
            Ok(LogRange::Range(0, 100))
        );
        // `to` beyond the head is clamped.
        assert_eq!(
            resolve_log_range(Some(Number(90)), Some(Number(500)), 100, 10_000),
            Ok(LogRange::Range(90, 100))
        );
        // Polling from head + 1 returns nothing instead of an error.
        assert_eq!(
            resolve_log_range(Some(Number(101)), None, 100, 10_000),
            Ok(LogRange::Empty)
        );
        assert!(matches!(
            resolve_log_range(Some(Number(50)), Some(Number(40)), 100, 10_000),
            Err(RpcBackendError::InvalidParams(_))
        ));
        // Exactly max_range blocks is allowed, one more is not.
        assert_eq!(
            resolve_log_range(Some(Number(1)), Some(Number(10_000)), 20_000, 10_000),
            Ok(LogRange::Range(1, 10_000))
        );
        assert!(matches!(
            resolve_log_range(Some(Number(0)), Some(Number(10_000)), 20_000, 10_000),
            Err(RpcBackendError::LimitExceeded(_))
        ));
        // 0 disables the limit.
        assert_eq!(
            resolve_log_range(Some(Earliest), None, 20_000, 0),
            Ok(LogRange::Range(0, 20_000))
        );
    }

    #[test]
    fn overrides_rejected_unless_empty() {
        assert!(reject_overrides(None).is_ok());
        assert!(reject_overrides(Some(json!(null))).is_ok());
        assert!(reject_overrides(Some(json!({}))).is_ok());
        assert!(reject_overrides(Some(json!({"0x01": {}}))).is_err());
    }
}

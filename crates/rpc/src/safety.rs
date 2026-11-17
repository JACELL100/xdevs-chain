//! Phase 4 user-safety RPCs (plan §16, §19.2): `xdevs_simulateTransaction`,
//! `xdevs_checkRecipient`, `xdevs_getApprovals`, `xdevs_getVaultQueue`,
//! `xdevs_resolveName` and `xdevs_reverseName`.
//!
//! Simulation runs in the backend ([`RpcBackend::simulate_transaction`], the
//! `xdevs-simulator` crate in the node). Everything else is composed here from the
//! ordinary backend methods (`eth_call`, `eth_getLogs`, `eth_getCode`, ...), so the usual
//! limits apply to every step: the gas cap and EVM time limit of each view call (plus a
//! small per-view gas limit, [`VIEW_CALL_GAS`]), the `eth_getLogs` block-range and result
//! caps, and the per-connection concurrency limit. Each request also has an overall
//! deadline ([`SAFETY_REQUEST_TIMEOUT`]). Log scans are bounded to the last
//! `max_logs_range` blocks: the indexer remains the full-history source.
//!
//! Facts vs opinions (principle 7): `facts` are read from the chain; `advice`, `flags`
//! and `warnings` are heuristics labelled with [`DISCLAIMER`].

use crate::{
    server::{cap_gas, normalize_block_id, RpcContext},
    types::*,
    BackendResult, RpcBackend, RpcBackendError,
};
use alloy_rpc_types_eth::TransactionInput;
use jsonrpsee::types::{ErrorObjectOwned, Params};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
    time::Duration,
};
use xdevs_primitives::{
    reexports::alloy_primitives::U64,
    safety::{
        abi, codes, delegation_target, is_lookalike, is_precompile, is_system_address,
        is_unlimited, normalize_xns_name, sort_warnings, xns_namehash, xns_skeleton_hash, Approval,
        ApprovalKind, HistoryScan, RecipientAdvice, RecipientCheck, RecipientFacts, Severity,
        SimulationReport, VaultActionStatus, VaultCall, VaultQueueItem, Warning, DISCLAIMER,
        ERC1967_IMPLEMENTATION_SLOT, PERMIT2_EXPIRING, SMART_ACCOUNT_IMPL, TOPICS,
        VAULT_EXECUTION_WINDOW, VOTE_CANCEL, WXDS, XNS_REGISTRY, XNS_SUFFIX,
    },
    Address, Bytes, TxKind, B256, U256,
};

type RpcResult<T> = Result<T, ErrorObjectOwned>;

/// Gas limit of every view call made by these methods (`allowance`, `getAction`, ...).
pub const VIEW_CALL_GAS: u64 = 300_000;
/// Max distinct `(token, spender)` pairs `xdevs_getApprovals` checks live.
pub const MAX_APPROVAL_PAIRS: usize = 256;
/// Max token addresses in the `tokens` option of `xdevs_getApprovals`.
pub const MAX_APPROVAL_TOKENS: usize = 256;
/// `xdevs_getVaultQueue` looks at the most recent action ids only.
pub const MAX_VAULT_SCAN: u64 = 256;
/// Past recipients reported as look-alikes at most.
pub const MAX_LOOKALIKES: usize = 16;
/// Overall deadline of one safety request (every step is bounded too).
pub const SAFETY_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// `executeBatch((address,uint256,bytes)[])` selector: how a queued batch is reported.
const EXECUTE_BATCH: [u8; 4] = [0x34, 0xfc, 0xd5, 0xbe];
/// ERC-165 interface id of ERC-1155.
const ERC1155_INTERFACE: [u8; 4] = [0xd9, 0xb6, 0x7a, 0x26];

fn invalid(msg: impl Into<String>) -> RpcBackendError {
    RpcBackendError::InvalidParams(msg.into())
}

/// Run `fut` under [`SAFETY_REQUEST_TIMEOUT`].
async fn bounded<T>(method: &str, fut: impl Future<Output = BackendResult<T>>) -> RpcResult<T> {
    match tokio::time::timeout(SAFETY_REQUEST_TIMEOUT, fut).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(RpcBackendError::LimitExceeded(format!(
            "{method} took longer than {}s",
            SAFETY_REQUEST_TIMEOUT.as_secs()
        ))
        .into()),
    }
}

/// A block pinned by number (so every step of a request reads the same state), its
/// number and timestamp.
async fn pin_block(backend: &dyn RpcBackend, block: BlockId) -> BackendResult<(BlockId, u64, u64)> {
    let b = match block {
        BlockId::Number(tag) => backend.block_by_number(tag, false).await?,
        BlockId::Hash(h) => backend.block_by_hash(h.block_hash, false).await?,
    }
    .ok_or(RpcBackendError::UnknownBlock)?;
    let (number, timestamp) = (b.header.inner.number, b.header.inner.timestamp);
    Ok((
        BlockId::Number(BlockNumberOrTag::Number(number)),
        number,
        timestamp,
    ))
}

/// `eth_call` of a view with [`VIEW_CALL_GAS`]: `None` when it reverts, halts or returns
/// nothing (no contract / not implemented). An aborted call (time limit) is an error.
async fn view(
    backend: &dyn RpcBackend,
    to: Address,
    data: Bytes,
    block: BlockId,
) -> BackendResult<Option<Bytes>> {
    let request = TransactionRequest {
        to: Some(TxKind::Call(to)),
        input: TransactionInput::new(data),
        gas: Some(VIEW_CALL_GAS),
        ..Default::default()
    };
    match backend.call(request, block).await {
        Ok(out) if out.is_empty() => Ok(None),
        Ok(out) => Ok(Some(out)),
        Err(RpcBackendError::Reverted { .. }) => Ok(None),
        Err(RpcBackendError::Rejected(m)) if !m.starts_with("execution aborted") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn view_word(
    backend: &dyn RpcBackend,
    to: Address,
    data: Bytes,
    block: BlockId,
) -> BackendResult<Option<U256>> {
    Ok(view(backend, to, data, block)
        .await?
        .and_then(|out| abi::word(&out, 0)))
}

fn word(a: Address) -> B256 {
    abi::address_word(a)
}

// ---------------------------------------------------------------------------
// xdevs_simulateTransaction
// ---------------------------------------------------------------------------

/// `xdevs_simulateTransaction(tx, blockTag?)`.
pub(crate) async fn simulate_transaction(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<SimulationReport> {
    let mut seq = params.sequence();
    let mut request: TransactionRequest = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    if request.from.is_none() {
        return Err(invalid("`from` is required to simulate a transaction").into());
    }
    cap_gas(&mut request, ctx.config.gas_cap);
    Ok(ctx.backend.simulate_transaction(request, block).await?)
}

// ---------------------------------------------------------------------------
// XNS
// ---------------------------------------------------------------------------

/// Forward resolution (`XNSRegistry.resolveAddr(namehash)`) of a normalized name.
async fn resolve(
    backend: &dyn RpcBackend,
    name: &str,
    block: BlockId,
) -> BackendResult<Option<Address>> {
    let data = abi::encode_call("resolveAddr(bytes32)", &[xns_namehash(name)]);
    Ok(view_word(backend, XNS_REGISTRY, data, block)
        .await?
        .map(|w| Address::from_word(B256::from(w)))
        .filter(|a| !a.is_zero()))
}

/// Forward-verified primary name (`XNSRegistry.primaryName(address)`).
async fn primary_name(
    backend: &dyn RpcBackend,
    address: Address,
    block: BlockId,
) -> BackendResult<Option<String>> {
    let data = abi::encode_call("primaryName(address)", &[word(address)]);
    Ok(view(backend, XNS_REGISTRY, data, block)
        .await?
        .and_then(|out| {
            let s = abi::bytes(&out, 0)?;
            let s = std::str::from_utf8(s).ok()?;
            normalize_xns_name(s).ok()
        }))
}

/// `xdevs_resolveName(name, blockTag?)`: address or `null`.
pub(crate) async fn resolve_name(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<Address>> {
    let mut seq = params.sequence();
    let name: String = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    let name = normalize_xns_name(&name).map_err(|e| invalid(format!("invalid XNS name: {e}")))?;
    bounded("xdevs_resolveName", async {
        let (block, _, _) = pin_block(&*ctx.backend, block).await?;
        resolve(&*ctx.backend, &name, block).await
    })
    .await
}

/// `xdevs_reverseName(address, blockTag?)`: forward-verified primary name or `null`.
pub(crate) async fn reverse_name(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Option<String>> {
    let mut seq = params.sequence();
    let address: Address = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    bounded("xdevs_reverseName", async {
        let (block, _, _) = pin_block(&*ctx.backend, block).await?;
        primary_name(&*ctx.backend, address, block).await
    })
    .await
}

// ---------------------------------------------------------------------------
// xdevs_checkRecipient
// ---------------------------------------------------------------------------

/// `xdevs_checkRecipient(to)` / `xdevs_checkRecipient(from, to, blockTag?)`; `to` is an
/// address or an XNS name, `from` may be `null`.
pub(crate) async fn check_recipient(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<RecipientCheck> {
    let values: Vec<Value> = params.parse()?;
    let parse_from = |v: &Value| -> Result<Option<Address>, RpcBackendError> {
        match v {
            Value::Null => Ok(None),
            v => serde_json::from_value(v.clone())
                .map(Some)
                .map_err(|e| invalid(format!("invalid `from`: {e}"))),
        }
    };
    let (from, to, block) = match values.as_slice() {
        [to] => (None, to, None),
        [from, to] => (parse_from(from)?, to, None),
        [from, to, block] => (
            parse_from(from)?,
            to,
            Some(
                serde_json::from_value::<BlockId>(block.clone())
                    .map_err(|e| invalid(format!("invalid block: {e}")))?,
            ),
        ),
        _ => {
            return Err(invalid(
                "expected [to], [from, to] or [from, to, block] (to: address or XNS name)",
            )
            .into())
        }
    };
    let Value::String(to) = to else {
        return Err(invalid("`to` must be an address or an XNS name").into());
    };
    let block = normalize_block_id(block);
    bounded(
        "xdevs_checkRecipient",
        check_recipient_inner(&ctx, from, to.clone(), block),
    )
    .await
}

/// Address of `input` (address or name) and the normalized name it came from.
async fn resolve_input(
    backend: &dyn RpcBackend,
    input: &str,
    block: BlockId,
) -> BackendResult<(Address, Option<String>)> {
    let s = input.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        let a = s
            .parse::<Address>()
            .map_err(|_| invalid(format!("invalid address {s:?}")))?;
        return Ok((a, None));
    }
    let name =
        normalize_xns_name(s).map_err(|e| invalid(format!("invalid XNS name {s:?}: {e}")))?;
    if let Some(a) = resolve(backend, &name, block).await? {
        return Ok((a, Some(name)));
    }
    // Not registered. The registry gives each live name a unique confusable skeleton
    // (`XNSRegistry.skeletonHolder`), so a look-alike of a registered name never resolves:
    // say which name holds the skeleton.
    let mut hint = String::new();
    let label = name.strip_suffix(XNS_SUFFIX).unwrap_or(&name);
    if !label.contains('.') {
        let data = abi::encode_call("skeletonHolder(bytes32)", &[xns_skeleton_hash(label)]);
        if let Some(holder) = view_word(backend, XNS_REGISTRY, data, block).await? {
            let holder = B256::from(holder);
            if !holder.is_zero() && holder != xns_namehash(&name) {
                let data = abi::encode_call("resolveAddr(bytes32)", &[holder]);
                let holder_addr = view_word(backend, XNS_REGISTRY, data, block)
                    .await?
                    .map(|w| Address::from_word(B256::from(w)))
                    .filter(|a| !a.is_zero());
                hint = match holder_addr {
                    Some(a) => format!(
                        "; LOOKALIKE_NAME: it looks like a registered name that resolves to {a} \
                         — check the spelling (advice, not fact)"
                    ),
                    None => "; LOOKALIKE_NAME: it looks like a registered name — check the \
                             spelling (advice, not fact)"
                        .to_string(),
                };
            }
        }
    }
    Err(invalid(format!(
        "{name} is not registered (or expired){hint}"
    )))
}

async fn check_recipient_inner(
    ctx: &RpcContext,
    from: Option<Address>,
    to: String,
    block: BlockId,
) -> BackendResult<RecipientCheck> {
    let b = &*ctx.backend;
    let (block, number, _) = pin_block(b, block).await?;
    let (address, resolved_from) = resolve_input(b, &to, block).await?;

    // Facts.
    let code = b.code(address, block).await?;
    let delegated_to = delegation_target(&code);
    let is_proxy_account = !code.is_empty()
        && delegated_to.is_none()
        && b.storage_at(address, ERC1967_IMPLEMENTATION_SLOT, block)
            .await?
            == SMART_ACCOUNT_IMPL.into_word();
    let eoa_lock = match b.eoa_lock(address, block).await {
        Ok(info) => Some(info.status),
        Err(RpcBackendError::Unsupported(_)) => None,
        Err(e) => return Err(e),
    };
    let facts = RecipientFacts {
        address,
        resolved_from: resolved_from.clone(),
        is_zero_address: address.is_zero(),
        is_precompile: is_precompile(address),
        is_system_contract: is_system_address(address),
        is_contract: !code.is_empty(),
        code_size: code.len() as u64,
        is_smart_account: delegated_to == Some(SMART_ACCOUNT_IMPL) || is_proxy_account,
        delegated_to,
        eoa_lock,
        nonce: b.transaction_count(address, block).await?,
        balance: b.balance(address, block).await?,
        primary_name: primary_name(b, address, block).await?,
    };

    // Advice.
    let mut flags = Vec::new();
    if facts.is_zero_address {
        flags.push(Warning::new(
            codes::ZERO_ADDRESS,
            Severity::High,
            "This is the zero address: anything sent to it is lost.",
        ));
    } else if facts.is_precompile {
        flags.push(Warning::new(
            codes::PRECOMPILE,
            Severity::High,
            "This is a precompile, not an account: funds sent to it cannot be recovered.",
        ));
    } else if address == WXDS {
        flags.push(Warning::new(
            codes::SYSTEM_CONTRACT,
            Severity::Info,
            "This is the WXDS contract: native coins sent to it are wrapped into WXDS for the sender.",
        ));
    } else if facts.is_system_contract {
        flags.push(Warning::new(
            codes::SYSTEM_CONTRACT,
            Severity::Medium,
            "This is a protocol system contract, not a personal account: funds sent to it may not be recoverable.",
        ));
    } else if facts.is_contract && !facts.is_smart_account && delegated_to.is_none() {
        flags.push(Warning::new(
            codes::CONTRACT_RECIPIENT,
            Severity::Info,
            "This address is a contract: make sure it can handle what you send.",
        ));
    } else if !facts.is_contract && facts.nonce == 0 && facts.balance.is_zero() {
        flags.push(Warning::new(
            codes::NEW_ADDRESS,
            Severity::Low,
            "This address has never been used on chain (no transactions, no balance).",
        ));
    }
    if let Some(name) = &resolved_from {
        if facts.primary_name.as_ref() != Some(name) {
            flags.push(Warning::new(
                codes::NAME_MISMATCH,
                Severity::Info,
                format!(
                    "{name} points to this address, but the address's primary name is {}.",
                    facts.primary_name.as_deref().unwrap_or("not set")
                ),
            ));
        }
    }

    let mut first_time = None;
    let mut look_alike_of = Vec::new();
    let mut history = None;
    if let Some(from) = from.filter(|f| *f != address) {
        let scan = scan_history(ctx, from, address, number).await?;
        first_time = Some(scan.first_time);
        look_alike_of = scan.look_alikes;
        history = Some(scan.range);
        if scan.first_time {
            flags.push(Warning::new(
                codes::FIRST_TIME_RECIPIENT,
                Severity::Low,
                format!(
                    "You have not sent tokens to this address in blocks {}..={}.",
                    scan.range.from_block, scan.range.to_block
                ),
            ));
        }
        if !look_alike_of.is_empty() {
            flags.push(Warning::new(
                codes::LOOKALIKE_RECIPIENT,
                Severity::High,
                format!(
                    "This address looks like {} you sent to before (same first and last characters) — possible address poisoning. Compare every character.",
                    look_alike_of[0]
                ),
            ));
        }
        if scan.poisoned {
            flags.push(Warning::new(
                codes::POISONING_PATTERN,
                Severity::High,
                "This address sent you a zero-value token transfer — a common address-poisoning trick. Do not copy addresses from your history.",
            ));
        }
    }
    sort_warnings(&mut flags);

    Ok(RecipientCheck {
        address,
        first_time: first_time.unwrap_or(false),
        look_alike_of: look_alike_of.clone(),
        name: facts.primary_name.clone(),
        is_contract: facts.is_contract,
        warnings: flags.clone(),
        facts,
        advice: RecipientAdvice {
            first_time,
            look_alike_of,
            flags,
            history,
            disclaimer: DISCLAIMER.to_string(),
        },
        block_number: U64::from(number),
    })
}

struct HistoryResult {
    first_time: bool,
    look_alikes: Vec<Address>,
    poisoned: bool,
    range: HistoryScan,
}

/// `eth_getLogs` over the newest `span` blocks up to `head`, shrinking the window (and
/// marking it incomplete) while the result cap is exceeded.
async fn bounded_logs(
    ctx: &RpcContext,
    head: u64,
    filter: impl Fn(u64) -> Filter,
) -> BackendResult<(Vec<RpcLog>, HistoryScan)> {
    let range = ctx.config.max_logs_range;
    let mut span = if range == 0 {
        head + 1
    } else {
        range.min(head + 1)
    };
    let max = ctx.config.max_logs_results;
    let limit = (max > 0).then_some(max);
    let mut complete = true;
    loop {
        let from_block = head + 1 - span;
        match ctx.backend.logs(filter(from_block), limit).await {
            Ok(logs) => {
                return Ok((
                    logs,
                    HistoryScan {
                        from_block,
                        to_block: head,
                        complete,
                    },
                ))
            }
            Err(RpcBackendError::LimitExceeded(_)) if span > 1 => {
                span = (span / 10).max(1);
                complete = false;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Token `Transfer` logs sent by `from` (first-time / look-alike) and zero-value ones
/// sent by `to` to `from` (poisoning pattern), within the bounded window.
async fn scan_history(
    ctx: &RpcContext,
    from: Address,
    to: Address,
    head: u64,
) -> BackendResult<HistoryResult> {
    let transfer = TOPICS.transfer;
    let (sent, range) = bounded_logs(ctx, head, |from_block| {
        Filter::new()
            .from_block(from_block)
            .to_block(head)
            .event_signature(transfer)
            .topic1(word(from))
    })
    .await?;
    let recipients: BTreeSet<Address> = sent
        .iter()
        .filter_map(|l| l.inner.topics().get(2).map(|t| Address::from_word(*t)))
        .collect();
    let look_alikes = recipients
        .iter()
        .copied()
        .filter(|p| is_lookalike(*p, to))
        .take(MAX_LOOKALIKES)
        .collect();
    let poisoned = match bounded_logs(ctx, head, |from_block| {
        Filter::new()
            .from_block(from_block.max(range.from_block))
            .to_block(head)
            .event_signature(transfer)
            .topic1(word(to))
            .topic2(word(from))
    })
    .await
    {
        Ok((logs, _)) => logs.iter().any(|l| {
            l.inner.topics().len() == 3 && abi::word(&l.inner.data.data, 0) == Some(U256::ZERO)
        }),
        Err(RpcBackendError::LimitExceeded(_)) => false,
        Err(e) => return Err(e),
    };
    Ok(HistoryResult {
        first_time: !recipients.contains(&to),
        look_alikes,
        poisoned,
        range,
    })
}

// ---------------------------------------------------------------------------
// xdevs_getApprovals
// ---------------------------------------------------------------------------

/// Options object of `xdevs_getApprovals`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovalOptions {
    tokens: Option<Vec<Address>>,
    from_block: Option<BlockNumberOrTag>,
}

/// `xdevs_getApprovals(owner, options?, blockTag?)` or `xdevs_getApprovals(owner,
/// blockTag)`; `options = { tokens?: Address[], fromBlock?: Quantity }`.
pub(crate) async fn get_approvals(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Vec<Approval>> {
    let mut seq = params.sequence();
    let owner: Address = seq.next()?;
    let second: Option<Value> = seq.optional_next()?;
    let third: Option<BlockId> = seq.optional_next()?;
    let is_options = |v: &Value| {
        v.as_object()
            .is_some_and(|o| o.contains_key("tokens") || o.contains_key("fromBlock"))
    };
    let (options, block) = match second {
        Some(v) if is_options(&v) => (
            serde_json::from_value::<ApprovalOptions>(v)
                .map_err(|e| invalid(format!("invalid options: {e}")))?,
            third,
        ),
        Some(Value::Null) | None => (ApprovalOptions::default(), third),
        Some(v) => (
            ApprovalOptions::default(),
            Some(
                serde_json::from_value::<BlockId>(v)
                    .map_err(|e| invalid(format!("invalid block: {e}")))?,
            ),
        ),
    };
    if options
        .tokens
        .as_ref()
        .is_some_and(|t| t.len() > MAX_APPROVAL_TOKENS)
    {
        return Err(RpcBackendError::LimitExceeded(format!(
            "too many tokens: max {MAX_APPROVAL_TOKENS}"
        ))
        .into());
    }
    let block = normalize_block_id(block);
    bounded(
        "xdevs_getApprovals",
        get_approvals_inner(&ctx, owner, options, block),
    )
    .await
}

/// Approval class of a candidate: plain / XDRC-20 allowance, Permit2, operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Allowance,
    Permit2,
    Operator,
}

async fn get_approvals_inner(
    ctx: &RpcContext,
    owner: Address,
    options: ApprovalOptions,
    block: BlockId,
) -> BackendResult<Vec<Approval>> {
    let b = &*ctx.backend;
    let (block, head, now) = pin_block(b, block).await?;
    let range = ctx.config.max_logs_range;
    let from_block = match options.from_block {
        Some(BlockNumberOrTag::Number(n)) => n,
        Some(BlockNumberOrTag::Earliest) => 0,
        _ if range == 0 => 0,
        _ => head.saturating_sub(range - 1),
    };
    if from_block > head {
        return Ok(Vec::new());
    }
    if range > 0 && head - from_block >= range {
        return Err(RpcBackendError::LimitExceeded(format!(
            "exceed maximum block range: {range} (scan at most the last {range} blocks; the \
             indexer API has the full history)"
        )));
    }
    let t = &*TOPICS;
    let mut filter = Filter::new()
        .from_block(from_block)
        .to_block(head)
        .event_signature(vec![
            t.approval,
            t.approval_with_expiry,
            t.approval_for_all,
            t.permit2_approval,
            t.permit2_permit,
        ])
        .topic1(word(owner));
    let tokens: Option<BTreeSet<Address>> = options.tokens.map(|t| t.into_iter().collect());
    if let Some(tokens) = &tokens {
        let mut addresses: Vec<Address> = tokens.iter().copied().collect();
        addresses.push(PERMIT2_EXPIRING);
        filter = filter.address(addresses);
    }
    let max = ctx.config.max_logs_results;
    let logs = b
        .logs(filter, (max > 0).then_some(max))
        .await
        .map_err(|e| match e {
            RpcBackendError::LimitExceeded(m) => RpcBackendError::LimitExceeded(format!(
                "{m}; narrow the scan with `tokens` or `fromBlock`, or use the indexer API"
            )),
            e => e,
        })?;

    // Latest event per (class, token, spender); logs come in chain order.
    let mut candidates: BTreeMap<(Class, Address, Address), (u64, Option<B256>)> = BTreeMap::new();
    let mut xdrc20 = BTreeSet::new();
    for l in &logs {
        let topics = l.inner.topics();
        let (Some(topic0), n) = (topics.first().copied(), topics.len()) else {
            continue;
        };
        let at = (l.block_number.unwrap_or_default(), l.transaction_hash);
        let emitter = l.inner.address;
        let topic_addr = |i: usize| Address::from_word(topics[i]);
        let key = if n == 3 && (topic0 == t.approval || topic0 == t.approval_with_expiry) {
            if topic0 == t.approval_with_expiry {
                xdrc20.insert(emitter);
            }
            (Class::Allowance, emitter, topic_addr(2))
        } else if n == 3 && topic0 == t.approval_for_all {
            (Class::Operator, emitter, topic_addr(2))
        } else if n == 4
            && emitter == PERMIT2_EXPIRING
            && (topic0 == t.permit2_approval || topic0 == t.permit2_permit)
        {
            let token = topic_addr(2);
            if tokens.as_ref().is_some_and(|t| !t.contains(&token)) {
                continue;
            }
            (Class::Permit2, token, topic_addr(3))
        } else {
            continue;
        };
        candidates.insert(key, at);
    }
    if candidates.len() > MAX_APPROVAL_PAIRS {
        return Err(RpcBackendError::LimitExceeded(format!(
            "more than {MAX_APPROVAL_PAIRS} (token, spender) pairs to check; narrow the scan \
             with `tokens` or `fromBlock`, or use the indexer API"
        )));
    }

    let mut symbols: BTreeMap<Address, Option<String>> = BTreeMap::new();
    let mut out = Vec::new();
    for ((class, token, spender), (last_block, tx)) in candidates {
        let (kind, raw, expires) = match class {
            Class::Allowance => {
                let data =
                    abi::encode_call("allowance(address,address)", &[word(owner), word(spender)]);
                let Some(amount) = view_word(b, token, data, block).await? else {
                    continue;
                };
                if amount.is_zero() {
                    continue;
                }
                let data = abi::encode_call(
                    "allowanceExpiry(address,address)",
                    &[word(owner), word(spender)],
                );
                let expiry = view(b, token, data, block)
                    .await?
                    .filter(|out| out.len() == 32)
                    .and_then(|out| abi::u64_word(&out, 0))
                    .filter(|e| *e != 0);
                if xdrc20.contains(&token) || expiry.is_some() {
                    (ApprovalKind::Xdrc20, amount, expiry)
                } else {
                    (ApprovalKind::Erc20, amount, None)
                }
            }
            Class::Permit2 => {
                let data = abi::encode_call(
                    "allowance(address,address,address)",
                    &[word(owner), word(token), word(spender)],
                );
                let Some(out) = view(b, PERMIT2_EXPIRING, data, block).await? else {
                    continue;
                };
                let (Some(amount), Some(expiration)) = (abi::word(&out, 0), abi::u64_word(&out, 1))
                else {
                    continue;
                };
                if amount.is_zero() || (expiration != 0 && expiration <= now) {
                    continue;
                }
                (
                    ApprovalKind::Permit2,
                    amount,
                    (expiration != 0).then_some(expiration),
                )
            }
            Class::Operator => {
                let data = abi::encode_call(
                    "isApprovedForAll(address,address)",
                    &[word(owner), word(spender)],
                );
                if view_word(b, token, data, block).await? != Some(U256::from(1)) {
                    continue;
                }
                let mut id = [0u8; 32];
                id[..4].copy_from_slice(&ERC1155_INTERFACE);
                let data = abi::encode_call("supportsInterface(bytes4)", &[B256::from(id)]);
                let kind = if view_word(b, token, data, block).await? == Some(U256::from(1)) {
                    ApprovalKind::Erc1155All
                } else {
                    ApprovalKind::Erc721All
                };
                (kind, U256::from(1), None)
            }
        };
        let symbol = match symbols.get(&token) {
            Some(s) => s.clone(),
            None => {
                let data = abi::encode_call("symbol()", &[]);
                let s = view(b, token, data, block)
                    .await?
                    .and_then(|out| abi::string_or_bytes32(&out));
                symbols.insert(token, s.clone());
                s
            }
        };
        let unlimited = kind.is_operator() || is_unlimited(raw);
        let amount = if kind.is_operator() {
            "all".to_string()
        } else if unlimited {
            "unlimited".to_string()
        } else {
            raw.to_string()
        };
        out.push(Approval {
            kind,
            token,
            symbol,
            owner,
            spender,
            amount,
            raw_amount: raw,
            unlimited,
            expires,
            last_updated_block: U64::from(last_block),
            transaction_hash: tx,
        });
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.last_updated_block));
    Ok(out)
}

// ---------------------------------------------------------------------------
// xdevs_getVaultQueue
// ---------------------------------------------------------------------------

/// `xdevs_getVaultQueue(account, blockTag?)`: pending (queued or ready, not cancelled,
/// executed, invalidated or expired) Guardian Vault actions, oldest first.
pub(crate) async fn get_vault_queue(
    params: Params<'static>,
    ctx: Arc<RpcContext>,
) -> RpcResult<Vec<VaultQueueItem>> {
    let mut seq = params.sequence();
    let account: Address = seq.next()?;
    let block = normalize_block_id(seq.optional_next()?);
    bounded(
        "xdevs_getVaultQueue",
        vault_queue(&*ctx.backend, account, block),
    )
    .await
}

/// Decode the `Call[]` encoded at byte `offset` of `data`.
pub(crate) fn decode_calls(data: &[u8], offset: usize) -> Option<Vec<VaultCall>> {
    let n = abi::usize_at(data, offset)?;
    let base = offset.checked_add(32)?;
    let mut calls = Vec::with_capacity(n.min(64));
    for i in 0..n {
        let tuple = base.checked_add(abi::usize_at(data, base + i * 32)?)?;
        let target = Address::from_word(B256::from(abi::word_at(data, tuple)?));
        let value = abi::word_at(data, tuple.checked_add(32)?)?;
        let data_offset = abi::usize_at(data, tuple.checked_add(64)?)?;
        let bytes = abi::bytes_at(data, tuple.checked_add(data_offset)?)?;
        calls.push(VaultCall {
            target,
            value,
            data: Bytes::copy_from_slice(bytes),
        });
    }
    Some(calls)
}

/// The pending actions of `account` among its last [`MAX_VAULT_SCAN`] action ids.
pub(crate) async fn vault_queue(
    backend: &dyn RpcBackend,
    account: Address,
    block: BlockId,
) -> BackendResult<Vec<VaultQueueItem>> {
    let (block, _, now) = pin_block(backend, block).await?;
    let data = abi::encode_call("lastActionId()", &[]);
    let Some(last) = view_word(backend, account, data, block).await? else {
        return Ok(Vec::new()); // not a smart account
    };
    let Ok(last) = u64::try_from(last) else {
        return Ok(Vec::new());
    };
    let first = last.saturating_sub(MAX_VAULT_SCAN - 1).max(1);
    let mut out = Vec::new();
    for id in first..=last {
        let id = U256::from(id);
        let data = abi::encode_call("getAction(uint256)", &[abi::u256_word(id)]);
        let Some(ret) = view(backend, account, data, block).await? else {
            continue;
        };
        let (Some(execute_after), Some(status), Some(valid), Some(offset)) = (
            abi::u64_word(&ret, 0),
            abi::u64_word(&ret, 1),
            abi::word(&ret, 2),
            abi::usize_at(&ret, 96),
        ) else {
            continue;
        };
        // AccountStorage.ActionStatus.Queued, current vault epoch.
        if status != 1 || valid.is_zero() {
            continue;
        }
        let expires_at = execute_after.saturating_add(VAULT_EXECUTION_WINDOW);
        if now > expires_at {
            continue;
        }
        let Some(calls) = decode_calls(&ret, offset) else {
            continue;
        };
        let (target, value, data) = match calls.as_slice() {
            [single] => (single.target, single.value, single.data.clone()),
            many => {
                let total = many
                    .iter()
                    .fold(U256::ZERO, |acc, c| acc.saturating_add(c.value));
                let mut d = EXECUTE_BATCH.to_vec();
                d.extend_from_slice(abi::u256_word(U256::from(32)).as_slice());
                d.extend_from_slice(&ret[offset..]);
                (account, total, Bytes::from(d))
            }
        };
        let data_votes = abi::encode_call(
            "guardianVotes(uint8,uint256)",
            &[abi::u256_word(U256::from(VOTE_CANCEL)), abi::u256_word(id)],
        );
        let cancel_votes = view(backend, account, data_votes, block)
            .await?
            .and_then(|out| abi::u64_word(&out, 1))
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(0);
        out.push(VaultQueueItem {
            account,
            id,
            target,
            value,
            data,
            execute_after: U64::from(execute_after),
            expires_at: U64::from(expires_at),
            status: if now >= execute_after {
                VaultActionStatus::Ready
            } else {
                VaultActionStatus::Queued
            },
            calls,
            cancel_votes,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(v: u64) -> [u8; 32] {
        abi::u256_word(U256::from(v)).0
    }

    #[test]
    fn decodes_call_arrays() {
        // getAction return: (executeAfter, status, valid, Call[]) with two calls.
        let target = Address::repeat_byte(0x11);
        let mut ret = Vec::new();
        for v in [100, 1, 1, 128] {
            ret.extend_from_slice(&w(v));
        }
        // Call[] at 128: len 2, offsets (relative to the element area) 64 and 192.
        for v in [2, 64, 192] {
            ret.extend_from_slice(&w(v));
        }
        // Call 0: target, value 5, data offset 96, data len 0.
        ret.extend_from_slice(abi::address_word(target).as_slice());
        for v in [5, 96, 0] {
            ret.extend_from_slice(&w(v));
        }
        // Call 1: target, value 7, data offset 96, data len 2 + 0xbeef.
        ret.extend_from_slice(abi::address_word(target).as_slice());
        for v in [7, 96, 2] {
            ret.extend_from_slice(&w(v));
        }
        let mut tail = [0u8; 32];
        tail[..2].copy_from_slice(&[0xbe, 0xef]);
        ret.extend_from_slice(&tail);
        let calls = decode_calls(&ret, 128).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].target, calls[0].value), (target, U256::from(5)));
        assert!(calls[0].data.is_empty());
        assert_eq!(calls[1].data.as_ref(), &[0xbe, 0xef]);
        // Truncated / hostile encodings are rejected instead of panicking.
        assert!(decode_calls(&ret[..ret.len() - 40], 128).is_none());
        assert!(decode_calls(&ret, 4_000).is_none());
        let mut huge = ret.clone();
        huge[128..160].copy_from_slice(&[0xff; 32]);
        assert!(decode_calls(&huge, 128).is_none());
    }
}

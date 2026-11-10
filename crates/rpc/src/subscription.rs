//! `eth_subscribe` / `eth_unsubscribe` over WebSocket (plan §19.1, §19.3).
//!
//! Subscription kinds:
//!
//! * `newHeads`: one notification per committed block, the header exactly as
//!   `eth_getBlockByNumber(n, false)` returns it minus the transactions.
//! * `logs` (optional filter `{address, topics}`): every matching log of every committed
//!   block, in block order, one notification per log, the log objects exactly as
//!   `eth_getLogs` returns them (`removed: false`: BFT finality, no reorgs). The
//!   address/topic semantics are those of `eth_getLogs`.
//! * `newPendingTransactions`: the hash of every transaction admitted to the mempool
//!   (`true` = full objects is not supported).
//!
//! Limits (§19.3): at most [`crate::RpcServerConfig::max_subscriptions_per_connection`]
//! per connection (jsonrpsee, error -32006) and
//! [`crate::RpcServerConfig::max_subscriptions`] per server (error -32005); log filters
//! with at most [`MAX_FILTER_ADDRESSES`] addresses and [`MAX_FILTER_TOPICS`] topics.
//! Events go through bounded broadcast channels ([`crate::events::ChainEvents`]): a
//! subscriber that falls behind by more than the channel capacity, or whose connection
//! does not drain a notification within [`SEND_TIMEOUT`], has its subscription closed
//! with an error notification
//! (`{"method":"eth_subscription","params":{"subscription":id,"error":"…"}}`).
//!
//! Over plain HTTP `eth_subscribe` / `eth_unsubscribe` fail with -32601 "notifications
//! not supported" (geth's wording) instead of jsonrpsee's generic internal error.

use crate::{events::BlockEvent, server::RpcContext, types::*, RpcBackendError, RpcServerConfig};
use http::{header::UPGRADE, Request};
use jsonrpsee::{
    core::{
        middleware::{Batch, Notification, RpcServiceT},
        server::{
            MethodResponse, PendingSubscriptionSink, SendTimeoutError, SubscriptionCloseResponse,
            SubscriptionMessage, SubscriptionSink,
        },
        RegisterMethodError, SubscriptionError,
    },
    server::IdProvider,
    types::{ErrorObjectOwned, Params, SubscriptionId},
    RpcModule,
};
use serde_json::value::RawValue;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::broadcast::{self, error::RecvError};
use xdevs_primitives::{keccak256, B256};

/// Subscribe method.
pub const SUBSCRIBE: &str = "eth_subscribe";
/// Notification method.
pub const NOTIFICATION: &str = "eth_subscription";
/// Unsubscribe method.
pub const UNSUBSCRIBE: &str = "eth_unsubscribe";
/// Max addresses in a `logs` subscription filter.
pub const MAX_FILTER_ADDRESSES: usize = 256;
/// Max topic values (all four positions together) in a `logs` subscription filter.
pub const MAX_FILTER_TOPICS: usize = 256;
/// A notification the connection does not accept within this time closes the
/// subscription (the client stopped reading).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// What a subscription delivers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SubscriptionKind {
    NewHeads,
    Logs(Box<Filter>),
    NewPendingTransactions,
}

impl SubscriptionKind {
    fn name(&self) -> &'static str {
        match self {
            Self::NewHeads => "newHeads",
            Self::Logs(_) => "logs",
            Self::NewPendingTransactions => "newPendingTransactions",
        }
    }
}

fn invalid(msg: impl Into<String>) -> ErrorObjectOwned {
    RpcBackendError::InvalidParams(msg.into()).into()
}

/// Parse `eth_subscribe` params: `["newHeads"]`, `["logs", filter?]`,
/// `["newPendingTransactions", false?]`.
pub(crate) fn parse_subscription(
    params: &Params<'_>,
) -> Result<SubscriptionKind, ErrorObjectOwned> {
    let mut seq = params.sequence();
    let kind: String = seq.next()?;
    match kind.as_str() {
        "newHeads" => Ok(SubscriptionKind::NewHeads),
        "logs" => {
            let filter: Option<Filter> = seq.optional_next()?;
            let filter = filter.unwrap_or_default();
            check_log_filter(&filter)?;
            Ok(SubscriptionKind::Logs(Box::new(filter)))
        }
        "newPendingTransactions" => match seq.optional_next::<bool>()? {
            Some(true) => Err(invalid(
                "newPendingTransactions with full transaction objects is not supported; \
                 subscribe without `true` to receive hashes",
            )),
            _ => Ok(SubscriptionKind::NewPendingTransactions),
        },
        other => Err(invalid(format!(
            "unsupported subscription type {other:?} (supported: newHeads, logs, \
             newPendingTransactions)"
        ))),
    }
}

/// Size limits of a `logs` filter, and no historical ranges (live logs only).
pub(crate) fn check_log_filter(filter: &Filter) -> Result<(), ErrorObjectOwned> {
    if filter.address.len() > MAX_FILTER_ADDRESSES {
        return Err(RpcBackendError::LimitExceeded(format!(
            "too many addresses in filter: {} > {MAX_FILTER_ADDRESSES}",
            filter.address.len()
        ))
        .into());
    }
    let topics: usize = filter.topics.iter().map(|t| t.len()).sum();
    if topics > MAX_FILTER_TOPICS {
        return Err(RpcBackendError::LimitExceeded(format!(
            "too many topics in filter: {topics} > {MAX_FILTER_TOPICS}"
        ))
        .into());
    }
    match filter.block_option {
        FilterBlockOption::AtBlockHash(_) => Err(invalid(
            "blockHash is not supported in a logs subscription (use eth_getLogs)",
        )),
        FilterBlockOption::Range {
            from_block,
            to_block,
        } => {
            let historical = |b: Option<BlockNumberOrTag>| {
                matches!(
                    b,
                    Some(BlockNumberOrTag::Number(_) | BlockNumberOrTag::Earliest)
                )
            };
            if historical(from_block) || historical(to_block) {
                return Err(invalid(
                    "a logs subscription only delivers new logs; use eth_getLogs for a \
                     block range",
                ));
            }
            Ok(())
        }
    }
}

/// Register `eth_subscribe` / `eth_unsubscribe` on `module`.
pub(crate) fn register(module: &mut RpcModule<RpcContext>) -> Result<(), RegisterMethodError> {
    module.register_subscription(
        SUBSCRIBE,
        NOTIFICATION,
        UNSUBSCRIBE,
        |params, pending, ctx, _| subscribe(params, pending, ctx),
    )?;
    Ok(())
}

/// Event source of an accepted subscription.
enum Source {
    Heads(broadcast::Receiver<Arc<BlockEvent>>),
    Logs(broadcast::Receiver<Arc<BlockEvent>>, Box<Filter>),
    PendingTxs(broadcast::Receiver<B256>),
}

async fn subscribe(
    params: Params<'static>,
    pending: PendingSubscriptionSink,
    ctx: Arc<RpcContext>,
) -> SubscriptionCloseResponse {
    let kind = match parse_subscription(&params) {
        Ok(kind) => kind,
        Err(e) => {
            pending.reject(e).await;
            return SubscriptionCloseResponse::None;
        }
    };
    // Subscribe to the source before answering, so every event committed after the
    // response is delivered.
    let name = kind.name();
    let source = match kind {
        SubscriptionKind::NewHeads => ctx.backend.subscribe_blocks().map(Source::Heads),
        SubscriptionKind::Logs(filter) => ctx
            .backend
            .subscribe_blocks()
            .map(|rx| Source::Logs(rx, filter)),
        SubscriptionKind::NewPendingTransactions => {
            ctx.backend.subscribe_pending_txs().map(Source::PendingTxs)
        }
    };
    let Some(source) = source else {
        let e = RpcBackendError::Unsupported(format!(
            "{SUBSCRIBE} {name} is not available on this node"
        ));
        pending.reject(e).await;
        return SubscriptionCloseResponse::None;
    };
    let Ok(permit) = ctx.subscriptions.clone().try_acquire_owned() else {
        let e = RpcBackendError::LimitExceeded(format!(
            "too many subscriptions on this server (max {})",
            ctx.config.max_subscriptions
        ));
        pending.reject(e).await;
        return SubscriptionCloseResponse::None;
    };
    let Ok(sink) = pending.accept().await else {
        return SubscriptionCloseResponse::None;
    };
    tracing::debug!(target: "xdevs::rpc", kind = name, id = ?sink.subscription_id(), "subscription started");

    let result = match source {
        Source::Heads(rx) => {
            pump(&sink, rx, |ev: Arc<BlockEvent>, out| push(out, &ev.header)).await
        }
        Source::Logs(rx, filter) => {
            pump(&sink, rx, |ev: Arc<BlockEvent>, out| {
                for log in ev.logs.iter().filter(|l| filter.matches(&l.inner)) {
                    push(out, log);
                }
            })
            .await
        }
        Source::PendingTxs(rx) => pump(&sink, rx, |hash: B256, out| push(out, &hash)).await,
    };
    drop(permit);
    match result {
        Ok(()) => SubscriptionCloseResponse::None,
        Err(reason) => {
            tracing::debug!(target: "xdevs::rpc", id = ?sink.subscription_id(), %reason, "subscription closed");
            SubscriptionCloseResponse::NotifErr(SubscriptionError::from(reason))
        }
    }
}

/// Serialise `value` as a notification payload (the sink adds method and id).
fn push(out: &mut Vec<Box<RawValue>>, value: &impl serde::Serialize) {
    match serde_json::value::to_raw_value(value) {
        Ok(json) => out.push(json),
        Err(e) => {
            tracing::error!(target: "xdevs::rpc", error = %e, "failed to serialise a notification")
        }
    }
}

/// Forward events from `rx` to `sink` until the client unsubscribes or disconnects
/// (`Ok`), or the subscriber cannot keep up (`Err(reason)`).
async fn pump<T, F>(
    sink: &SubscriptionSink,
    mut rx: broadcast::Receiver<T>,
    mut notify: F,
) -> Result<(), String>
where
    T: Clone,
    F: FnMut(T, &mut Vec<Box<RawValue>>),
{
    let mut out = Vec::new();
    loop {
        let event = tokio::select! {
            biased;
            _ = sink.closed() => return Ok(()),
            event = rx.recv() => event,
        };
        match event {
            Ok(event) => notify(event, &mut out),
            Err(RecvError::Lagged(missed)) => {
                return Err(format!(
                    "subscription too slow: {missed} events were dropped; resubscribe"
                ))
            }
            Err(RecvError::Closed) => return Ok(()),
        }
        for json in out.drain(..) {
            match sink
                .send_timeout(SubscriptionMessage::from(json), SEND_TIMEOUT)
                .await
            {
                Ok(()) => {}
                Err(SendTimeoutError::Closed(_)) => return Ok(()),
                Err(SendTimeoutError::Timeout(_)) => {
                    return Err(format!(
                        "subscription too slow: a notification was not accepted within {}s",
                        SEND_TIMEOUT.as_secs()
                    ))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Subscription ids
// ---------------------------------------------------------------------------

/// geth-style subscription ids: `0x` + 32 hex digits (16 bytes), unpredictable across
/// processes (seeded from the process' random hasher keys), unique within one.
#[derive(Debug)]
pub(crate) struct HexIdProvider {
    seed: [u8; 16],
    counter: AtomicU64,
}

impl Default for HexIdProvider {
    fn default() -> Self {
        use std::hash::{BuildHasher, Hasher};
        let mut seed = [0u8; 16];
        for chunk in seed.chunks_mut(8) {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            chunk.copy_from_slice(&h.finish().to_be_bytes());
        }
        Self {
            seed,
            counter: AtomicU64::new(0),
        }
    }
}

impl IdProvider for HexIdProvider {
    fn next_id(&self) -> SubscriptionId<'static> {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut input = [0u8; 24];
        input[..16].copy_from_slice(&self.seed);
        input[16..].copy_from_slice(&n.to_be_bytes());
        let digest = keccak256(input);
        SubscriptionId::Str(xdevs_primitives::hex::encode_prefixed(&digest[..16]).into())
    }
}

// ---------------------------------------------------------------------------
// HTTP vs WebSocket
// ---------------------------------------------------------------------------

/// Request extension set by [`TransportLayer`] on WebSocket upgrade requests (jsonrpsee
/// copies the upgrade request's extensions into every call of the connection).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WsConnection;

/// HTTP middleware marking WebSocket upgrade requests with [`WsConnection`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TransportLayer;

impl<S> tower::Layer<S> for TransportLayer {
    type Service = Transport<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Transport { inner }
    }
}

/// Service of [`TransportLayer`].
#[derive(Clone, Debug)]
pub(crate) struct Transport<S> {
    inner: S,
}

impl<S, B> tower::Service<Request<B>> for Transport<S>
where
    S: tower::Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let is_ws = req
            .headers()
            .get(UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        if is_ws {
            req.extensions_mut().insert(WsConnection);
        } else {
            req.extensions_mut().remove::<WsConnection>();
        }
        self.inner.call(req)
    }
}

/// RPC middleware answering subscription calls made over plain HTTP with -32601
/// "notifications not supported".
#[derive(Clone, Debug)]
pub(crate) struct NoHttpSubscriptions<S> {
    inner: S,
}

impl<S> NoHttpSubscriptions<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self { inner }
    }
}

/// The error of a subscription call over HTTP.
pub(crate) fn notifications_unsupported() -> ErrorObjectOwned {
    ErrorObjectOwned::owned::<()>(-32601, "notifications not supported", None)
}

impl<S> RpcServiceT for NoHttpSubscriptions<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
{
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;
    type BatchResponse = S::BatchResponse;

    fn call<'a>(
        &self,
        request: jsonrpsee::types::Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let reject = matches!(request.method_name(), SUBSCRIBE | UNSUBSCRIBE)
            && request.extensions().get::<WsConnection>().is_none();
        let inner = self.inner.clone();
        async move {
            if reject {
                MethodResponse::error(request.id, notifications_unsupported())
            } else {
                inner.call(request).await
            }
        }
    }

    fn batch<'a>(
        &self,
        requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        // Subscription calls inside an HTTP batch get jsonrpsee's generic error.
        self.inner.batch(requests)
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(n)
    }
}

/// Global subscription cap of a server.
pub(crate) fn subscription_slots(config: &RpcServerConfig) -> Arc<tokio::sync::Semaphore> {
    Arc::new(tokio::sync::Semaphore::new(
        config.max_subscriptions as usize,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xdevs_primitives::Address;

    fn parse(v: serde_json::Value) -> Result<SubscriptionKind, ErrorObjectOwned> {
        let s = v.to_string();
        parse_subscription(&Params::new(Some(&s)))
    }

    #[test]
    fn parses_kinds() {
        assert_eq!(
            parse(json!(["newHeads"])).unwrap(),
            SubscriptionKind::NewHeads
        );
        assert_eq!(
            parse(json!(["newPendingTransactions"])).unwrap(),
            SubscriptionKind::NewPendingTransactions
        );
        assert_eq!(
            parse(json!(["newPendingTransactions", false])).unwrap(),
            SubscriptionKind::NewPendingTransactions
        );
        assert_eq!(
            parse(json!(["newPendingTransactions", true]))
                .unwrap_err()
                .code(),
            -32602
        );
        assert_eq!(
            parse(json!(["logs"])).unwrap(),
            SubscriptionKind::Logs(Box::default())
        );
        let a = Address::repeat_byte(1);
        let t = B256::repeat_byte(2);
        let SubscriptionKind::Logs(f) =
            parse(json!(["logs", {"address": a, "topics": [t, null, [t]]}])).unwrap()
        else {
            panic!("not logs")
        };
        assert!(f.address.matches(&a));
        assert!(f.topics[0].matches(&t));
        assert!(f.topics[1].is_empty());
        // `latest` tags are fine, historical ranges and block hashes are not.
        assert!(parse(json!(["logs", {"fromBlock": "latest"}])).is_ok());
        assert_eq!(
            parse(json!(["logs", {"fromBlock": "0x1"}]))
                .unwrap_err()
                .code(),
            -32602
        );
        assert_eq!(
            parse(json!(["logs", {"blockHash": t}])).unwrap_err().code(),
            -32602
        );
        assert_eq!(parse(json!(["syncing"])).unwrap_err().code(), -32602);
        assert_eq!(parse(json!([])).unwrap_err().code(), -32602);
    }

    #[test]
    fn filter_size_limits() {
        let addrs: Vec<Address> = (0..=MAX_FILTER_ADDRESSES)
            .map(|i| Address::left_padding_from(&(i as u64).to_be_bytes()))
            .collect();
        let err = parse(json!(["logs", {"address": addrs}])).unwrap_err();
        assert_eq!(err.code(), -32005);
        assert!(
            err.message().contains("too many addresses"),
            "{}",
            err.message()
        );
        assert!(parse(json!(["logs", {"address": &addrs[..MAX_FILTER_ADDRESSES]}])).is_ok());

        let topics: Vec<B256> = (0..MAX_FILTER_TOPICS / 2 + 1)
            .map(|i| B256::left_padding_from(&(i as u64).to_be_bytes()))
            .collect();
        let err = parse(json!(["logs", {"topics": [topics, topics]}])).unwrap_err();
        assert_eq!(err.code(), -32005);
        assert!(
            err.message().contains("too many topics"),
            "{}",
            err.message()
        );
        assert!(parse(json!(["logs", {"topics": [topics]}])).is_ok());
    }

    #[test]
    fn hex_ids_are_unique_and_geth_shaped() {
        let p = HexIdProvider::default();
        let a = p.next_id();
        let b = p.next_id();
        assert_ne!(a, b);
        let SubscriptionId::Str(s) = a else {
            panic!("string id")
        };
        assert!(s.starts_with("0x") && s.len() == 34, "{s}");
        assert!(s[2..].chars().all(|c| c.is_ascii_hexdigit()));
    }
}

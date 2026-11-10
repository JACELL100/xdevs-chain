//! JSON-RPC server for xDEVS Chain (plan §19). **Owner: rpc agent.**
//!
//! The server (jsonrpsee, HTTP on `127.0.0.1:8545` by default) exposes the `eth_`,
//! `net_` and `web3_` methods of §19.1 and forwards every call to an [`RpcBackend`],
//! which the node implements on top of storage, state, execution and the mempool.
//! RPC shapes are the `alloy-rpc-types-eth` types re-exported in [`types`].
//!
//! WebSocket clients (same port) can `eth_subscribe` to `newHeads`, `logs` and
//! `newPendingTransactions` (see [`subscription`]); the node publishes the events through
//! [`events::ChainEvents`] and hands out receivers via [`RpcBackend::subscribe_blocks`] /
//! [`RpcBackend::subscribe_pending_txs`].
//!
//! Finality tags: `latest`, `safe` and `finalized` all mean the latest committed
//! block (BFT finality); `pending` = latest state + pool view (nonce only in Phase 1);
//! `earliest` = block 0.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod convert;
mod cors;
mod error;
pub mod events;
mod limits;
mod server;
pub mod subscription;

pub use error::RpcBackendError;
pub use events::{BlockEvent, ChainEvents};
pub use server::build_module;

use async_trait::async_trait;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::broadcast;
use xdevs_primitives::{Address, Bytes, B256, U256};

/// RPC types used by [`RpcBackend`] (re-exported from `alloy-rpc-types-eth`, plus the
/// xDEVS-specific [`EoaLockInfo`]).
pub mod types {
    pub use alloy_rpc_types_eth::{
        Block as RpcBlock, BlockId, BlockNumberOrTag, BlockTransactions, BlockTransactionsKind,
        FeeHistory, Filter, FilterBlockOption, Header as RpcHeader, Log as RpcLog, SyncInfo,
        SyncStatus, Transaction as RpcTransaction, TransactionReceipt as RpcReceipt,
        TransactionRequest,
    };
    pub use xdevs_primitives::EoaLockStatus;

    /// Result of `xdevs_getEoaLock(address, blockTag)` (XIP-0001 §5 f):
    /// `{ "active": true, "status": "locked", "word": "0x…01", "delegatedTo": "0x…1005" }`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct EoaLockInfo {
        /// XIP-0001 is enforced for the block after the requested one (the block a new
        /// transaction would go into).
        pub active: bool,
        /// `"unlocked"`, `"locked"` or `"sealed"` at the requested block (the protocol
        /// classification, reported even when `active` is false).
        pub status: EoaLockStatus,
        /// Raw lock word `storage[address][LOCK_SLOT]` (32 bytes).
        pub word: xdevs_primitives::B256,
        /// Target of the account's EIP-7702 delegation (`0x…1005` for an xDEVS Smart
        /// Account), `null` if the account is not delegated.
        pub delegated_to: Option<xdevs_primitives::Address>,
    }

    /// Result of `xdevs_getCommitCertificate(block)` (Phase 2, research doc §6.5): the
    /// precommit signatures that finalized the block, plus the canonical RLP (`raw`) a
    /// client can verify against the validator set.
    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CommitCertificateInfo {
        /// Block number.
        pub height: xdevs_primitives::reexports::alloy_primitives::U64,
        /// Consensus round of the decision.
        pub round: xdevs_primitives::reexports::alloy_primitives::U64,
        /// Block hash the validators precommitted.
        pub block_hash: xdevs_primitives::B256,
        /// Signatures, sorted by validator address.
        pub signatures: Vec<CommitSigInfo>,
        /// Canonical RLP of the certificate (as stored and served by block sync).
        pub raw: xdevs_primitives::Bytes,
    }

    /// One signature of a [`CommitCertificateInfo`].
    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CommitSigInfo {
        /// Validator address (20 bytes, `sha256(ed25519 pubkey)[..20]`).
        pub validator: xdevs_primitives::Bytes,
        /// Ed25519 signature (64 bytes).
        pub signature: xdevs_primitives::Bytes,
    }
}
use types::*;

/// Result type of backend calls.
pub type BackendResult<T> = Result<T, RpcBackendError>;

/// Everything the Phase-1 RPC server needs from the node (plan §19.1).
///
/// Block arguments use [`BlockId`] (number, tag or hash). Unknown blocks yield
/// `Ok(None)` for block/tx lookups and [`RpcBackendError::UnknownBlock`] for state queries.
#[async_trait]
pub trait RpcBackend: Send + Sync + 'static {
    /// `eth_chainId` / `net_version`.
    fn chain_id(&self) -> u64;

    /// `web3_clientVersion`.
    fn client_version(&self) -> String {
        xdevs_primitives::constants::CLIENT_VERSION.to_string()
    }

    /// `eth_blockNumber`: latest committed block.
    async fn block_number(&self) -> BackendResult<u64>;

    /// `eth_syncing` (single node: always `SyncStatus::None`).
    async fn syncing(&self) -> BackendResult<SyncStatus> {
        Ok(SyncStatus::None)
    }

    /// `eth_getBalance`.
    async fn balance(&self, address: Address, block: BlockId) -> BackendResult<U256>;

    /// `eth_getTransactionCount` (`pending` includes pool txs).
    async fn transaction_count(&self, address: Address, block: BlockId) -> BackendResult<u64>;

    /// `eth_getCode`.
    async fn code(&self, address: Address, block: BlockId) -> BackendResult<Bytes>;

    /// `eth_getStorageAt` (slot as 32-byte key; value returned as 32 bytes).
    async fn storage_at(&self, address: Address, slot: B256, block: BlockId)
        -> BackendResult<B256>;

    /// `eth_call`: return data, or [`RpcBackendError::Reverted`].
    async fn call(&self, request: TransactionRequest, block: BlockId) -> BackendResult<Bytes>;

    /// `eth_estimateGas`.
    async fn estimate_gas(&self, request: TransactionRequest, block: BlockId)
        -> BackendResult<u64>;

    /// `eth_sendRawTransaction`: decode, recover, add to pool; returns tx hash.
    async fn send_raw_transaction(&self, raw: Bytes) -> BackendResult<B256>;

    /// `eth_getBlockByNumber` (`full` = include full tx objects instead of hashes).
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> BackendResult<Option<RpcBlock>>;

    /// `eth_getBlockByHash`.
    async fn block_by_hash(&self, hash: B256, full: bool) -> BackendResult<Option<RpcBlock>>;

    /// `eth_getBlockTransactionCountByNumber` / `...ByHash`.
    async fn block_transaction_count(&self, block: BlockId) -> BackendResult<Option<u64>>;

    /// `eth_getTransactionByHash` (mined or pending).
    async fn transaction_by_hash(&self, hash: B256) -> BackendResult<Option<RpcTransaction>>;

    /// `eth_getTransactionReceipt`.
    async fn transaction_receipt(&self, hash: B256) -> BackendResult<Option<RpcReceipt>>;

    /// `eth_getBlockReceipts`.
    async fn block_receipts(&self, block: BlockId) -> BackendResult<Option<Vec<RpcReceipt>>>;

    /// `eth_getLogs`. The server resolves block tags to numbers and enforces
    /// [`RpcServerConfig::max_logs_range`] before calling this, so the backend receives
    /// either `AtBlockHash` or a numeric `Range { from <= to <= head }`.
    ///
    /// `limit` is [`RpcServerConfig::max_logs_results`] (`None` = unlimited). The
    /// backend must stop collecting and return [`RpcBackendError::LimitExceeded`] as
    /// soon as more than `limit` logs match (use [`check_logs_limit`]), so memory use
    /// is bounded by the result cap rather than by the block range. The server also
    /// checks the returned length as a backstop.
    async fn logs(&self, filter: Filter, limit: Option<usize>) -> BackendResult<Vec<RpcLog>>;

    /// `eth_gasPrice`: next base fee + suggested tip.
    async fn gas_price(&self) -> BackendResult<U256>;

    /// `eth_maxPriorityFeePerGas`: suggested tip.
    async fn max_priority_fee_per_gas(&self) -> BackendResult<U256>;

    /// Base fee of the next block (used for `gasPrice` / `feeHistory`).
    async fn next_base_fee(&self) -> BackendResult<u64>;

    /// `eth_feeHistory`.
    async fn fee_history(
        &self,
        block_count: u64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> BackendResult<FeeHistory>;

    /// `xdevs_getEoaLock(address, blockTag)`: XIP-0001 lock state of `address` at `block`.
    /// The default reports the method as unsupported.
    async fn eoa_lock(&self, _address: Address, _block: BlockId) -> BackendResult<EoaLockInfo> {
        Err(RpcBackendError::Unsupported("xdevs_getEoaLock".into()))
    }

    /// `net_peerCount`: connected p2p peers (Phase 2). The default reports 0.
    async fn peer_count(&self) -> BackendResult<u64> {
        Ok(0)
    }

    /// `xdevs_getCommitCertificate(block)`: the commit certificate of `block` (`None` for
    /// an unknown block or one without a certificate, e.g. genesis or dev-mode blocks).
    /// The default reports the method as unsupported.
    async fn commit_certificate(
        &self,
        _block: BlockId,
    ) -> BackendResult<Option<CommitCertificateInfo>> {
        Err(RpcBackendError::Unsupported(
            "xdevs_getCommitCertificate".into(),
        ))
    }

    /// `eth_subscribe("newHeads" | "logs")`: a receiver of every block committed from
    /// now on, in commit order (see [`events::ChainEvents::subscribe_blocks`]). The
    /// default (`None`) answers these subscriptions with -32601.
    fn subscribe_blocks(&self) -> Option<broadcast::Receiver<Arc<BlockEvent>>> {
        None
    }

    /// `eth_subscribe("newPendingTransactions")`: a receiver of the hash of every
    /// transaction admitted to the mempool from now on. The default (`None`) answers
    /// the subscription with -32601.
    fn subscribe_pending_txs(&self) -> Option<broadcast::Receiver<B256>> {
        None
    }
}

/// `eth_getLogs` result cap check: `Err(LimitExceeded)` once `count` matching logs
/// exceed `limit` (`None` = unlimited). Shared by the server and backends.
pub fn check_logs_limit(count: usize, limit: Option<usize>) -> BackendResult<()> {
    match limit {
        Some(max) if count > max => Err(RpcBackendError::LimitExceeded(format!(
            "query returned more than {max} results"
        ))),
        _ => Ok(()),
    }
}

/// Default of [`RpcServerConfig::max_in_flight_per_connection`].
pub const DEFAULT_MAX_IN_FLIGHT_PER_CONNECTION: u32 = 32;

/// Server settings (plan §19.3, §33.2 `[rpc]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcServerConfig {
    /// HTTP listen address (default `127.0.0.1:8545`).
    pub http: SocketAddr,
    /// Gas cap for `eth_call` / `eth_estimateGas` (default 50M).
    pub gas_cap: u64,
    /// Max block range for `eth_getLogs` (default 10,000).
    pub max_logs_range: u64,
    /// Max results for `eth_getLogs` (default 10,000).
    pub max_logs_results: usize,
    /// Max JSON-RPC batch size (default 50).
    pub max_batch: u32,
    /// Allowed CORS origins (`["*"]` for local dev / MetaMask, empty = no CORS headers).
    pub cors: Vec<String>,
    /// Max HTTP request body / WS message size in bytes (default 5 MiB, like geth).
    pub max_request_body_size: u32,
    /// Max response size in bytes (default 32 MiB).
    pub max_response_body_size: u32,
    /// Max concurrent connections (default 256).
    pub max_connections: u32,
    /// Max active `eth_subscribe` subscriptions per WebSocket connection (default 20;
    /// error -32006 above).
    pub max_subscriptions_per_connection: u32,
    /// Max active subscriptions over all connections of the server (default 1,000;
    /// error -32005 above).
    pub max_subscriptions: u32,
    /// Max calls running at once per connection (default 32; 0 = unlimited). Calls
    /// above it fail at once with -32005: one WebSocket client cannot run thousands of
    /// heavy calls in parallel.
    pub max_in_flight_per_connection: u32,
    /// Per-IP token bucket: calls per second per client IP (plan §19.3; 0 = off, the
    /// default). Calls over the budget fail with -32005. Behind a reverse proxy every
    /// client shares the proxy's IP, so rate-limit at the proxy instead.
    pub rate_limit_per_ip: u32,
    /// Bucket size (burst) of the per-IP rate limit (default 2 × `rate_limit_per_ip`).
    pub rate_limit_burst: u32,
}

impl Default for RpcServerConfig {
    fn default() -> Self {
        Self {
            http: SocketAddr::from(([127, 0, 0, 1], 8545)),
            gas_cap: 50_000_000,
            max_logs_range: 10_000,
            max_logs_results: 10_000,
            max_batch: 50,
            cors: vec!["*".to_string()],
            max_request_body_size: 5 * 1024 * 1024,
            max_response_body_size: 32 * 1024 * 1024,
            max_connections: 256,
            max_subscriptions_per_connection: 20,
            max_subscriptions: 1_000,
            max_in_flight_per_connection: DEFAULT_MAX_IN_FLIGHT_PER_CONNECTION,
            rate_limit_per_ip: 0,
            rate_limit_burst: 0,
        }
    }
}

/// Handle of a running server; dropping it does not stop the server, call
/// [`RpcServerHandle::stop`].
#[derive(Debug, Clone)]
pub struct RpcServerHandle {
    /// Bound address (useful with port 0 in tests).
    pub local_addr: SocketAddr,
    inner: jsonrpsee::server::ServerHandle,
}

impl RpcServerHandle {
    /// Wrap a jsonrpsee handle.
    pub fn new(local_addr: SocketAddr, inner: jsonrpsee::server::ServerHandle) -> Self {
        Self { local_addr, inner }
    }

    /// Stop the server.
    pub fn stop(&self) {
        let _ = self.inner.stop();
    }

    /// Wait until the server has stopped.
    pub async fn stopped(self) {
        self.inner.stopped().await
    }
}

/// Start the JSON-RPC server (HTTP and WebSocket on the same port) serving
/// `eth_` / `net_` / `web3_` from `backend`, with the §19.3 limits from `config`
/// (plus the per-connection in-flight cap and the per-IP rate limit).
pub async fn start_server(
    config: RpcServerConfig,
    backend: Arc<dyn RpcBackend>,
) -> anyhow::Result<RpcServerHandle> {
    use jsonrpsee::{
        core::middleware::RpcServiceBuilder,
        server::{
            serve_with_graceful_shutdown, stop_channel, BatchRequestConfig, Server, ServerConfig,
        },
    };

    let server_config = ServerConfig::builder()
        .max_request_body_size(config.max_request_body_size)
        .max_response_body_size(config.max_response_body_size)
        .max_connections(config.max_connections)
        .max_subscriptions_per_connection(config.max_subscriptions_per_connection)
        .set_id_provider(subscription::HexIdProvider::default())
        .set_batch_request_config(BatchRequestConfig::Limit(config.max_batch))
        .build();
    let cors = cors::CorsLayer::new(cors::CorsPolicy::from_config(&config.cors));
    let http_middleware = tower::ServiceBuilder::new()
        .layer(cors)
        .layer(subscription::TransportLayer);
    let limits = limits::LimitsConfig {
        max_in_flight: config.max_in_flight_per_connection,
        rate: (config.rate_limit_per_ip > 0).then(|| {
            let burst = match config.rate_limit_burst {
                0 => config.rate_limit_per_ip.saturating_mul(2),
                b => b,
            };
            Arc::new(limits::IpRateLimiter::new(config.rate_limit_per_ip, burst))
        }),
    };
    // Instantiated once per WebSocket connection (or HTTP request): the in-flight cap
    // is per connection.
    let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |service| {
        limits::RequestLimits::new(subscription::NoHttpSubscriptions::new(service), &limits)
    });

    let listener = tokio::net::TcpListener::bind(config.http)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind JSON-RPC server on {}: {e}", config.http))?;
    let local_addr = listener.local_addr()?;
    let builder = Server::builder()
        .set_config(server_config)
        .set_http_middleware(http_middleware)
        .set_rpc_middleware(rpc_middleware)
        .to_service_builder();
    let methods: jsonrpsee::Methods = build_module(config, backend)?.into();
    let (stop, handle) = stop_channel();

    // Our own accept loop (instead of `Server::start`) so every request carries the
    // client IP for the per-IP rate limit.
    tokio::spawn(async move {
        loop {
            let (socket, remote) = tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok(conn) => conn,
                    Err(e) => {
                        // E.g. out of file descriptors: back off instead of spinning.
                        tracing::debug!(target: "xdevs::rpc", "accept failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    }
                },
                _ = stop.clone().shutdown() => break,
            };
            let _ = socket.set_nodelay(true);
            let ip = limits::RemoteIp(remote.ip());
            let (builder, methods, conn_stop) = (builder.clone(), methods.clone(), stop.clone());
            let service = tower::service_fn(move |mut req: http::Request<_>| {
                req.extensions_mut().insert(ip);
                let mut svc = builder.clone().build(methods.clone(), conn_stop.clone());
                async move { tower::Service::call(&mut svc, req).await }
            });
            tokio::spawn(serve_with_graceful_shutdown(
                socket,
                service,
                stop.clone().shutdown(),
            ));
        }
    });
    tracing::info!(target: "xdevs::rpc", %local_addr, "JSON-RPC server started (HTTP + WS)");
    Ok(RpcServerHandle::new(local_addr, handle))
}

/// Alias of [`start_server`].
pub async fn start_rpc_server(
    config: RpcServerConfig,
    backend: Arc<dyn RpcBackend>,
) -> anyhow::Result<RpcServerHandle> {
    start_server(config, backend).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_limit_check() {
        assert!(check_logs_limit(3, Some(3)).is_ok());
        assert!(check_logs_limit(usize::MAX, None).is_ok());
        let err = check_logs_limit(4, Some(3)).unwrap_err();
        assert_eq!(err.code(), -32005);
        assert!(err.to_string().contains("more than 3 results"), "{err}");
    }
}

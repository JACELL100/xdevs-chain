//! JSON-RPC server for xDEVS Chain (plan §19). **Owner: rpc agent.**
//!
//! The server (jsonrpsee, HTTP on `127.0.0.1:8545` by default) exposes the `eth_`,
//! `net_` and `web3_` methods of §19.1 and forwards every call to an [`RpcBackend`],
//! which the node implements on top of storage, state, execution and the mempool.
//! RPC shapes are the `alloy-rpc-types-eth` types re-exported in [`types`].
//!
//! Finality tags: `latest`, `safe` and `finalized` all mean the latest committed
//! block (BFT finality); `pending` = latest state + pool view (nonce only in Phase 1);
//! `earliest` = block 0.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod convert;
mod cors;
mod error;
mod server;

pub use error::RpcBackendError;
pub use server::build_module;

use async_trait::async_trait;
use std::{net::SocketAddr, sync::Arc};
use xdevs_primitives::{Address, Bytes, B256, U256};

/// RPC types used by [`RpcBackend`] (re-exported from `alloy-rpc-types-eth`).
pub mod types {
    pub use alloy_rpc_types_eth::{
        Block as RpcBlock, BlockId, BlockNumberOrTag, BlockTransactions, BlockTransactionsKind,
        FeeHistory, Filter, FilterBlockOption, Header as RpcHeader, Log as RpcLog, SyncInfo,
        SyncStatus, Transaction as RpcTransaction, TransactionReceipt as RpcReceipt,
        TransactionRequest,
    };
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

    /// `eth_getLogs`. The server resolves block tags to numbers, enforces
    /// [`RpcServerConfig::max_logs_range`] before calling this and
    /// [`RpcServerConfig::max_logs_results`] on the result, so the backend receives
    /// either `AtBlockHash` or a numeric `Range { from <= to <= head }`.
    async fn logs(&self, filter: Filter) -> BackendResult<Vec<RpcLog>>;

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
}

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
/// `eth_` / `net_` / `web3_` from `backend`, with the §19.3 limits from `config`.
pub async fn start_server(
    config: RpcServerConfig,
    backend: Arc<dyn RpcBackend>,
) -> anyhow::Result<RpcServerHandle> {
    use jsonrpsee::server::{BatchRequestConfig, Server, ServerConfig};

    let server_config = ServerConfig::builder()
        .max_request_body_size(config.max_request_body_size)
        .max_response_body_size(config.max_response_body_size)
        .max_connections(config.max_connections)
        .set_batch_request_config(BatchRequestConfig::Limit(config.max_batch))
        .build();
    let cors = cors::CorsLayer::new(cors::CorsPolicy::from_config(&config.cors));
    let http_middleware = tower::ServiceBuilder::new().layer(cors);

    let server = Server::builder()
        .set_config(server_config)
        .set_http_middleware(http_middleware)
        .build(config.http)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind JSON-RPC server on {}: {e}", config.http))?;
    let local_addr = server.local_addr()?;
    let module = build_module(config, backend)?;
    let handle = server.start(module);
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

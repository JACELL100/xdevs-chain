//! Node wiring (plan §5, §33.2). **Owner: node agent.**
//!
//! Phase 1: a single-validator *dev mode* node — load genesis, open storage, run the
//! block producer every `block_time_ms`, serve JSON-RPC.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let node = xdevs_node::Node::new(xdevs_node::NodeConfig::default())?;
//! node.run().await // until Ctrl-C
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod backend;
pub mod chain;
mod config;
mod producer;

pub use backend::{decode_revert_reason, NodeBackend, DEFAULT_PRIORITY_FEE};
pub use config::{ChainSection, DevSection, LogSection, MempoolSection, NodeConfig, RpcSection};
pub use producer::{unix_now, BlockProducer, ProducedBlock};

use anyhow::Context;
use chain::Db;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use xdevs_execution::{next_base_fee, EvmExecutor};
use xdevs_genesis::Genesis;
use xdevs_mempool::{FromState, Pool, PoolConfig, PoolParams};
use xdevs_primitives::{constants::DEV_CHAIN_ID, Bytes, ChainConfig, B256};
use xdevs_rpc::{RpcServerConfig, RpcServerHandle};
use xdevs_state::DbStateProvider;
use xdevs_storage::RedbDatabase;

/// Per-transaction gas cap of the Osaka rules the executor runs (EIP-7825).
pub const MAX_TX_GAS: u64 = 1 << 24;
/// `extraData` of dev-mode blocks.
pub const DEV_BLOCK_EXTRA_DATA: &[u8] = b"xdevs";

/// A node with its database open and genesis initialised, ready to [`Node::start`].
pub struct Node {
    config: NodeConfig,
    genesis_hash: B256,
    cfg: ChainConfig,
    db: Db,
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("config", &self.config)
            .field("genesis_hash", &self.genesis_hash)
            .finish_non_exhaustive()
    }
}

/// Load the configured genesis (`None` = built-in dev genesis, chain id 7117).
pub fn load_genesis(config: &NodeConfig) -> anyhow::Result<Genesis> {
    match &config.chain.genesis {
        Some(path) => Genesis::from_file(path)
            .with_context(|| format!("failed to load genesis {}", path.display())),
        None => Ok(Genesis::dev(DEV_CHAIN_ID)),
    }
}

impl Node {
    /// Prepare a node: load genesis, open the database, initialise genesis state if the
    /// database is empty (and verify the stored genesis hash and head state root
    /// otherwise).
    pub fn new(config: NodeConfig) -> anyhow::Result<Self> {
        config.chain.state_history_blocks()?;
        let genesis = load_genesis(&config)?;
        let data_dir = &config.chain.data_dir;
        let db = RedbDatabase::open_dir(data_dir)
            .with_context(|| format!("failed to open database in {}", data_dir.display()))?;
        let db: Db = Arc::new(db);
        let genesis_hash = chain::init_genesis(&*db, &genesis)?;
        let (head_number, head) = chain::verify_head(&*db)?;
        tracing::info!(
            target: "xdevs::node",
            chain_id = genesis.config.chain_id,
            %genesis_hash,
            head = head_number,
            state_root = %head.state_root,
            data_dir = %data_dir.display(),
            "database ready"
        );

        let cfg = genesis.chain_config();
        let executor = Arc::new(EvmExecutor::new(cfg.clone()));
        let mut params = PoolParams::from_chain_config(&cfg);
        params.max_tx_gas = Some(MAX_TX_GAS);
        let pool_config = PoolConfig {
            max_txs: config.mempool.max_txs,
            max_bytes: config.mempool.max_bytes,
            max_per_sender: config.mempool.max_per_sender,
            ..PoolConfig::default()
        };
        // Latest committed state: accounts plus the XIP-0001 lock slot.
        let reader = FromState(DbStateProvider::new(db.clone()));
        let pool = Arc::new(Pool::new(
            params,
            pool_config,
            Arc::new(reader),
            next_base_fee(&head, &cfg),
        ));
        pool.set_head_number(head_number);
        Ok(Self {
            config,
            genesis_hash,
            cfg,
            db,
            pool,
            executor,
        })
    }

    /// Node configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Effective chain configuration (from genesis).
    pub fn chain_config(&self) -> &ChainConfig {
        &self.cfg
    }

    /// Hash of the genesis block.
    pub fn genesis_hash(&self) -> B256 {
        self.genesis_hash
    }

    /// Block interval of the dev producer.
    pub fn block_time(&self) -> Duration {
        Duration::from_millis(
            self.config
                .dev
                .block_time_ms
                .unwrap_or(self.cfg.block_time_ms)
                .max(1),
        )
    }

    /// Start the RPC server(s) and, in dev mode, the block producer.
    pub async fn start(self) -> anyhow::Result<NodeHandle> {
        let backend = Arc::new(NodeBackend::new(
            self.db.clone(),
            self.pool.clone(),
            self.executor.clone(),
            self.cfg.clone(),
            self.config.rpc.gas_cap,
        ));
        let rpc_config = RpcServerConfig {
            http: self.config.rpc.http,
            gas_cap: self.config.rpc.gas_cap,
            max_logs_range: self.config.rpc.max_logs_range,
            max_batch: self.config.rpc.max_batch,
            cors: self.config.rpc.cors.clone(),
            ..Default::default()
        };
        let mut rpc = vec![xdevs_rpc::start_server(rpc_config.clone(), backend.clone()).await?];
        if let Some(ws) = self.config.rpc.ws.filter(|ws| *ws != rpc[0].local_addr) {
            let ws_config = RpcServerConfig {
                http: ws,
                ..rpc_config
            };
            rpc.push(xdevs_rpc::start_server(ws_config, backend).await?);
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let producer = if self.config.dev.enabled {
            let producer = Arc::new(
                BlockProducer::new(
                    self.db.clone(),
                    self.pool.clone(),
                    self.executor.clone(),
                    self.cfg.clone(),
                    self.config.dev.fee_recipient,
                    Bytes::from_static(DEV_BLOCK_EXTRA_DATA),
                )
                .with_state_history(self.config.chain.state_history_blocks()?),
            );
            let block_time = self.block_time();
            tracing::info!(
                target: "xdevs::node",
                block_time_ms = block_time.as_millis() as u64,
                fee_recipient = %self.config.dev.fee_recipient,
                "dev block producer started"
            );
            Some(tokio::spawn(produce_loop(
                producer,
                block_time,
                shutdown_rx,
            )))
        } else {
            tracing::warn!(
                target: "xdevs::node",
                "dev mode disabled and consensus is not implemented yet (Phase 2): \
                 no blocks will be produced"
            );
            None
        };

        Ok(NodeHandle {
            http_addr: rpc[0].local_addr,
            rpc,
            producer,
            shutdown_tx,
            db: Some(self.db),
            genesis_hash: self.genesis_hash,
        })
    }

    /// Run until Ctrl-C: block producer + RPC server, then shut down gracefully.
    pub async fn run(self) -> anyhow::Result<()> {
        let handle = self.start().await?;
        tokio::signal::ctrl_c()
            .await
            .context("failed to listen for Ctrl-C")?;
        tracing::info!(target: "xdevs::node", "Ctrl-C received, shutting down");
        handle.shutdown().await;
        tracing::info!(target: "xdevs::node", "shutdown complete");
        Ok(())
    }
}

/// Produce a block every `block_time` until shutdown is signalled.
async fn produce_loop(
    producer: Arc<BlockProducer>,
    block_time: Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval =
        tokio::time::interval_at(tokio::time::Instant::now() + block_time, block_time);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => break,
        }
        let p = producer.clone();
        // A started block always finishes (and commits) before shutdown completes.
        match tokio::task::spawn_blocking(move || p.produce(unix_now())).await {
            Ok(Ok(b)) if b.tx_hashes.is_empty() => tracing::debug!(
                target: "xdevs::producer",
                number = b.number, hash = %b.hash, base_fee = b.base_fee, "sealed empty block"
            ),
            Ok(Ok(b)) => tracing::info!(
                target: "xdevs::producer",
                number = b.number,
                hash = %b.hash,
                txs = b.tx_hashes.len(),
                gas_used = b.gas_used,
                base_fee = b.base_fee,
                "sealed block"
            ),
            Ok(Err(e)) => {
                tracing::error!(target: "xdevs::producer", error = %e, "block production failed")
            }
            Err(e) => {
                tracing::error!(target: "xdevs::producer", error = %e, "block production task panicked")
            }
        }
        if *shutdown.borrow() {
            break;
        }
    }
}

/// A running node.
pub struct NodeHandle {
    http_addr: SocketAddr,
    rpc: Vec<RpcServerHandle>,
    producer: Option<JoinHandle<()>>,
    shutdown_tx: watch::Sender<bool>,
    db: Option<Db>,
    genesis_hash: B256,
}

impl std::fmt::Debug for NodeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle")
            .field("http_addr", &self.http_addr)
            .field("genesis_hash", &self.genesis_hash)
            .finish_non_exhaustive()
    }
}

impl NodeHandle {
    /// Bound JSON-RPC address (HTTP + WS).
    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// `http://<addr>` URL of the RPC server.
    pub fn http_url(&self) -> String {
        format!("http://{}", self.http_addr)
    }

    /// Hash of the genesis block.
    pub fn genesis_hash(&self) -> B256 {
        self.genesis_hash
    }

    /// Stop block production and the RPC servers, then wait (up to 10 s) until every
    /// database handle is released so the data dir can be reopened.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(p) = self.producer.take() {
            if let Err(e) = p.await {
                tracing::error!(target: "xdevs::node", error = %e, "producer task failed");
            }
        }
        for server in std::mem::take(&mut self.rpc) {
            server.stop();
            server.stopped().await;
        }
        if let Some(db) = self.db.take() {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while Arc::strong_count(&db) > 1 && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            if Arc::strong_count(&db) > 1 {
                tracing::warn!(
                    target: "xdevs::node",
                    "database still referenced after shutdown (in-flight RPC requests?)"
                );
            }
        }
    }
}

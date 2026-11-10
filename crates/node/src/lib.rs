//! Node wiring (plan §5, §33.2). **Owner: node agent.**
//!
//! Run modes ([`NodeMode`], chosen from the genesis and the config):
//!
//! * **dev** (Phase 1): single node, a local producer seals a block every
//!   `block_time_ms`.
//! * **validator** (Phase 2): [`ChainApp`] driven by the Malachite BFT engine with the
//!   node's Ed25519 consensus key; tx gossip over `xdevs-network`.
//! * **full node** (Phase 2): the same engine without a key: follows the chain through
//!   consensus messages and block sync, never signs.
//! * **RPC only**: serves the stored chain.
//!
//! If the consensus engine stops with an error (for example a SignGuard refusal) the
//! node halts: [`Node::run`] returns the error; nothing restarts it.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let node = xdevs_node::Node::new(xdevs_node::NodeConfig::default())?;
//! node.run().await // until Ctrl-C
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod backend;
pub mod chain;
mod config;
pub mod consensus;
mod events;
mod producer;

pub use app::{ChainApp, ChainAppConfig, MAX_BLOCK_BYTES, PROPOSAL_EXTRA_DATA};
pub use backend::{decode_revert_reason, NodeBackend, DEFAULT_PRIORITY_FEE, SYNC_LAG_THRESHOLD};
pub use config::{
    ChainSection, DevSection, GossipSection, LogSection, MempoolSection, NodeConfig, NodeMode,
    RpcSection, ValidatorSection, DEFAULT_MAX_TIMESTAMP_DRIFT_SECS,
};
pub use producer::{unix_now, BlockProducer, ProducedBlock};

use anyhow::{anyhow, Context};
use chain::Db;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use xdevs_consensus::{ConsensusApp, ConsensusEngine, EngineStatus, MalachiteEngine, ValidatorSet};
use xdevs_execution::{next_base_fee, EvmExecutor};
use xdevs_genesis::Genesis;
use xdevs_mempool::{FromState, Pool, PoolConfig, PoolParams};
use xdevs_network::{Keypair, NetworkHandle};
use xdevs_primitives::{constants::DEV_CHAIN_ID, Bytes, ChainConfig, B256};
use xdevs_rpc::{ChainEvents, RpcServerConfig, RpcServerHandle};
use xdevs_state::DbStateProvider;
use xdevs_storage::RedbDatabase;

/// Per-transaction gas cap of the Osaka rules the executor runs (EIP-7825).
pub const MAX_TX_GAS: u64 = 1 << 24;
/// `extraData` of dev-mode blocks.
pub const DEV_BLOCK_EXTRA_DATA: &[u8] = b"xdevs";
/// Max wait for the consensus engine to stop at shutdown.
const ENGINE_STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// A node with its database open and genesis initialised, ready to [`Node::start`].
pub struct Node {
    config: NodeConfig,
    genesis: Genesis,
    genesis_hash: B256,
    cfg: ChainConfig,
    mode: NodeMode,
    validator_set: Option<ValidatorSet>,
    head_number: u64,
    db: Db,
    pool: Arc<Pool>,
    executor: Arc<EvmExecutor>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("config", &self.config)
            .field("mode", &self.mode)
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
    /// Prepare a node: load genesis, pick the run mode, open the database, initialise
    /// genesis state if the database is empty (and verify the stored genesis hash,
    /// validator set and head state root otherwise).
    pub fn new(config: NodeConfig) -> anyhow::Result<Self> {
        config.validate()?;
        let genesis = load_genesis(&config)?;
        let validator_set = consensus::genesis_validator_set(&genesis)?;
        let mode = config.mode(validator_set.is_some())?;
        let data_dir = &config.chain.data_dir;
        let db = RedbDatabase::open_dir(data_dir)
            .with_context(|| format!("failed to open database in {}", data_dir.display()))?;
        let db: Db = Arc::new(db);
        let genesis_hash = chain::init_genesis(&*db, &genesis)?;
        chain::ensure_validator_set(&*db, &genesis.validators)?;
        let (head_number, head) = chain::verify_head(&*db)?;
        tracing::info!(
            target: "xdevs::node",
            chain_id = genesis.config.chain_id,
            ?mode,
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
            genesis,
            genesis_hash,
            cfg,
            mode,
            validator_set,
            head_number,
            db,
            pool,
            executor,
        })
    }

    /// Node configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// How this node runs.
    pub fn mode(&self) -> NodeMode {
        self.mode
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

    /// Start the node: tx gossip and the consensus engine (validator / full node) or
    /// the block producer (dev), then the RPC server(s).
    pub async fn start(self) -> anyhow::Result<NodeHandle> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        // `eth_subscribe` events: committed blocks and admitted txs.
        let events = Arc::new(ChainEvents::default());
        let backend = NodeBackend::new(
            self.db.clone(),
            self.pool.clone(),
            self.executor.clone(),
            self.cfg.clone(),
            self.config.rpc.gas_cap,
        )
        .with_events(events.clone());
        let mut handle = NodeHandle {
            http_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            rpc: Vec::new(),
            producer: None,
            consensus: None,
            consensus_status: None,
            gossip: None,
            shutdown_tx,
            db: Some(self.db.clone()),
            genesis_hash: self.genesis_hash,
            mode: self.mode,
        };

        match self.mode {
            NodeMode::Dev => {
                let producer = Arc::new(
                    BlockProducer::new(
                        self.db.clone(),
                        self.pool.clone(),
                        self.executor.clone(),
                        self.cfg.clone(),
                        self.config.dev.fee_recipient,
                        Bytes::from_static(DEV_BLOCK_EXTRA_DATA),
                    )
                    .with_state_history(self.config.chain.state_history_blocks()?)
                    .with_events(events.clone()),
                );
                let block_time = self.block_time();
                tracing::info!(
                    target: "xdevs::node",
                    block_time_ms = block_time.as_millis() as u64,
                    fee_recipient = %self.config.dev.fee_recipient,
                    "dev block producer started"
                );
                handle.producer = Some(tokio::spawn(produce_loop(
                    producer,
                    block_time,
                    shutdown_rx,
                )));
            }
            NodeMode::RpcOnly => tracing::warn!(
                target: "xdevs::node",
                "dev mode disabled and the genesis has no validators: no blocks will be produced"
            ),
            NodeMode::Validator | NodeMode::FullNode => {
                if let Err(e) = self
                    .start_consensus(&mut handle, shutdown_rx, events.clone())
                    .await
                {
                    handle.shutdown().await;
                    return Err(e);
                }
            }
        }

        let mut backend = backend;
        if let Some(gossip) = &handle.gossip {
            backend = backend.with_gossip(gossip.clone());
        }
        if let Some(status) = &handle.consensus_status {
            backend = backend.with_consensus(status.clone(), self.head_number);
        }
        let backend = Arc::new(backend);
        let rpc_config = RpcServerConfig {
            http: self.config.rpc.http,
            gas_cap: self.config.rpc.gas_cap,
            max_logs_range: self.config.rpc.max_logs_range,
            max_batch: self.config.rpc.max_batch,
            cors: self.config.rpc.cors.clone(),
            ..Default::default()
        };
        let mut configs = vec![rpc_config.clone()];
        if let Some(ws) = self.config.rpc.ws {
            configs.push(RpcServerConfig {
                http: ws,
                ..rpc_config
            });
        }
        for (i, config) in configs.into_iter().enumerate() {
            if i > 0 && config.http == handle.http_addr {
                continue;
            }
            match xdevs_rpc::start_server(config, backend.clone()).await {
                Ok(server) => {
                    if i == 0 {
                        handle.http_addr = server.local_addr;
                    }
                    handle.rpc.push(server);
                }
                Err(e) => {
                    drop(backend);
                    handle.shutdown().await;
                    return Err(e);
                }
            }
        }
        Ok(handle)
    }

    /// Validator / full-node mode: tx gossip, then the consensus engine on a task.
    async fn start_consensus(
        &self,
        handle: &mut NodeHandle,
        shutdown: watch::Receiver<bool>,
        events: Arc<ChainEvents>,
    ) -> anyhow::Result<()> {
        let set = self
            .validator_set
            .clone()
            .context("consensus mode without a genesis validator set")?;
        let consensus_key = if self.mode == NodeMode::Validator {
            let key = consensus::load_consensus_key(&self.config.validator)?;
            let address = xdevs_consensus::ValidatorKey(key.public_key()).address();
            if set.get(&address).is_none() {
                tracing::warn!(
                    target: "xdevs::node",
                    %address,
                    "the consensus key is not in the genesis validator set: its votes \
                     never count towards a quorum"
                );
            }
            Some(key)
        } else {
            None
        };
        let node_key = consensus::load_node_key(self.config.p2p.node_key_file.as_deref())?;

        if self.config.gossip.enabled {
            let mut seed = zeroize::Zeroizing::new(*node_key.as_bytes());
            let keypair =
                Keypair::ed25519_from_bytes(&mut seed[..]).map_err(|e| anyhow!("node key: {e}"))?;
            let gossip = xdevs_network::start(
                self.config.gossip.network.clone(),
                self.cfg.chain_id,
                keypair,
                events::publishing_gossip_validator(
                    consensus::gossip_validator(self.pool.clone(), self.cfg.chain_id),
                    self.pool.clone(),
                    events.clone(),
                ),
            )
            .await
            .context("failed to start tx gossip")?;
            tracing::info!(
                target: "xdevs::node",
                peer_id = %gossip.local_peer_id(),
                listen = %self.config.gossip.network.listen,
                "tx gossip started"
            );
            handle.gossip = Some(Arc::new(gossip));
        }

        let (engine_config, keys) = consensus::engine_setup(
            &self.config,
            &self.genesis,
            consensus_key.as_ref(),
            &node_key,
        );
        let app = ChainApp::new(
            self.db.clone(),
            self.pool.clone(),
            self.executor.clone(),
            self.cfg.clone(),
            set,
            ChainAppConfig {
                fee_recipient: self.config.validator.fee_recipient,
                extra_data: Bytes::from_static(PROPOSAL_EXTRA_DATA),
                state_history: self.config.chain.state_history_blocks()?,
                max_timestamp_drift_secs: self.config.validator.max_timestamp_drift_secs,
            },
        )
        .with_events(events);
        tracing::info!(
            target: "xdevs::node",
            mode = ?self.mode,
            moniker = %engine_config.moniker,
            p2p = %engine_config.p2p.listen,
            head = self.head_number,
            "starting consensus engine"
        );
        let engine = MalachiteEngine::new(engine_config, keys);
        handle.consensus_status = Some(engine.status());
        let app: Arc<dyn ConsensusApp> = Arc::new(app);
        handle.consensus = Some(tokio::spawn(engine.run(app, shutdown)));
        Ok(())
    }

    /// Run until Ctrl-C (then shut down gracefully), or until the consensus engine
    /// halts: then everything is stopped and the engine's error is returned. The node
    /// never restarts a halted engine by itself (a SignGuard refusal must be looked at
    /// by an operator).
    pub async fn run(self) -> anyhow::Result<()> {
        let mut handle = self.start().await?;
        let result = tokio::select! {
            r = tokio::signal::ctrl_c() => {
                r.context("failed to listen for Ctrl-C")?;
                tracing::info!(target: "xdevs::node", "Ctrl-C received, shutting down");
                Ok(())
            }
            e = handle.consensus_halted() => {
                tracing::error!(target: "xdevs::node", "consensus halted, stopping the node: {e:#}");
                Err(e)
            }
        };
        handle.shutdown().await;
        tracing::info!(target: "xdevs::node", "shutdown complete");
        result
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
    consensus: Option<JoinHandle<anyhow::Result<()>>>,
    consensus_status: Option<EngineStatus>,
    gossip: Option<Arc<NetworkHandle>>,
    shutdown_tx: watch::Sender<bool>,
    db: Option<Db>,
    genesis_hash: B256,
    mode: NodeMode,
}

impl std::fmt::Debug for NodeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle")
            .field("http_addr", &self.http_addr)
            .field("mode", &self.mode)
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

    /// How the node runs.
    pub fn mode(&self) -> NodeMode {
        self.mode
    }

    /// Live consensus status (validator / full-node mode).
    pub fn consensus_status(&self) -> Option<&EngineStatus> {
        self.consensus_status.as_ref()
    }

    /// Inbound tx-gossip counters (validator / full-node mode with `[gossip]` enabled).
    pub fn gossip_stats(&self) -> Option<&xdevs_network::GossipStats> {
        self.gossip.as_deref().map(NetworkHandle::stats)
    }

    /// Connected tx-gossip peers subscribed to the tx topic (0 without gossip).
    pub fn gossip_topic_peers(&self) -> usize {
        self.gossip
            .as_deref()
            .map_or(0, NetworkHandle::topic_peer_count)
    }

    /// `true` while the consensus engine task is alive (always `false` without
    /// consensus).
    pub fn consensus_running(&self) -> bool {
        self.consensus.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Wait until the consensus engine stops on its own and return why. Pending forever
    /// in the modes without consensus, or once the stop was already reported.
    pub async fn consensus_halted(&mut self) -> anyhow::Error {
        let Some(task) = self.consensus.as_mut() else {
            return std::future::pending().await;
        };
        let result = task.await;
        self.consensus = None;
        match result {
            Ok(Ok(())) => anyhow!("consensus engine stopped unexpectedly"),
            Ok(Err(e)) => e.context("consensus engine halted"),
            Err(e) => anyhow!("consensus engine task failed: {e}"),
        }
    }

    /// Stop block production / consensus, the RPC servers and tx gossip, then wait (up
    /// to 10 s) until every database handle is released so the data dir can be
    /// reopened.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(p) = self.producer.take() {
            if let Err(e) = p.await {
                tracing::error!(target: "xdevs::node", error = %e, "producer task failed");
            }
        }
        if let Some(task) = self.consensus.take() {
            match tokio::time::timeout(ENGINE_STOP_TIMEOUT, task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(e))) => tracing::error!(
                    target: "xdevs::node",
                    "consensus engine stopped with an error: {e:#}"
                ),
                Ok(Err(e)) => {
                    tracing::error!(target: "xdevs::node", error = %e, "consensus task failed")
                }
                Err(_) => tracing::error!(
                    target: "xdevs::node",
                    "consensus engine did not stop in time"
                ),
            }
        }
        for server in std::mem::take(&mut self.rpc) {
            server.stop();
            server.stopped().await;
        }
        if let Some(mut gossip) = self.gossip.take() {
            // The RPC backend holds the other reference until its servers are gone.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                match Arc::try_unwrap(gossip) {
                    Ok(network) => {
                        network.shutdown().await;
                        break;
                    }
                    Err(shared) if tokio::time::Instant::now() < deadline => {
                        gossip = shared;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(_) => {
                        tracing::warn!(target: "xdevs::node", "tx gossip still referenced after shutdown");
                        break;
                    }
                }
            }
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

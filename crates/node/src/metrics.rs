//! Prometheus `/metrics` for the node (`[metrics]`, plan §24.7).
//!
//! Observability only: committed blocks are taken from the same [`ChainEvents`] stream that
//! feeds `eth_subscribe`, and peer / mempool gauges are sampled on a timer, so nothing here
//! touches the consensus or commit paths.

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use tokio::{
    sync::{broadcast::error::RecvError, watch},
    task::JoinHandle,
};
use xdevs_consensus::EngineStatus;
use xdevs_mempool::{Pool, TxPool};
use xdevs_metrics::{ChainMetrics, CommittedBlock, MetricsConfig, MetricsRegistry, MetricsServer};
use xdevs_network::NetworkHandle;
use xdevs_rpc::ChainEvents;

/// How often peer and mempool gauges are refreshed.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// The running metrics endpoint and its feeder tasks.
#[derive(Debug)]
pub(crate) struct NodeMetrics {
    server: MetricsServer,
    tasks: Vec<JoinHandle<()>>,
}

/// What the gauges are sampled from.
pub(crate) struct MetricsSources {
    pub(crate) events: Arc<ChainEvents>,
    pub(crate) pool: Arc<Pool>,
    pub(crate) gossip: Option<Arc<NetworkHandle>>,
    pub(crate) consensus: Option<EngineStatus>,
}

impl NodeMetrics {
    /// Start `/metrics` if `[metrics] enabled`; `None` when disabled.
    pub(crate) async fn start(
        config: &MetricsConfig,
        sources: MetricsSources,
        shutdown: watch::Receiver<bool>,
    ) -> anyhow::Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        let addr = config.listen_addr().map_err(anyhow::Error::msg)?;
        let registry = MetricsRegistry::new();
        let chain = ChainMetrics::register(&registry);
        let server = MetricsServer::start(addr, registry)
            .await
            .with_context(|| format!("starting the metrics endpoint on {addr}"))?;
        tracing::info!(target: "xdevs::node", addr = %server.local_addr(), "metrics endpoint started");

        let blocks = tokio::spawn(record_blocks(
            chain.clone(),
            sources.events.clone(),
            shutdown.clone(),
        ));
        let gauges = tokio::spawn(sample_gauges(chain, sources, shutdown));
        Ok(Some(Self {
            server,
            tasks: vec![blocks, gauges],
        }))
    }

    /// Address `/metrics` is served on.
    pub(crate) fn local_addr(&self) -> std::net::SocketAddr {
        self.server.local_addr()
    }

    /// Stop the endpoint and wait for the feeder tasks (they end on the shutdown signal).
    pub(crate) async fn shutdown(self) {
        self.server.shutdown().await;
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// Record every committed block (height, block time, finality latency, tx count).
async fn record_blocks(
    chain: ChainMetrics,
    events: Arc<ChainEvents>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut blocks = events.subscribe_blocks();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            next = blocks.recv() => match next {
                Ok(block) => chain.on_block_committed(CommittedBlock {
                    height: block.header.number,
                    round: 0,
                    tx_count: block.tx_count as u64,
                    timestamp: block.header.timestamp,
                    finality_latency: None,
                }),
                // Metrics may skip blocks under heavy load; the height gauge catches up.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// Refresh peer and mempool gauges every [`SAMPLE_INTERVAL`].
async fn sample_gauges(
    chain: ChainMetrics,
    sources: MetricsSources,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {
                let status = sources.pool.status();
                chain.set_mempool(status.pending as u64, status.queued as u64);
                if let Some(gossip) = &sources.gossip {
                    chain.set_gossip_peers(gossip.peer_count() as u64);
                }
                if let Some(consensus) = &sources.consensus {
                    chain.set_consensus_peers(consensus.peer_count() as u64);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Node, NodeConfig};
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn scrape(addr: std::net::SocketAddr) -> String {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    /// Value of an un-labelled sample `name` in an OpenMetrics text body.
    fn sample(body: &str, name: &str) -> Option<f64> {
        body.lines()
            .find_map(|l| l.strip_prefix(&format!("{name} ")))
            .and_then(|v| v.trim().parse().ok())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dev_node_serves_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = NodeConfig::default();
        config.chain.data_dir = dir.path().join("data");
        config.rpc.http = "127.0.0.1:0".parse().unwrap();
        config.dev.block_time_ms = Some(200);
        config.metrics.enabled = true;
        config.metrics.listen = "127.0.0.1:0".into();
        let handle = Node::new(config).unwrap().start().await.unwrap();
        let addr = handle.metrics_addr().expect("metrics enabled");

        // Wait until a few blocks are recorded.
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        let body = loop {
            let body = scrape(addr).await;
            if sample(&body, "xdevs_block_height").is_some_and(|h| h >= 3.0) {
                break body;
            }
            assert!(
                Instant::now() < deadline,
                "block height never reached 3:\n{body}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        };
        assert!(body.starts_with("HTTP/1.1 200 OK"), "{body}");
        assert!(body.contains("xdevs_block_time_seconds_count"), "{body}");
        assert!(body.contains("xdevs_transactions_total"), "{body}");
        assert!(body.contains("xdevs_mempool_size"), "{body}");
        assert!(body.trim_end().ends_with("# EOF"), "{body}");

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn disabled_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = NodeConfig::default();
        config.chain.data_dir = dir.path().join("data");
        config.rpc.http = "127.0.0.1:0".parse().unwrap();
        let handle = Node::new(config).unwrap().start().await.unwrap();
        assert!(handle.metrics_addr().is_none());
        handle.shutdown().await;
    }
}

//! Chain and consensus metrics (plan §24.7, research doc §7 `build_ms` / `validate_ms`).

use crate::{MetricsRegistry, DURATION_BUCKETS};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Buckets (seconds) for block time and finality latency.
const BLOCK_TIME_BUCKETS: [f64; 13] = [
    0.25, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 7.5, 10.0, 20.0, 30.0,
];

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct NetworkLabels {
    network: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct PoolLabels {
    state: &'static str,
}

/// What the node reports after it has committed a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommittedBlock {
    /// Block number.
    pub height: u64,
    /// Consensus round the block was decided in (0 = first round; dev mode: 0).
    pub round: u32,
    /// Transactions in the block.
    pub tx_count: u64,
    /// Header timestamp (unix seconds).
    pub timestamp: u64,
    /// Time from the start of the height (or the proposal) to commit, if the caller
    /// measured it. `None` = derive it from `timestamp` (1 s resolution).
    pub finality_latency: Option<Duration>,
}

/// Chain / consensus metrics. Clones share the same underlying metrics.
///
/// Names (with the `xdevs_` prefix): `block_height`, `block_time_seconds`,
/// `finality_latency_seconds`, `consensus_height`, `consensus_round`,
/// `consensus_rounds_total` (extra rounds: > 0 means trouble), `transactions_total`
/// (`rate()` = tx throughput), `block_build_duration_seconds` (`build_ms`),
/// `block_validate_duration_seconds` (`validate_ms`, the re-execution on validators),
/// `block_commit_duration_seconds` (`commit_ms`), `state_root_duration_seconds`,
/// `peers{network="consensus"|"gossip"}`, `mempool_size{state="pending"|"queued"}`.
#[derive(Clone, Debug)]
pub struct ChainMetrics {
    block_height: Gauge,
    block_time: Histogram,
    finality_latency: Histogram,
    consensus_height: Gauge,
    consensus_round: Gauge,
    consensus_rounds: Counter,
    transactions: Counter,
    build: Histogram,
    validate: Histogram,
    commit: Histogram,
    state_root: Histogram,
    peers: Family<NetworkLabels, Gauge>,
    mempool: Family<PoolLabels, Gauge>,
    last_commit: Arc<Mutex<Option<Instant>>>,
}

fn gauge_value(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl ChainMetrics {
    /// Create and register every chain metric in `registry`.
    pub fn register(registry: &MetricsRegistry) -> Self {
        let duration = || Histogram::new(DURATION_BUCKETS);
        let m = Self {
            block_height: Gauge::default(),
            block_time: Histogram::new(BLOCK_TIME_BUCKETS),
            finality_latency: Histogram::new(BLOCK_TIME_BUCKETS),
            consensus_height: Gauge::default(),
            consensus_round: Gauge::default(),
            consensus_rounds: Counter::default(),
            transactions: Counter::default(),
            build: duration(),
            validate: duration(),
            commit: duration(),
            state_root: duration(),
            peers: Family::default(),
            mempool: Family::default(),
            last_commit: Arc::new(Mutex::new(None)),
        };
        registry.with_registry(|r| {
            r.register(
                "block_height",
                "Latest committed block number",
                m.block_height.clone(),
            );
            r.register(
                "block_time_seconds",
                "Wall-clock time between consecutive block commits",
                m.block_time.clone(),
            );
            r.register(
                "finality_latency_seconds",
                "Time from block proposal (or header timestamp) to commit",
                m.finality_latency.clone(),
            );
            r.register(
                "consensus_height",
                "Height the consensus engine is working on",
                m.consensus_height.clone(),
            );
            r.register(
                "consensus_round",
                "Current consensus round at the current height",
                m.consensus_round.clone(),
            );
            r.register(
                "consensus_rounds",
                "Extra consensus rounds needed to decide blocks (round > 0 means trouble)",
                m.consensus_rounds.clone(),
            );
            r.register(
                "transactions",
                "Transactions in committed blocks (rate() = throughput)",
                m.transactions.clone(),
            );
            r.register(
                "block_build_duration_seconds",
                "Proposer block build + execution time (build_ms)",
                m.build.clone(),
            );
            r.register(
                "block_validate_duration_seconds",
                "Validator re-execution of a proposed block (validate_ms)",
                m.validate.clone(),
            );
            r.register(
                "block_commit_duration_seconds",
                "Time to persist a decided block (commit_ms)",
                m.commit.clone(),
            );
            r.register(
                "state_root_duration_seconds",
                "State root computation time",
                m.state_root.clone(),
            );
            r.register("peers", "Connected peers per network", m.peers.clone());
            r.register(
                "mempool_size",
                "Transactions in the mempool",
                m.mempool.clone(),
            );
        });
        m
    }

    /// Record a committed block: height, block time (wall clock since the previous
    /// commit), finality latency, extra rounds and tx count.
    pub fn on_block_committed(&self, block: CommittedBlock) {
        self.on_block_committed_at(block, Instant::now(), SystemTime::now());
    }

    fn on_block_committed_at(&self, block: CommittedBlock, now: Instant, wall: SystemTime) {
        self.block_height.set(gauge_value(block.height));
        self.transactions.inc_by(block.tx_count);
        self.consensus_rounds.inc_by(u64::from(block.round));
        let latency = block.finality_latency.unwrap_or_else(|| {
            let now_secs = wall
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Duration::from_secs_f64((now_secs - block.timestamp as f64).max(0.0))
        });
        self.finality_latency.observe(latency.as_secs_f64());
        let mut last = self
            .last_commit
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(prev) = last.replace(now) {
            self.block_time
                .observe(now.saturating_duration_since(prev).as_secs_f64());
        }
    }

    /// The engine started `round` at `height`.
    pub fn on_round_started(&self, height: u64, round: u32) {
        self.consensus_height.set(gauge_value(height));
        self.consensus_round.set(i64::from(round));
    }

    /// Proposer build time (`build_ms`).
    pub fn observe_build(&self, elapsed: Duration) {
        self.build.observe(elapsed.as_secs_f64());
    }

    /// Validator re-execution time (`validate_ms`).
    pub fn observe_validate(&self, elapsed: Duration) {
        self.validate.observe(elapsed.as_secs_f64());
    }

    /// Commit (persist) time (`commit_ms`).
    pub fn observe_commit(&self, elapsed: Duration) {
        self.commit.observe(elapsed.as_secs_f64());
    }

    /// State root computation time.
    pub fn observe_state_root(&self, elapsed: Duration) {
        self.state_root.observe(elapsed.as_secs_f64());
    }

    /// Peers of the Malachite consensus network.
    pub fn set_consensus_peers(&self, peers: u64) {
        self.peers
            .get_or_create(&NetworkLabels {
                network: "consensus",
            })
            .set(gauge_value(peers));
    }

    /// Peers of the tx-gossip network.
    pub fn set_gossip_peers(&self, peers: u64) {
        self.peers
            .get_or_create(&NetworkLabels { network: "gossip" })
            .set(gauge_value(peers));
    }

    /// Mempool size (`PoolStatus { pending, queued }`).
    pub fn set_mempool(&self, pending: u64, queued: u64) {
        self.mempool
            .get_or_create(&PoolLabels { state: "pending" })
            .set(gauge_value(pending));
        self.mempool
            .get_or_create(&PoolLabels { state: "queued" })
            .set(gauge_value(queued));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(height: u64, round: u32, tx_count: u64) -> CommittedBlock {
        CommittedBlock {
            height,
            round,
            tx_count,
            timestamp: 1_700_000_000,
            finality_latency: Some(Duration::from_millis(1500)),
        }
    }

    #[test]
    fn records_and_encodes_every_metric() {
        let reg = MetricsRegistry::new();
        let m = ChainMetrics::register(&reg);
        let t0 = Instant::now();
        let wall = UNIX_EPOCH + Duration::from_secs(1_700_000_002);
        m.on_block_committed_at(block(1, 0, 3), t0, wall);
        m.on_block_committed_at(block(2, 2, 4), t0 + Duration::from_secs(2), wall);
        m.on_round_started(3, 1);
        m.observe_build(Duration::from_millis(12));
        m.observe_validate(Duration::from_millis(15));
        m.observe_commit(Duration::from_millis(4));
        m.observe_state_root(Duration::from_millis(3));
        m.set_consensus_peers(3);
        m.set_gossip_peers(5);
        m.set_mempool(10, 2);

        let text = reg.render().unwrap();
        for needle in [
            "xdevs_block_height 2",
            "xdevs_transactions_total 7",
            "xdevs_consensus_rounds_total 2",
            "xdevs_consensus_height 3",
            "xdevs_consensus_round 1",
            // one interval between two commits, 2 s.
            "xdevs_block_time_seconds_count 1",
            "xdevs_block_time_seconds_sum 2.0",
            "xdevs_finality_latency_seconds_count 2",
            "xdevs_finality_latency_seconds_sum 3.0",
            "xdevs_block_build_duration_seconds_count 1",
            "xdevs_block_validate_duration_seconds_count 1",
            "xdevs_block_commit_duration_seconds_count 1",
            "xdevs_state_root_duration_seconds_count 1",
            "xdevs_peers{network=\"consensus\"} 3",
            "xdevs_peers{network=\"gossip\"} 5",
            "xdevs_mempool_size{state=\"pending\"} 10",
            "xdevs_mempool_size{state=\"queued\"} 2",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in\n{text}");
        }
    }

    #[test]
    fn finality_latency_from_timestamp_when_not_measured() {
        let reg = MetricsRegistry::new();
        let m = ChainMetrics::register(&reg);
        let b = CommittedBlock {
            finality_latency: None,
            ..block(1, 0, 0)
        };
        // Header timestamp 1_700_000_000, now 4 s later.
        let wall = UNIX_EPOCH + Duration::from_secs(1_700_000_004);
        m.on_block_committed_at(b, Instant::now(), wall);
        // A timestamp in the future (clock skew) is clamped to 0, not negative.
        let wall = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        m.on_block_committed_at(b, Instant::now(), wall);
        let text = reg.render().unwrap();
        assert!(
            text.contains("xdevs_finality_latency_seconds_sum 4.0"),
            "{text}"
        );
        assert!(text.contains("xdevs_finality_latency_seconds_count 2"));
    }

    #[test]
    fn huge_values_saturate() {
        assert_eq!(gauge_value(u64::MAX), i64::MAX);
        assert_eq!(gauge_value(5), 5);
    }
}

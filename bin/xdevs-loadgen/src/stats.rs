//! Statistics: percentiles, the run report (JSON for CI / soak) and its human summary.

use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write as _};

/// Nearest-rank percentile of an ascending-sorted slice (`p` in 0..=100). `None` if empty.
pub fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let p = p.clamp(0.0, 100.0);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// Latency distribution in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LatencySummary {
    /// Number of samples.
    pub count: usize,
    /// Minimum.
    pub min: f64,
    /// Mean.
    pub mean: f64,
    /// 50th percentile.
    pub p50: f64,
    /// 90th percentile.
    pub p90: f64,
    /// 99th percentile.
    pub p99: f64,
    /// Maximum.
    pub max: f64,
}

impl LatencySummary {
    /// Summarize `samples` (any order).
    pub fn from_samples(samples: &[f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut v = samples.to_vec();
        v.sort_by(f64::total_cmp);
        let pct = |p| round1(percentile(&v, p).unwrap_or(0.0));
        Self {
            count: v.len(),
            min: round1(v[0]),
            mean: round1(v.iter().sum::<f64>() / v.len() as f64),
            p50: pct(50.0),
            p90: pct(90.0),
            p99: pct(99.0),
            max: round1(v[v.len() - 1]),
        }
    }
}

/// Round to one decimal (keeps the JSON readable).
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Round to three decimals.
pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Per-kind counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KindStats {
    /// Signed and handed to the RPC.
    pub sent: u64,
    /// Seen in a block.
    pub included: u64,
    /// Included with receipt status 0.
    pub failed: u64,
}

/// The run's settings, echoed in the report.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReportConfig {
    /// RPC endpoints (sends are round-robin).
    pub rpc: Vec<String>,
    /// Target rate.
    pub target_tps: f64,
    /// Configured send-phase length (s).
    pub duration_s: f64,
    /// Number of sender accounts.
    pub senders: usize,
    /// Tx mix, e.g. `transfer=80,erc20=20`.
    pub mix: String,
    /// Per-sender unconfirmed-tx window.
    pub window_per_sender: u64,
    /// Chain id.
    pub chain_id: u64,
}

/// Full run report.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    /// Settings.
    pub config: ReportConfig,
    /// Stopped early by Ctrl-C.
    pub interrupted: bool,
    /// Actual send-phase length (s).
    pub send_duration_s: f64,
    /// Wall time from the first send to the end of the drain (s).
    pub total_duration_s: f64,
    /// Txs signed and sent.
    pub sent: u64,
    /// `eth_sendRawTransaction` returned a hash.
    pub accepted: u64,
    /// Definitively rejected by the node.
    pub rejected: u64,
    /// RPC timeouts / transport errors (outcome unknown).
    pub send_errors: u64,
    /// Error message (normalized) -> count, rejections and send errors together.
    pub errors: BTreeMap<String, u64>,
    /// Txs seen in a block (any send outcome).
    pub included: u64,
    /// Accepted txs seen in a block.
    pub included_accepted: u64,
    /// Accepted txs never seen in a block before the drain ended.
    pub not_included: u64,
    /// `included_accepted / accepted` (1.0 when nothing was accepted).
    pub inclusion_rate: f64,
    /// Included txs whose receipt status is 1.
    pub receipts_ok: u64,
    /// Included txs whose receipt status is 0.
    pub receipts_failed: u64,
    /// Included txs whose receipt could not be fetched.
    pub receipts_unchecked: u64,
    /// Send → first poll that saw the block (ms).
    pub latency_ms: LatencySummary,
    /// Send → block timestamp (ms; 1 s timestamp resolution, clamped at 0).
    pub block_ts_latency_ms: LatencySummary,
    /// `sent / send_duration_s`.
    pub send_tps: f64,
    /// Included txs / (first send → last inclusion).
    pub achieved_tps: f64,
    /// Blocks seen between the run start and the last inclusion.
    pub blocks_seen: u64,
    /// Of those, blocks without any tx.
    pub empty_blocks: u64,
    /// All txs in those blocks (including other senders' txs).
    pub block_txs: u64,
    /// First block of the run.
    pub first_block: u64,
    /// Last block looked at.
    pub last_block: u64,
    /// Nonce resyncs (`eth_getTransactionCount`) after errors.
    pub nonce_resyncs: u64,
    /// Times the send loop had a token but every sender's window was full.
    pub window_full_stalls: u64,
    /// Per-kind counters.
    pub by_kind: BTreeMap<String, KindStats>,
}

impl Report {
    /// Human-readable summary.
    pub fn human(&self) -> String {
        let c = &self.config;
        let mut s = String::new();
        let _ = writeln!(s, "=== xdevs-loadgen report ===");
        let _ = writeln!(
            s,
            "target      {} tps for {} s, {} senders, mix {}, chain {}, rpc {}",
            c.target_tps,
            c.duration_s,
            c.senders,
            c.mix,
            c.chain_id,
            c.rpc.join(",")
        );
        if self.interrupted {
            let _ = writeln!(s, "INTERRUPTED (Ctrl-C): partial results");
        }
        let _ = writeln!(
            s,
            "sent        {} in {:.1} s ({:.1} tps)",
            self.sent, self.send_duration_s, self.send_tps
        );
        let _ = writeln!(
            s,
            "accepted    {}   rejected {}   send errors {}",
            self.accepted, self.rejected, self.send_errors
        );
        for (e, n) in &self.errors {
            let _ = writeln!(s, "  {n:>8} x {e}");
        }
        let _ = writeln!(
            s,
            "included    {} ({:.2}% of accepted), not included {}",
            self.included,
            self.inclusion_rate * 100.0,
            self.not_included
        );
        let _ = writeln!(
            s,
            "receipts    ok {}   failed {}   unchecked {}",
            self.receipts_ok, self.receipts_failed, self.receipts_unchecked
        );
        let l = &self.latency_ms;
        let _ = writeln!(
            s,
            "latency ms  p50 {}  p90 {}  p99 {}  (min {}, mean {}, max {}; send -> block observed)",
            l.p50, l.p90, l.p99, l.min, l.mean, l.max
        );
        let b = &self.block_ts_latency_ms;
        let _ = writeln!(
            s,
            "  block ts  p50 {}  p90 {}  p99 {}  (send -> block timestamp, 1 s resolution)",
            b.p50, b.p90, b.p99
        );
        let _ = writeln!(
            s,
            "achieved    {:.1} tps included (target {})",
            self.achieved_tps, c.target_tps
        );
        let _ = writeln!(
            s,
            "blocks      {} seen (#{}..#{}), {} empty, {} txs total",
            self.blocks_seen, self.first_block, self.last_block, self.empty_blocks, self.block_txs
        );
        for (k, v) in &self.by_kind {
            let _ = writeln!(
                s,
                "  {k:<9} sent {}  included {}  failed {}",
                v.sent, v.included, v.failed
            );
        }
        let _ = writeln!(
            s,
            "nonce resyncs {}   window-full stalls {}",
            self.nonce_resyncs, self.window_full_stalls
        );
        s
    }

    /// Pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("report serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 50.0), Some(50.0));
        assert_eq!(percentile(&v, 90.0), Some(90.0));
        assert_eq!(percentile(&v, 99.0), Some(99.0));
        assert_eq!(percentile(&v, 100.0), Some(100.0));
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&[], 50.0), None);
        assert_eq!(percentile(&[7.0], 99.0), Some(7.0));
        // 4 samples: p50 is the 2nd, p90 / p99 the 4th.
        let v = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&v, 50.0), Some(20.0));
        assert_eq!(percentile(&v, 90.0), Some(40.0));
        assert_eq!(percentile(&v, 25.0), Some(10.0));
        assert_eq!(percentile(&v, 26.0), Some(20.0));
    }

    #[test]
    fn summary_sorts_and_rounds() {
        let s = LatencySummary::from_samples(&[300.0, 100.0, 200.04, 400.0]);
        assert_eq!(s.count, 4);
        assert_eq!(s.min, 100.0);
        assert_eq!(s.p50, 200.0);
        assert_eq!(s.p99, 400.0);
        assert_eq!(s.max, 400.0);
        assert_eq!(s.mean, 250.0);
        assert_eq!(LatencySummary::from_samples(&[]), LatencySummary::default());
    }

    #[test]
    fn report_json_shape() {
        let mut r = Report {
            sent: 10,
            accepted: 9,
            rejected: 1,
            included: 9,
            included_accepted: 9,
            inclusion_rate: 1.0,
            latency_ms: LatencySummary::from_samples(&[1.0, 2.0]),
            ..Default::default()
        };
        r.config.mix = "transfer=100".into();
        r.errors.insert("nonce too low".into(), 1);
        r.by_kind.insert(
            "transfer".into(),
            KindStats {
                sent: 10,
                included: 9,
                failed: 0,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["sent"], 10);
        assert_eq!(v["accepted"], 9);
        assert_eq!(v["errors"]["nonce too low"], 1);
        assert_eq!(v["inclusion_rate"], 1.0);
        assert_eq!(v["latency_ms"]["p50"], 1.0);
        assert_eq!(v["latency_ms"]["count"], 2);
        assert_eq!(v["by_kind"]["transfer"]["included"], 9);
        assert_eq!(v["config"]["mix"], "transfer=100");
        assert_eq!(v["interrupted"], false);
        for key in [
            "receipts_ok",
            "receipts_failed",
            "achieved_tps",
            "blocks_seen",
            "empty_blocks",
            "block_ts_latency_ms",
            "not_included",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        let h = r.human();
        assert!(h.contains("accepted    9"));
        assert!(h.contains("1 x nonce too low"));
    }
}

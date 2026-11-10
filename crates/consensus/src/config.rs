//! Consensus configuration (research doc §6.10, §7): the `[consensus]` and `[p2p]` node
//! config sections and the [`EngineConfig`] handed to [`crate::MalachiteEngine`].
//!
//! The node (WP2) embeds [`ConsensusTimeouts`] as `[consensus]` and [`P2pConfig`] as
//! `[p2p]` in its `NodeConfig`. Multiaddrs are plain strings here so no libp2p or
//! Malachite type leaks out of this crate; the engine parses them at start.

use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

/// `[consensus]`: block time and round timeouts (plan §11.2). All values in milliseconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsensusTimeouts {
    /// Minimum height duration (Malachite `HeightParams.target_time`); the block time.
    pub block_time_ms: u64,
    /// Wait for a proposal before prevoting nil.
    pub timeout_propose_ms: u64,
    /// Increase of `timeout_propose` per round.
    pub timeout_propose_delta_ms: u64,
    /// Wait after +2/3 prevotes for anything.
    pub timeout_prevote_ms: u64,
    /// Increase of `timeout_prevote` per round.
    pub timeout_prevote_delta_ms: u64,
    /// Wait after +2/3 precommits for anything.
    pub timeout_precommit_ms: u64,
    /// Increase of `timeout_precommit` per round.
    pub timeout_precommit_delta_ms: u64,
    /// Upper bound of any per-round timeout.
    pub max_timeout_ms: u64,
}

impl Default for ConsensusTimeouts {
    fn default() -> Self {
        Self {
            block_time_ms: 2000,
            timeout_propose_ms: 1000,
            timeout_propose_delta_ms: 500,
            timeout_prevote_ms: 500,
            timeout_prevote_delta_ms: 250,
            timeout_precommit_ms: 500,
            timeout_precommit_delta_ms: 250,
            max_timeout_ms: 10_000,
        }
    }
}

impl ConsensusTimeouts {
    /// Target block time.
    pub fn block_time(&self) -> Duration {
        Duration::from_millis(self.block_time_ms)
    }

    /// Sanity checks: base timeouts > 0 and none above `max_timeout_ms`.
    pub fn validate(&self) -> Result<(), String> {
        let bases = [
            ("timeout_propose_ms", self.timeout_propose_ms),
            ("timeout_prevote_ms", self.timeout_prevote_ms),
            ("timeout_precommit_ms", self.timeout_precommit_ms),
        ];
        for (name, v) in bases {
            if v == 0 {
                return Err(format!("consensus.{name} must be > 0"));
            }
            if v > self.max_timeout_ms {
                return Err(format!("consensus.{name} exceeds max_timeout_ms"));
            }
        }
        Ok(())
    }

    /// Timeouts scaled down for in-process tests (`block_time_ms` given, rest proportional).
    pub fn fast_for_tests(block_time_ms: u64) -> Self {
        let t = block_time_ms.max(10);
        Self {
            block_time_ms: t,
            timeout_propose_ms: t,
            timeout_propose_delta_ms: t / 2,
            timeout_prevote_ms: t / 2,
            timeout_prevote_delta_ms: t / 4,
            timeout_precommit_ms: t / 2,
            timeout_precommit_delta_ms: t / 4,
            max_timeout_ms: t * 10,
        }
    }
}

/// `[p2p]`: Malachite's consensus network (consensus, proposal parts, sync, liveness).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct P2pConfig {
    /// Listen multiaddr (TCP in Phase 2; QUIC later: `/ip4/0.0.0.0/udp/30333/quic-v1`).
    pub listen: String,
    /// Peers to keep connected (multiaddrs).
    pub persistent_peers: Vec<String>,
    /// libp2p Ed25519 node identity file (WP4 format). `None` = ephemeral (tests).
    pub node_key_file: Option<PathBuf>,
    /// Kademlia discovery (off in Phase 2: static peers).
    pub discovery: bool,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            listen: "/ip4/0.0.0.0/tcp/30333".into(),
            persistent_peers: Vec::new(),
            node_key_file: None,
            discovery: false,
        }
    }
}

/// Everything [`crate::MalachiteEngine`] needs besides keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// EVM chain id (domain of every sign-bytes and channel name `/xdevs/<chain>/…`).
    pub chain_id: u64,
    /// Node name in logs and metrics.
    pub moniker: String,
    /// `<data_dir>/consensus`: Malachite WAL (`wal/`) and `consensus.redb` (SignGuard,
    /// undecided blocks). Kept apart from the chain DB so wiping it never resets them.
    pub consensus_dir: PathBuf,
    /// `[consensus]`.
    pub timeouts: ConsensusTimeouts,
    /// `[p2p]`.
    pub p2p: P2pConfig,
    /// Malachite ValueSync (block sync for lagging / new nodes).
    pub value_sync: bool,
}

impl EngineConfig {
    /// Defaults for `chain_id` with the consensus dir under `data_dir`.
    pub fn new(chain_id: u64, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            chain_id,
            moniker: "xdevs".into(),
            consensus_dir: data_dir.into().join("consensus"),
            timeouts: ConsensusTimeouts::default(),
            p2p: P2pConfig::default(),
            value_sync: true,
        }
    }

    /// Malachite WAL path.
    pub fn wal_path(&self) -> PathBuf {
        self.consensus_dir.join("wal")
    }

    /// SignGuard + undecided-block store path.
    pub fn consensus_db_path(&self) -> PathBuf {
        self.consensus_dir.join("consensus.redb")
    }

    /// Gossip channel name `/xdevs/<chain_id>/<name>` (e.g. `consensus`).
    pub fn channel_name(&self, name: &str) -> String {
        format!("/xdevs/{}/{name}", self.chain_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_plan() {
        let t = ConsensusTimeouts::default();
        assert_eq!(t.block_time(), Duration::from_secs(2));
        assert_eq!(
            (t.timeout_propose_ms, t.timeout_propose_delta_ms),
            (1000, 500)
        );
        assert_eq!(
            (t.timeout_prevote_ms, t.timeout_prevote_delta_ms),
            (500, 250)
        );
        assert_eq!(
            (t.timeout_precommit_ms, t.timeout_precommit_delta_ms),
            (500, 250)
        );
        assert_eq!(t.max_timeout_ms, 10_000);
        t.validate().unwrap();
        ConsensusTimeouts::fast_for_tests(300).validate().unwrap();
        ConsensusTimeouts::fast_for_tests(0).validate().unwrap();
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut t = ConsensusTimeouts {
            timeout_prevote_ms: 0,
            ..Default::default()
        };
        assert!(t.validate().is_err());
        t.timeout_prevote_ms = 20_000;
        assert!(t.validate().is_err());
    }

    #[test]
    fn toml_sections_parse() {
        let t: ConsensusTimeouts = toml::from_str("block_time_ms = 1000").unwrap();
        assert_eq!(t.block_time_ms, 1000);
        assert_eq!(t.timeout_propose_ms, 1000);
        let p: P2pConfig = toml::from_str(
            r#"listen = "/ip4/127.0.0.1/tcp/1"
               persistent_peers = ["/dns4/val2/tcp/30333"]"#,
        )
        .unwrap();
        assert_eq!(p.persistent_peers.len(), 1);
        assert!(!p.discovery);
        // Unknown keys are ignored, like every other node config section.
        assert_eq!(
            toml::from_str::<P2pConfig>("bogus = 1").unwrap(),
            P2pConfig::default()
        );
        assert_eq!(
            toml::from_str::<P2pConfig>(&toml::to_string(&p).unwrap()).unwrap(),
            p
        );
    }

    #[test]
    fn engine_paths_and_channels() {
        let c = EngineConfig::new(7117, "/data");
        assert!(c.wal_path().ends_with("consensus/wal"));
        assert!(c.consensus_db_path().ends_with("consensus/consensus.redb"));
        assert_eq!(c.channel_name("consensus"), "/xdevs/7117/consensus");
        assert!(c.value_sync);
    }
}

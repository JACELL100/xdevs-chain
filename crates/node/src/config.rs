//! `config.toml` (plan §33.2, Phase-1 subset). Every section and field is optional;
//! keys outside the subset (`[p2p]`, `[validator]`, `[metrics]`, ...) are ignored.
//!
//! ```toml
//! [chain]
//! genesis = "genesis.json"     # omit in dev mode to use the built-in dev genesis
//! data_dir = ".xdevs/data"
//! pruning = "full"            # full (10,000 blocks of state history) | minimal (128) | archive
//! # state_history_blocks = 10_000  # optional: overrides the pruning mode's window
//!
//! [dev]
//! enabled = true
//! block_time_ms = 2000
//! fee_recipient = "0x0000000000000000000000000000000000000000"
//!
//! [rpc]
//! http = "127.0.0.1:8545"
//! ws = "127.0.0.1:8546"        # optional: extra listener (the http port also speaks WS)
//! cors = ["*"]
//! gas_cap = 50_000_000
//! max_logs_range = 10_000
//! max_batch = 50
//!
//! [mempool]
//! max_txs = 10_000
//! max_bytes = 67_108_864
//! max_per_sender = 64
//!
//! [log]
//! level = "info"
//! format = "text"   # text | json
//! ```

use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};
use xdevs_primitives::Address;

/// Full node configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeConfig {
    /// `[chain]`.
    pub chain: ChainSection,
    /// `[dev]` single-node block producer.
    pub dev: DevSection,
    /// `[rpc]`.
    pub rpc: RpcSection,
    /// `[mempool]`.
    pub mempool: MempoolSection,
    /// `[log]`.
    pub log: LogSection,
}

impl NodeConfig {
    /// Parse a TOML string.
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Read a TOML file.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        Self::from_toml_str(&std::fs::read_to_string(path)?)
    }

    /// Serialize to TOML.
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

/// `[chain]` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChainSection {
    /// Path of `genesis.json`; `None` = built-in dev genesis.
    pub genesis: Option<PathBuf>,
    /// Database directory.
    pub data_dir: PathBuf,
    /// `full` | `minimal` | `archive`. Blocks and receipts are always kept in Phase 1;
    /// the mode sets how many past blocks' state stays readable (see
    /// [`ChainSection::state_history_blocks`]).
    pub pruning: String,
    /// Explicit state-history window in blocks (overrides `pruning`).
    pub state_history_blocks: Option<u64>,
}

/// State-history window of `pruning = "minimal"`.
pub const MINIMAL_STATE_HISTORY_BLOCKS: u64 = 128;

impl ChainSection {
    /// Number of past blocks whose state can be queried (`eth_getBalance`, `eth_call`,
    /// ... at an older block): `state_history_blocks` if set, else `full` = 10,000
    /// (plan §14.4), `minimal` = 128, `archive` = unlimited (`u64::MAX`). At least 1.
    pub fn state_history_blocks(&self) -> anyhow::Result<u64> {
        if let Some(n) = self.state_history_blocks {
            return Ok(n.max(1));
        }
        match self.pruning.as_str() {
            "full" => Ok(xdevs_state::history::DEFAULT_STATE_HISTORY_BLOCKS),
            "minimal" => Ok(MINIMAL_STATE_HISTORY_BLOCKS),
            "archive" => Ok(u64::MAX),
            other => anyhow::bail!(
                "unknown [chain] pruning mode {other:?} (expected full, minimal or archive)"
            ),
        }
    }
}

impl Default for ChainSection {
    fn default() -> Self {
        Self {
            genesis: None,
            data_dir: PathBuf::from(".xdevs/data"),
            pruning: "full".into(),
            state_history_blocks: None,
        }
    }
}

/// `[dev]` section: single-validator block producer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevSection {
    /// Produce blocks locally.
    pub enabled: bool,
    /// Block interval in ms (`None` = chain config `blockTimeMs`).
    pub block_time_ms: Option<u64>,
    /// Beneficiary of produced blocks (receives tips).
    pub fee_recipient: Address,
}

impl Default for DevSection {
    fn default() -> Self {
        Self {
            enabled: true,
            block_time_ms: None,
            fee_recipient: Address::ZERO,
        }
    }
}

/// `[rpc]` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcSection {
    /// HTTP listen address (also accepts WebSocket upgrades).
    pub http: SocketAddr,
    /// Optional separate WebSocket listen address (`None` = WS only on `http`).
    pub ws: Option<SocketAddr>,
    /// Allowed CORS origins (`["*"]` = any, empty = no CORS headers).
    pub cors: Vec<String>,
    /// Enabled namespaces.
    pub namespaces: Vec<String>,
    /// `eth_call` / `eth_estimateGas` gas cap.
    pub gas_cap: u64,
    /// Max `eth_getLogs` block range.
    pub max_logs_range: u64,
    /// Max JSON-RPC batch size.
    pub max_batch: u32,
}

impl Default for RpcSection {
    fn default() -> Self {
        Self {
            http: SocketAddr::from(([127, 0, 0, 1], 8545)),
            ws: None,
            cors: vec!["*".into()],
            namespaces: vec!["eth".into(), "net".into(), "web3".into()],
            gas_cap: 50_000_000,
            max_logs_range: 10_000,
            max_batch: 50,
        }
    }
}

/// `[mempool]` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MempoolSection {
    /// Max pooled transactions.
    pub max_txs: usize,
    /// Max pooled bytes.
    pub max_bytes: usize,
    /// Max transactions per sender.
    pub max_per_sender: usize,
}

impl Default for MempoolSection {
    fn default() -> Self {
        Self {
            max_txs: 10_000,
            max_bytes: 64 * 1024 * 1024,
            max_per_sender: 64,
        }
    }
}

/// `[log]` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogSection {
    /// `tracing` filter (e.g. `info`, `xdevs_rpc=debug`).
    pub level: String,
    /// `text` or `json`.
    pub format: String,
}

impl Default for LogSection {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: "text".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_default() {
        assert_eq!(
            NodeConfig::from_toml_str("").unwrap(),
            NodeConfig::default()
        );
    }

    #[test]
    fn parses_sample_and_roundtrips() {
        let s = r#"
            [chain]
            genesis = "genesis.json"
            data_dir = "/tmp/xdevs"
            pruning = "archive"

            [dev]
            block_time_ms = 1000
            fee_recipient = "0x00000000000000000000000000000000000000aa"

            [rpc]
            http = "0.0.0.0:9545"
            gas_cap = 1_000_000

            [mempool]
            max_per_sender = 16

            [log]
            level = "debug"
        "#;
        let c = NodeConfig::from_toml_str(s).unwrap();
        assert_eq!(c.chain.genesis, Some(PathBuf::from("genesis.json")));
        assert_eq!(c.dev.block_time_ms, Some(1000));
        assert_eq!(c.rpc.http.port(), 9545);
        assert_eq!(c.rpc.gas_cap, 1_000_000);
        assert_eq!(c.mempool.max_per_sender, 16);
        assert_eq!(c.mempool.max_txs, 10_000);
        let again = NodeConfig::from_toml_str(&c.to_toml_string().unwrap()).unwrap();
        assert_eq!(again, c);
        assert_eq!(c.chain.state_history_blocks().unwrap(), u64::MAX);
    }

    #[test]
    fn state_history_window_from_pruning_mode() {
        let mut c = ChainSection::default();
        assert_eq!(c.state_history_blocks().unwrap(), 10_000);
        c.pruning = "minimal".into();
        assert_eq!(
            c.state_history_blocks().unwrap(),
            MINIMAL_STATE_HISTORY_BLOCKS
        );
        c.pruning = "bogus".into();
        assert!(c.state_history_blocks().is_err());
        c.state_history_blocks = Some(0);
        assert_eq!(c.state_history_blocks().unwrap(), 1);
        let parsed = NodeConfig::from_toml_str("[chain]\nstate_history_blocks = 256").unwrap();
        assert_eq!(parsed.chain.state_history_blocks().unwrap(), 256);
    }

    #[test]
    fn plan_sections_outside_phase1_are_ignored() {
        let s = "[p2p]
max_peers = 50
[validator]
enabled = true
[metrics]
listen = \"x\"";
        assert_eq!(NodeConfig::from_toml_str(s).unwrap(), NodeConfig::default());
    }

    #[test]
    fn rpc_ws_and_cors_parse() {
        let c = NodeConfig::from_toml_str(
            "[rpc]
ws = \"127.0.0.1:8546\"
cors = [\"http://localhost:3000\"]",
        )
        .unwrap();
        assert_eq!(c.rpc.ws, Some(SocketAddr::from(([127, 0, 0, 1], 8546))));
        assert_eq!(c.rpc.cors, vec!["http://localhost:3000".to_string()]);
    }
}

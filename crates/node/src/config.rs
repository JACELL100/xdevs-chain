//! `config.toml` (plan §33.2, research doc §6.10). Every section and field is optional;
//! unknown keys and sections (`[metrics]`, ...) are ignored.
//!
//! ```toml
//! [chain]
//! genesis = "genesis.json"     # omit in dev mode to use the built-in dev genesis
//! data_dir = ".xdevs/data"
//! pruning = "full"            # full (10,000 blocks of state history) | minimal (128) | archive
//! # state_history_blocks = 10_000  # optional: overrides the pruning mode's window
//!
//! [dev]                        # single-node producer; only for a genesis without validators
//! enabled = true
//! block_time_ms = 2000
//! fee_recipient = "0x0000000000000000000000000000000000000000"
//!
//! [validator]                  # absent / disabled => full node (genesis with validators)
//! enabled = true
//! key_file = "/var/lib/xdevs/keys/consensus.json"   # Ed25519 seed: keystore v3 or hex
//! password_file = "/var/lib/xdevs/keys/password"    # optional (else $XDEVS_KEY_PASSWORD)
//! fee_recipient = "0x..."
//! max_timestamp_drift_secs = 2
//!
//! [consensus]                  # plan §11.2 defaults
//! block_time_ms = 2000
//! timeout_propose_ms = 1000
//! timeout_propose_delta_ms = 500
//! timeout_prevote_ms = 500
//! timeout_prevote_delta_ms = 250
//! timeout_precommit_ms = 500
//! timeout_precommit_delta_ms = 250
//! max_timeout_ms = 10000
//!
//! [p2p]                        # Malachite consensus network
//! listen = "/ip4/0.0.0.0/tcp/30333"
//! persistent_peers = ["/dns4/val2/tcp/30333"]
//! node_key_file = "/var/lib/xdevs/keys/node_key.json"   # omit = ephemeral identity
//! discovery = false
//!
//! [gossip]                     # tx gossip (xdevs-network)
//! enabled = true
//! listen = "/ip4/0.0.0.0/tcp/30334"
//! bootnodes = []
//! max_peers = 50
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
//!
//! The run mode follows from the genesis and these sections, see [`NodeConfig::mode`].

use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};
use xdevs_consensus::{ConsensusTimeouts, P2pConfig};
use xdevs_network::NetworkConfig;
use xdevs_primitives::Address;

/// Full node configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeConfig {
    /// `[chain]`.
    pub chain: ChainSection,
    /// `[dev]` single-node block producer.
    pub dev: DevSection,
    /// `[validator]`: sign as a consensus validator (Phase 2).
    pub validator: ValidatorSection,
    /// `[consensus]`: block time and round timeouts (Phase 2).
    pub consensus: ConsensusTimeouts,
    /// `[p2p]`: Malachite consensus network (Phase 2).
    pub p2p: P2pConfig,
    /// `[gossip]`: tx gossip network (Phase 2).
    pub gossip: GossipSection,
    /// `[rpc]`.
    pub rpc: RpcSection,
    /// `[mempool]`.
    pub mempool: MempoolSection,
    /// `[log]`.
    pub log: LogSection,
    /// `[metrics]`: Prometheus `/metrics` endpoint (plan §24.7).
    pub metrics: xdevs_metrics::MetricsConfig,
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

    /// How the node runs, given whether the genesis has a validator set:
    ///
    /// * genesis **with** validators: [`NodeMode::Validator`] if `[validator] enabled`,
    ///   else [`NodeMode::FullNode`]. `[dev]` is ignored: such a chain only grows through
    ///   consensus, so a node can never extend it on its own.
    /// * genesis **without** validators: [`NodeMode::Dev`] if `[dev] enabled`, else
    ///   [`NodeMode::RpcOnly`]; `[validator] enabled` is an error.
    pub fn mode(&self, genesis_has_validators: bool) -> anyhow::Result<NodeMode> {
        Ok(match (genesis_has_validators, self.validator.enabled) {
            (true, true) => NodeMode::Validator,
            (true, false) => NodeMode::FullNode,
            (false, true) => anyhow::bail!(
                "[validator] enabled = true, but the genesis has no validators \
                 (use a network genesis, or [dev] mode for a single-node chain)"
            ),
            (false, false) if self.dev.enabled => NodeMode::Dev,
            (false, false) => NodeMode::RpcOnly,
        })
    }

    /// Sanity checks of the sections (run by `Node::new`).
    pub fn validate(&self) -> anyhow::Result<()> {
        self.chain.state_history_blocks()?;
        self.consensus
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid [consensus] section: {e}"))?;
        if self.gossip.enabled {
            self.gossip
                .network
                .validate()
                .map_err(|e| anyhow::anyhow!("invalid [gossip] section: {e}"))?;
        }
        Ok(())
    }
}

/// How a node runs (see [`NodeConfig::mode`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeMode {
    /// Phase-1 single-node dev chain: a local producer seals a block every tick.
    Dev,
    /// Consensus validator: proposes, re-executes and votes (needs a consensus key).
    Validator,
    /// Consensus follower: validates and commits decided blocks, never signs.
    FullNode,
    /// No block production at all: serves the stored chain over RPC.
    RpcOnly,
}

impl NodeMode {
    /// `true` for the modes that run the consensus engine.
    pub const fn runs_consensus(self) -> bool {
        matches!(self, Self::Validator | Self::FullNode)
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

/// `[validator]` section (research doc §6.10).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidatorSection {
    /// Sign proposals and votes with the consensus key.
    pub enabled: bool,
    /// Ed25519 consensus key: a keystore v3 JSON around the 32-byte seed, or the seed as
    /// hex text.
    pub key_file: Option<PathBuf>,
    /// File holding the keystore password (else `$XDEVS_KEY_PASSWORD`, else empty).
    pub password_file: Option<PathBuf>,
    /// Beneficiary of proposed blocks (receives priority fees).
    pub fee_recipient: Address,
    /// Max seconds a live proposal's timestamp may run ahead of the local clock
    /// (research doc §6.3 rule 3; not applied to synced blocks).
    pub max_timestamp_drift_secs: u64,
}

/// Default of [`ValidatorSection::max_timestamp_drift_secs`].
pub const DEFAULT_MAX_TIMESTAMP_DRIFT_SECS: u64 = 2;

impl Default for ValidatorSection {
    fn default() -> Self {
        Self {
            enabled: false,
            key_file: None,
            password_file: None,
            fee_recipient: Address::ZERO,
            max_timestamp_drift_secs: DEFAULT_MAX_TIMESTAMP_DRIFT_SECS,
        }
    }
}

/// `[gossip]` section: the tx-gossip swarm ([`NetworkConfig`] plus an on/off switch).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GossipSection {
    /// Run tx gossip in validator / full-node mode (ignored in the other modes).
    pub enabled: bool,
    /// Swarm settings (`listen`, `bootnodes`, `max_peers`, ...).
    #[serde(flatten)]
    pub network: NetworkConfig,
}

impl Default for GossipSection {
    fn default() -> Self {
        Self {
            enabled: true,
            network: NetworkConfig::default(),
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
    fn unknown_sections_and_keys_are_ignored() {
        let s = "[p2p]
max_peers = 50
[future_section]
listen = \"x\"";
        assert_eq!(NodeConfig::from_toml_str(s).unwrap(), NodeConfig::default());
    }

    #[test]
    fn phase2_sections_parse_and_roundtrip() {
        let s = r#"
            [validator]
            enabled = true
            key_file = "keys/consensus.json"
            fee_recipient = "0x00000000000000000000000000000000000000bb"

            [consensus]
            block_time_ms = 1000

            [p2p]
            listen = "/ip4/127.0.0.1/tcp/40000"
            persistent_peers = ["/ip4/127.0.0.1/tcp/40001"]

            [gossip]
            enabled = false
            listen = "/ip4/127.0.0.1/tcp/40010"
            max_peers = 7
        "#;
        let c = NodeConfig::from_toml_str(s).unwrap();
        assert!(c.validator.enabled);
        assert_eq!(
            c.validator.key_file,
            Some(PathBuf::from("keys/consensus.json"))
        );
        assert_eq!(c.validator.max_timestamp_drift_secs, 2);
        assert_eq!(c.consensus.block_time_ms, 1000);
        assert_eq!(c.consensus.timeout_propose_ms, 1000);
        assert_eq!(c.p2p.persistent_peers.len(), 1);
        assert!(!c.gossip.enabled);
        assert_eq!(c.gossip.network.listen, "/ip4/127.0.0.1/tcp/40010");
        assert_eq!(c.gossip.network.max_peers, 7);
        assert_eq!(c.gossip.network.channel_capacity, 1024);
        c.validate().unwrap();
        let again = NodeConfig::from_toml_str(&c.to_toml_string().unwrap()).unwrap();
        assert_eq!(again, c);

        let d = NodeConfig::default();
        assert!(d.gossip.enabled && !d.validator.enabled);
        d.validate().unwrap();
    }

    #[test]
    fn invalid_phase2_sections_are_rejected() {
        let c = NodeConfig::from_toml_str("[consensus]\ntimeout_prevote_ms = 0").unwrap();
        assert!(c.validate().is_err());
        let c = NodeConfig::from_toml_str("[gossip]\nlisten = \"not an addr\"").unwrap();
        assert!(c.validate().is_err());
        // A disabled gossip section is not checked.
        let c = NodeConfig::from_toml_str("[gossip]\nenabled = false\nlisten = \"x\"").unwrap();
        c.validate().unwrap();
    }

    #[test]
    fn run_mode_follows_genesis_and_sections() {
        let mut c = NodeConfig::default();
        assert_eq!(c.mode(false).unwrap(), NodeMode::Dev);
        assert_eq!(
            c.mode(true).unwrap(),
            NodeMode::FullNode,
            "[dev] is ignored"
        );
        c.dev.enabled = false;
        assert_eq!(c.mode(false).unwrap(), NodeMode::RpcOnly);
        assert_eq!(c.mode(true).unwrap(), NodeMode::FullNode);
        c.validator.enabled = true;
        assert_eq!(c.mode(true).unwrap(), NodeMode::Validator);
        assert!(c.mode(false).is_err());
        assert!(NodeMode::Validator.runs_consensus() && NodeMode::FullNode.runs_consensus());
        assert!(!NodeMode::Dev.runs_consensus() && !NodeMode::RpcOnly.runs_consensus());
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

//! [`MalachiteEngine`]: the [`ConsensusEngine`] implementation (WP1). Its public surface
//! has no Malachite types; the work happens in the private `malachite` module.

use crate::{
    app::{ConsensusApp, ConsensusEngine},
    config::EngineConfig,
};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};
use zeroize::Zeroizing;

/// Live status of a running engine, for the node's RPC (`eth_syncing`, `net_peerCount`).
/// Cheap to clone; every clone sees the same values. All zero / `false` until
/// [`ConsensusEngine::run`] starts the engine.
#[derive(Clone, Debug, Default)]
pub struct EngineStatus(Arc<StatusInner>);

#[derive(Debug, Default)]
struct StatusInner {
    running: AtomicBool,
    height: AtomicU64,
    highest_seen: AtomicU64,
    peers: AtomicUsize,
}

impl EngineStatus {
    /// `true` while the engine runs.
    pub fn is_running(&self) -> bool {
        self.0.running.load(Ordering::Relaxed)
    }

    /// The height consensus is working on (head + 1 once started).
    pub fn height(&self) -> u64 {
        self.0.height.load(Ordering::Relaxed)
    }

    /// Highest height of any consensus message (vote or proposal) received from a peer.
    /// Peers voting at `h` have decided `h - 1`. Not signature-checked: informational
    /// (sync progress), never used for safety.
    pub fn highest_seen_height(&self) -> u64 {
        self.0.highest_seen.load(Ordering::Relaxed)
    }

    /// Connected consensus-network peers.
    pub fn peer_count(&self) -> usize {
        self.0.peers.load(Ordering::Relaxed)
    }

    pub(crate) fn set_running(&self, running: bool) {
        self.0.running.store(running, Ordering::Relaxed);
    }

    pub(crate) fn set_height(&self, height: u64) {
        self.0.height.store(height, Ordering::Relaxed);
        self.observe_height(height);
    }

    pub(crate) fn observe_height(&self, height: u64) {
        self.0.highest_seen.fetch_max(height, Ordering::Relaxed);
    }

    pub(crate) fn set_peer_count(&self, peers: usize) {
        self.0.peers.store(peers, Ordering::Relaxed);
    }
}

/// Secret keys of a node. Debug output never shows key material.
pub struct EngineKeys {
    /// Ed25519 consensus key seed; `None` = full node (follows and syncs, never signs).
    pub consensus_seed: Option<Zeroizing<[u8; 32]>>,
    /// libp2p Ed25519 node identity seed for Malachite's consensus network.
    pub node_key_seed: Zeroizing<[u8; 32]>,
}

impl EngineKeys {
    /// Keys of a validator.
    pub fn validator(consensus_seed: [u8; 32], node_key_seed: [u8; 32]) -> Self {
        Self {
            consensus_seed: Some(Zeroizing::new(consensus_seed)),
            node_key_seed: Zeroizing::new(node_key_seed),
        }
    }

    /// Keys of a non-validating full node.
    pub fn full_node(node_key_seed: [u8; 32]) -> Self {
        Self {
            consensus_seed: None,
            node_key_seed: Zeroizing::new(node_key_seed),
        }
    }

    /// `true` if this node signs votes and proposals.
    pub fn is_validator(&self) -> bool {
        self.consensus_seed.is_some()
    }

    /// Ed25519 consensus public key, if a validator.
    pub fn consensus_public_key(&self) -> Option<crate::types::ValidatorKey> {
        self.consensus_seed.as_ref().map(|seed| {
            crate::types::ValidatorKey(*crate::malachite::private_key(seed).public_key().as_bytes())
        })
    }
}

impl fmt::Debug for EngineKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineKeys")
            .field("validator", &self.is_validator())
            .finish_non_exhaustive()
    }
}

/// Tendermint BFT through Malachite 0.8.2 (ADR-0002), driving a [`ConsensusApp`].
#[derive(Debug)]
pub struct MalachiteEngine {
    config: EngineConfig,
    keys: EngineKeys,
    status: EngineStatus,
}

impl MalachiteEngine {
    /// An engine with `config` and `keys`; nothing starts until [`ConsensusEngine::run`].
    pub fn new(config: EngineConfig, keys: EngineKeys) -> Self {
        Self {
            config,
            keys,
            status: EngineStatus::default(),
        }
    }

    /// Live status handle (clone it before [`ConsensusEngine::run`] consumes the engine).
    pub fn status(&self) -> EngineStatus {
        self.status.clone()
    }

    /// The engine configuration.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The node's keys.
    pub fn keys(&self) -> &EngineKeys {
        &self.keys
    }
}

#[async_trait::async_trait]
impl ConsensusEngine for MalachiteEngine {
    async fn run(
        self,
        app: Arc<dyn ConsensusApp>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        crate::malachite::engine::run(self.config, self.keys, app, shutdown, self.status).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_debug_hides_secrets_and_derives_public_key() {
        let k = EngineKeys::validator([7u8; 32], [8u8; 32]);
        assert!(k.is_validator());
        let dbg = format!("{k:?}");
        assert!(!dbg.contains("7, 7"), "{dbg}");
        let pk = k.consensus_public_key().unwrap();
        let expected = ed25519_consensus::SigningKey::from([7u8; 32])
            .verification_key()
            .to_bytes();
        assert_eq!(pk.0, expected);
        assert!(EngineKeys::full_node([1; 32])
            .consensus_public_key()
            .is_none());
    }

    #[test]
    fn engine_is_a_consensus_engine() {
        fn assert_engine<E: ConsensusEngine>(_: &E) {}
        let e = MalachiteEngine::new(
            EngineConfig::new(1, "/tmp/x"),
            EngineKeys::full_node([0; 32]),
        );
        assert_engine(&e);
        assert_eq!(e.config().chain_id, 1);
        assert!(!e.keys().is_validator());
    }
}

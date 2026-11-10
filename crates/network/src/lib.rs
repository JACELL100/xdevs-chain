//! P2P networking for xDEVS Chain outside consensus (research doc §3.4, §6.11, plan §12).
//!
//! Consensus, proposal and sync traffic use Malachite's own libp2p network inside
//! `xdevs-consensus`. This crate runs a second, small libp2p **0.56** swarm (the same
//! version as Malachite's, so only one libp2p stack is compiled) for what Malachite lacks:
//! **transaction gossip** now, evidence gossip in Phase 3.
//!
//! * Topic `/xdevs/<chainId>/tx/1` (gossipsub, strict validation, message id =
//!   keccak256(raw tx)).
//! * Inbound txs are checked by the caller's [`TxValidator`] callback (the node decodes,
//!   recovers and adds to the pool); only [`Verdict::Accept`] messages are forwarded.
//! * Every channel is bounded (plan §12.5).
//!
//! **Owner: WP3.** The public API below is frozen on day 0; change it only additively.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod handle;
mod swarm;

pub use config::NetworkConfig;
pub use handle::{GossipStats, NetworkHandle};
pub use libp2p::{identity::Keypair, Multiaddr, PeerId};
pub use swarm::{tx_message_id, tx_topic};

use std::sync::Arc;

/// Max size of one gossiped transaction (plan §12.2): 128 KiB.
pub const MAX_TX_SIZE: usize = 128 * 1024;

/// The node's verdict on an inbound gossiped transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Valid (added to the pool, or already known): deliver and forward.
    Accept,
    /// Not forwarded, no penalty (e.g. pool full, nonce too far ahead).
    Ignore,
    /// Invalid (bad encoding, signature or chain id): drop and penalise the sender.
    Reject,
}

/// Validation callback for inbound txs: raw EIP-2718 bytes → verdict. Called on the
/// network task, so it must be fast and must not block.
pub type TxValidator = Arc<dyn Fn(&[u8]) -> Verdict + Send + Sync + 'static>;

/// Network errors.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// Invalid configuration (bad multiaddr, zero limits…).
    #[error("invalid network config: {0}")]
    Config(String),
    /// The outbound queue is full; the tx was not published.
    #[error("network queue full")]
    Busy,
    /// The network task has stopped.
    #[error("network stopped")]
    Closed,
    /// A tx larger than [`MAX_TX_SIZE`] (or the configured cap).
    #[error("transaction too large: {0} bytes")]
    TooLarge(usize),
    /// libp2p transport / behaviour failure.
    #[error("libp2p: {0}")]
    Libp2p(String),
}

/// Start the tx-gossip swarm for `chain_id` with identity `keypair` and return its handle.
pub async fn start(
    config: NetworkConfig,
    chain_id: u64,
    keypair: Keypair,
    validator: TxValidator,
) -> Result<NetworkHandle, NetworkError> {
    config.validate()?;
    swarm::spawn(config, chain_id, keypair, validator).await
}

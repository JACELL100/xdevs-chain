//! `[gossip]` node config section (research doc §6.10).

use crate::{NetworkError, MAX_TX_SIZE};
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

/// Default of [`NetworkConfig::tx_rate_per_peer`]: above the chain's peak throughput
/// (30M gas per 2 s block = ~714 transfers/s), since a single upstream peer may be the
/// first to deliver almost every tx.
pub const DEFAULT_TX_RATE_PER_PEER: u32 = 1_000;
/// Default of [`NetworkConfig::tx_burst_per_peer`].
pub const DEFAULT_TX_BURST_PER_PEER: u32 = 4_000;

/// `[gossip]`: the tx-gossip swarm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Listen multiaddr (TCP by default; QUIC behind [`NetworkConfig::quic`]).
    pub listen: String,
    /// Peers dialled at start (multiaddrs, usually ending in `/p2p/<peer id>`).
    pub bootnodes: Vec<String>,
    /// Max connected peers (plan §12.5).
    pub max_peers: usize,
    /// Max peers that dialled us (`None` = `max_peers` minus a reserve of
    /// max(`max_peers` / 5, bootnodes, 1)). The rest stays free for our own dials
    /// (bootnodes), so inbound connections alone can never fill every slot.
    pub max_inbound_peers: Option<usize>,
    /// Inbound txs per second one peer may send (token bucket, plan §12.5; 0 = no
    /// limit). Every message counts, whatever its verdict; an ignored tx costs
    /// extra. Messages over the budget are dropped unvalidated, and a peer that keeps
    /// sending over the budget is banned.
    pub tx_rate_per_peer: u32,
    /// Bucket size (burst) of the per-peer tx rate limit.
    pub tx_burst_per_peer: u32,
    /// Max size of one gossiped tx; at most [`MAX_TX_SIZE`].
    pub max_tx_size: usize,
    /// Also listen on QUIC (off until a Linux soak, research doc §3.4).
    pub quic: bool,
    /// Kademlia discovery (off in Phase 2).
    pub discovery: bool,
    /// Capacity of every internal channel (outbound queue, each inbound subscriber).
    pub channel_capacity: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen: "/ip4/0.0.0.0/tcp/30334".into(),
            bootnodes: Vec::new(),
            max_peers: 50,
            max_inbound_peers: None,
            tx_rate_per_peer: DEFAULT_TX_RATE_PER_PEER,
            tx_burst_per_peer: DEFAULT_TX_BURST_PER_PEER,
            max_tx_size: MAX_TX_SIZE,
            quic: false,
            discovery: false,
            channel_capacity: 1024,
        }
    }
}

impl NetworkConfig {
    /// Parsed listen address.
    pub fn listen_addr(&self) -> Result<Multiaddr, NetworkError> {
        parse_addr("listen", &self.listen)
    }

    /// Parsed bootnode addresses.
    pub fn bootnode_addrs(&self) -> Result<Vec<Multiaddr>, NetworkError> {
        self.bootnodes
            .iter()
            .map(|b| parse_addr("bootnodes", b))
            .collect()
    }

    /// Max inbound peers (see [`NetworkConfig::max_inbound_peers`]).
    pub fn inbound_limit(&self) -> usize {
        match self.max_inbound_peers {
            Some(n) => n.min(self.max_peers),
            None => {
                let reserve = (self.max_peers / 5).max(self.bootnodes.len()).max(1);
                self.max_peers.saturating_sub(reserve).max(1)
            }
        }
    }

    /// Check addresses and limits.
    pub fn validate(&self) -> Result<(), NetworkError> {
        self.listen_addr()?;
        self.bootnode_addrs()?;
        if self.max_peers == 0 || self.channel_capacity == 0 {
            return Err(NetworkError::Config(
                "max_peers and channel_capacity must be > 0".into(),
            ));
        }
        if self.max_inbound_peers.is_some_and(|n| n > self.max_peers) {
            return Err(NetworkError::Config(
                "max_inbound_peers must be <= max_peers".into(),
            ));
        }
        if self.tx_rate_per_peer > 0 && self.tx_burst_per_peer == 0 {
            return Err(NetworkError::Config(
                "tx_burst_per_peer must be > 0 when tx_rate_per_peer is set".into(),
            ));
        }
        if is_quic(&self.listen_addr()?) && !self.quic {
            return Err(NetworkError::Config(
                "gossip.listen is a QUIC address but gossip.quic = false".into(),
            ));
        }
        if self.max_tx_size == 0 || self.max_tx_size > MAX_TX_SIZE {
            return Err(NetworkError::Config(format!(
                "max_tx_size must be in 1..={MAX_TX_SIZE}"
            )));
        }
        Ok(())
    }
}

/// Whether `addr` is a QUIC address (`.../udp/<port>/quic-v1`).
pub(crate) fn is_quic(addr: &Multiaddr) -> bool {
    addr.iter()
        .any(|p| matches!(p, libp2p::multiaddr::Protocol::QuicV1))
}

fn parse_addr(field: &str, s: &str) -> Result<Multiaddr, NetworkError> {
    s.parse()
        .map_err(|e| NetworkError::Config(format!("gossip.{field}: {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let c = NetworkConfig::default();
        c.validate().unwrap();
        assert_eq!(c.max_peers, 50);
        assert_eq!(c.max_tx_size, 128 * 1024);
        assert!(!c.quic && !c.discovery);
    }

    #[test]
    fn rejects_bad_values() {
        let bad = |f: &dyn Fn(&mut NetworkConfig)| {
            let mut c = NetworkConfig::default();
            f(&mut c);
            assert!(matches!(c.validate(), Err(NetworkError::Config(_))));
        };
        bad(&|c| c.listen = "not an addr".into());
        bad(&|c| c.bootnodes = vec!["/ip4/1.2.3.4/tcp/x".into()]);
        bad(&|c| c.max_peers = 0);
        bad(&|c| c.channel_capacity = 0);
        bad(&|c| c.max_tx_size = MAX_TX_SIZE + 1);
        bad(&|c| c.max_tx_size = 0);
        bad(&|c| c.listen = "/ip4/127.0.0.1/udp/1/quic-v1".into());
        bad(&|c| c.max_inbound_peers = Some(51));
        bad(&|c| c.tx_burst_per_peer = 0);
    }

    /// Regression (security review): inbound connections alone can never take every
    /// slot; room stays for our own dials (bootnodes).
    #[test]
    fn inbound_limit_keeps_room_for_outbound() {
        let mut c = NetworkConfig::default();
        assert_eq!(c.inbound_limit(), 40);
        c.bootnodes = (0..12).map(|i| format!("/ip4/10.0.0.{i}/tcp/1")).collect();
        assert_eq!(c.inbound_limit(), 38);
        c.max_peers = 4;
        c.bootnodes.truncate(1);
        assert_eq!(c.inbound_limit(), 3);
        c.max_peers = 1;
        assert_eq!(c.inbound_limit(), 1);
        c.max_peers = 10;
        c.max_inbound_peers = Some(2);
        assert_eq!(c.inbound_limit(), 2);
    }

    #[test]
    fn quic_detection() {
        assert!(is_quic(&"/ip4/1.2.3.4/udp/9/quic-v1".parse().unwrap()));
        assert!(!is_quic(&"/ip4/1.2.3.4/tcp/9".parse().unwrap()));
        let c = NetworkConfig {
            listen: "/ip4/127.0.0.1/udp/0/quic-v1".into(),
            quic: true,
            ..NetworkConfig::default()
        };
        c.validate().unwrap();
    }

    #[test]
    fn toml_roundtrip() {
        let c: NetworkConfig = toml::from_str(
            r#"listen = "/ip4/127.0.0.1/tcp/0"
               bootnodes = ["/dns4/val1/tcp/30334"]
               max_peers = 8"#,
        )
        .unwrap();
        assert_eq!(c.max_peers, 8);
        assert_eq!(c.bootnode_addrs().unwrap().len(), 1);
        assert_eq!(
            toml::from_str::<NetworkConfig>(&toml::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
}

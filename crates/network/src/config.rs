//! `[gossip]` node config section (research doc §6.10).

use crate::{NetworkError, MAX_TX_SIZE};
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

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

    /// Check addresses and limits.
    pub fn validate(&self) -> Result<(), NetworkError> {
        self.listen_addr()?;
        self.bootnode_addrs()?;
        if self.max_peers == 0 || self.channel_capacity == 0 {
            return Err(NetworkError::Config(
                "max_peers and channel_capacity must be > 0".into(),
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

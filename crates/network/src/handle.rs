//! [`NetworkHandle`]: the node's view of the tx-gossip swarm (research doc §6.11).

use crate::{NetworkError, Verdict};
use libp2p::{Multiaddr, PeerId};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::{sync::mpsc, task::JoinHandle};
use xdevs_primitives::Bytes;

/// Commands from the handle to the swarm task.
#[derive(Debug)]
pub(crate) enum Command {
    /// Publish a raw tx on the tx topic.
    PublishTx(Bytes),
    /// Test hook: the gossipsub score of a peer.
    #[cfg(test)]
    PeerScore(PeerId, tokio::sync::oneshot::Sender<Option<f64>>),
    /// Stop the swarm task.
    Shutdown,
}

/// Counters of inbound gossip verdicts and bans (for metrics and tests).
#[derive(Debug, Default)]
pub struct GossipStats {
    /// Inbound txs accepted (delivered and forwarded).
    pub accepted: AtomicU64,
    /// Inbound txs ignored (not forwarded, no penalty).
    pub ignored: AtomicU64,
    /// Inbound txs rejected (not forwarded, sender penalised).
    pub rejected: AtomicU64,
    /// Peers banned for a graylisted gossip score.
    pub banned: AtomicU64,
}

impl GossipStats {
    pub(crate) fn record(&self, verdict: Verdict) {
        let counter = match verdict {
            Verdict::Accept => &self.accepted,
            Verdict::Ignore => &self.ignored,
            Verdict::Reject => &self.rejected,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// State shared between the handle and the swarm task.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    /// One bounded sender per [`NetworkHandle::inbound`] subscriber.
    subscribers: Mutex<Vec<mpsc::Sender<(PeerId, Bytes)>>>,
    /// Set once the swarm task stopped: new subscribers are closed at once.
    closed: Mutex<bool>,
    /// Current listen addresses.
    listen_addrs: Mutex<Vec<Multiaddr>>,
    /// Connected peers.
    pub(crate) peers: AtomicUsize,
    /// Connected peers subscribed to the tx topic.
    pub(crate) topic_peers: AtomicUsize,
    /// Verdict and ban counters.
    pub(crate) stats: GossipStats,
}

impl Shared {
    /// Hand an accepted inbound tx to every subscriber. Full subscribers miss it (bounded
    /// channels, plan §12.5); closed ones are dropped.
    pub(crate) fn deliver(&self, from: PeerId, tx: &Bytes) {
        let mut subs = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| match s.try_send((from, tx.clone())) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
    }

    /// Drop every subscriber (their streams end) and refuse new ones.
    pub(crate) fn close(&self) {
        *self.closed.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub(crate) fn add_listen_addr(&self, addr: Multiaddr) {
        let mut addrs = self.listen_addrs.lock().unwrap_or_else(|e| e.into_inner());
        if !addrs.contains(&addr) {
            addrs.push(addr);
        }
    }

    pub(crate) fn remove_listen_addr(&self, addr: &Multiaddr) {
        self.listen_addrs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|a| a != addr);
    }

    pub(crate) fn listen_addrs(&self) -> Vec<Multiaddr> {
        self.listen_addrs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Handle to a running tx-gossip swarm. Dropping it does not stop the swarm; call
/// [`NetworkHandle::shutdown`].
#[derive(Debug)]
pub struct NetworkHandle {
    local_peer_id: PeerId,
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    capacity: usize,
    max_tx_size: usize,
    task: JoinHandle<()>,
}

impl NetworkHandle {
    pub(crate) fn new(
        local_peer_id: PeerId,
        commands: mpsc::Sender<Command>,
        shared: Arc<Shared>,
        capacity: usize,
        max_tx_size: usize,
        task: JoinHandle<()>,
    ) -> Self {
        Self {
            local_peer_id,
            commands,
            shared,
            capacity,
            max_tx_size,
            task,
        }
    }

    /// This node's libp2p peer id.
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Queue a raw (EIP-2718) tx for gossip. Never blocks: fails with
    /// [`NetworkError::Busy`] when the outbound queue is full.
    pub fn publish_tx(&self, tx: Bytes) -> Result<(), NetworkError> {
        if tx.len() > self.max_tx_size {
            return Err(NetworkError::TooLarge(tx.len()));
        }
        self.commands
            .try_send(Command::PublishTx(tx))
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => NetworkError::Busy,
                mpsc::error::TrySendError::Closed(_) => NetworkError::Closed,
            })
    }

    /// A new bounded stream of accepted inbound txs `(from, raw tx)`. Each call creates
    /// an independent subscriber; a slow subscriber misses txs rather than blocking.
    /// The stream ends once the swarm stops.
    pub fn inbound(&self) -> mpsc::Receiver<(PeerId, Bytes)> {
        let (tx, rx) = mpsc::channel(self.capacity);
        // Holding `closed` while pushing means `Shared::close` cannot miss this subscriber.
        let closed = self.shared.closed.lock().unwrap_or_else(|e| e.into_inner());
        if !*closed {
            self.shared
                .subscribers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(tx);
        }
        rx
    }

    /// Number of connected gossip peers.
    pub fn peer_count(&self) -> usize {
        self.shared.peers.load(Ordering::Relaxed)
    }

    /// Number of connected peers subscribed to the tx topic.
    pub fn topic_peer_count(&self) -> usize {
        self.shared.topic_peers.load(Ordering::Relaxed)
    }

    /// The addresses the swarm listens on (resolved ports, e.g. for `tcp/0`).
    pub fn listen_addrs(&self) -> Vec<Multiaddr> {
        self.shared.listen_addrs()
    }

    /// Inbound verdict and ban counters.
    pub fn stats(&self) -> &GossipStats {
        &self.shared.stats
    }

    /// Test hook: the gossipsub score of `peer` (None if unknown or stopped).
    #[cfg(test)]
    pub(crate) async fn peer_score(&self, peer: PeerId) -> Option<f64> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.commands
            .send(Command::PeerScore(peer, tx))
            .await
            .ok()?;
        rx.await.ok().flatten()
    }

    /// Stop the swarm and wait for its task to finish.
    pub async fn shutdown(self) {
        let _ = self.commands.send(Command::Shutdown).await;
        let _ = self.task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Seen = Arc<Mutex<Vec<Bytes>>>;

    /// A handle whose "swarm" just records commands until `Shutdown`.
    fn fake(capacity: usize) -> (NetworkHandle, Arc<Shared>, Seen) {
        let (tx, mut rx) = mpsc::channel(capacity);
        let shared = Arc::new(Shared::default());
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let task = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    Command::PublishTx(b) => seen2.lock().unwrap().push(b),
                    Command::PeerScore(_, reply) => {
                        let _ = reply.send(None);
                    }
                    Command::Shutdown => break,
                }
            }
        });
        let h = NetworkHandle::new(PeerId::random(), tx, shared.clone(), capacity, 8, task);
        (h, shared, seen)
    }

    #[tokio::test]
    async fn publish_inbound_peers_shutdown() {
        let (h, shared, seen) = fake(2);
        assert!(matches!(
            h.publish_tx(Bytes::from(vec![0u8; 9])),
            Err(NetworkError::TooLarge(9))
        ));
        let mut a = h.inbound();
        let mut b = h.inbound();
        let peer = PeerId::random();
        shared.deliver(peer, &Bytes::from_static(b"tx1"));
        assert_eq!(a.recv().await.unwrap(), (peer, Bytes::from_static(b"tx1")));
        assert_eq!(b.recv().await.unwrap().1, Bytes::from_static(b"tx1"));
        // Bounded: a full subscriber misses txs but stays subscribed.
        for _ in 0..5 {
            shared.deliver(peer, &Bytes::from_static(b"x"));
        }
        assert_eq!(a.len(), 2);
        // Closed subscribers are pruned.
        drop(b);
        shared.deliver(peer, &Bytes::from_static(b"y"));
        assert_eq!(shared.subscribers.lock().unwrap().len(), 1);

        shared.peers.store(3, Ordering::Relaxed);
        assert_eq!(h.peer_count(), 3);
        h.publish_tx(Bytes::from_static(b"out")).unwrap();
        h.shutdown().await;
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[Bytes::from_static(b"out")]
        );
    }

    #[tokio::test]
    async fn publish_fails_when_full_or_closed() {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(async {});
        let h = NetworkHandle::new(PeerId::random(), tx, Arc::default(), 1, 64, task);
        h.publish_tx(Bytes::from_static(b"a")).unwrap();
        assert!(matches!(
            h.publish_tx(Bytes::from_static(b"b")),
            Err(NetworkError::Busy)
        ));
        drop(rx);
        assert!(matches!(
            h.publish_tx(Bytes::from_static(b"c")),
            Err(NetworkError::Closed)
        ));
        h.shutdown().await;
    }

    #[tokio::test]
    async fn close_ends_streams_and_refuses_new_subscribers() {
        let (h, shared, _) = fake(4);
        let mut a = h.inbound();
        shared.stats.record(Verdict::Reject);
        shared.stats.record(Verdict::Accept);
        assert_eq!(h.stats().rejected.load(Ordering::Relaxed), 1);
        assert_eq!(h.stats().accepted.load(Ordering::Relaxed), 1);
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/1".parse().unwrap();
        shared.add_listen_addr(addr.clone());
        shared.add_listen_addr(addr.clone());
        assert_eq!(h.listen_addrs(), vec![addr.clone()]);
        shared.remove_listen_addr(&addr);
        assert!(h.listen_addrs().is_empty());
        assert_eq!(h.peer_score(PeerId::random()).await, None);

        shared.close();
        assert!(a.recv().await.is_none());
        let mut b = h.inbound();
        assert!(b.recv().await.is_none());
        h.shutdown().await;
    }
}

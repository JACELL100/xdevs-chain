//! The libp2p 0.56 swarm task (WP3): tcp + noise + yamux (quic behind config), gossipsub
//! tx topic with strict validation, identify, ping, optional kad, bootnodes, peer limits and
//! a temporary ban list for peers whose gossip score falls below the graylist threshold.
//!
//! Gossipsub scores use `f64` internally; they only steer peer selection on the tx-gossip
//! network and never feed into consensus.

use crate::{
    config::is_quic,
    handle::{Command, NetworkHandle, Shared},
    NetworkConfig, NetworkError, TxValidator, Verdict,
};
use libp2p::{
    allow_block_list::{self, BlockedPeers},
    connection_limits::{self, ConnectionLimits},
    core::ConnectedPoint,
    futures::StreamExt,
    gossipsub::{
        self, IdentTopic, IdentityTransform, MessageAcceptance, MessageAuthenticity, MessageId,
        PeerScoreParams, PeerScoreThresholds, TopicHash, TopicScoreParams,
        WhitelistSubscriptionFilter,
    },
    identify,
    identity::Keypair,
    kad,
    multiaddr::Protocol,
    noise, ping,
    swarm::{
        behaviour::toggle::Toggle,
        dial_opts::{DialOpts, PeerCondition},
        NetworkBehaviour, SwarmEvent,
    },
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use std::{
    collections::{BTreeMap, HashSet},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use xdevs_primitives::{keccak256, Bytes};

/// Slack above the tx cap for one gossipsub message envelope (author, seqno, topic,
/// signature). A message whose *data* exceeds the tx cap is rejected explicitly anyway.
const MESSAGE_ENVELOPE: usize = 1024;
/// Slack above the tx cap for a whole RPC frame (control messages such as IHAVE / IWANT).
const FRAME_OVERHEAD: usize = 64 * 1024;
/// Max message ids per IHAVE (32-byte ids, so ~35 KiB: fits [`FRAME_OVERHEAD`]).
const MAX_IHAVE_LENGTH: usize = 1000;
/// Max established connections to one peer (2 tolerates simultaneous dials).
const MAX_CONNECTIONS_PER_PEER: u32 = 2;
/// Close connections idle for this long.
const IDLE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);
/// Period of the maintenance tick (bootnode redial, ban expiry, score sweep).
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(5);
/// Run a kad bootstrap every this many maintenance ticks (when discovery is on).
const KAD_BOOTSTRAP_TICKS: u32 = 6;
/// How long a peer stays banned after its score falls below the graylist threshold.
const BAN_DURATION: Duration = Duration::from_secs(600);
/// Ban list cap; the entry closest to expiry is evicted first.
const MAX_BANNED: usize = 1024;
/// How long `start` waits for the listeners to bind.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(10);
/// How long shutdown waits for peers to see our unsubscribe and disconnects.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

/// Score below which gossipsub ignores a peer entirely; we also ban it.
pub(crate) const GRAYLIST_THRESHOLD: f64 = -80.0;
/// P4 weight: score = weight × (invalid messages)², so three invalid txs graylist a peer.
const INVALID_MESSAGE_WEIGHT: f64 = -10.0;

/// The tx gossip topic: `/xdevs/<chainId>/tx/1` (plan §12.2).
pub fn tx_topic(chain_id: u64) -> IdentTopic {
    IdentTopic::new(format!("/xdevs/{chain_id}/tx/1"))
}

/// Gossipsub message id of a tx message: `keccak256(raw tx)`, so the same tx from
/// different publishers is deduplicated by the seen-cache.
pub fn tx_message_id(data: &[u8]) -> MessageId {
    MessageId::new(keccak256(data).as_slice())
}

impl From<Verdict> for MessageAcceptance {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Accept => MessageAcceptance::Accept,
            Verdict::Ignore => MessageAcceptance::Ignore,
            Verdict::Reject => MessageAcceptance::Reject,
        }
    }
}

/// Identify protocol version; peers announcing another one (another chain) are dropped.
fn identify_protocol(chain_id: u64) -> String {
    format!("/xdevs/{chain_id}/gossip/1")
}

/// Kademlia protocol name, per chain so chains never share a DHT.
fn kad_protocol(chain_id: u64) -> StreamProtocol {
    StreamProtocol::try_from_owned(format!("/xdevs/{chain_id}/kad/1"))
        .expect("protocol name starts with '/'")
}

type GossipBehaviour = gossipsub::Behaviour<IdentityTransform, WhitelistSubscriptionFilter>;

/// The composed behaviour. Limits and the ban list come first so they deny early.
#[derive(NetworkBehaviour)]
pub(crate) struct Behaviour {
    limits: connection_limits::Behaviour,
    blocked: allow_block_list::Behaviour<BlockedPeers>,
    gossipsub: GossipBehaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    kad: Toggle<kad::Behaviour<kad::store::MemoryStore>>,
}

/// Gossipsub config: strict signing, app-level validation, keccak message ids and size caps.
fn gossipsub_config(
    config: &NetworkConfig,
    topic: &TopicHash,
) -> Result<gossipsub::Config, String> {
    gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Strict)
        .validate_messages()
        .message_id_fn(|m: &gossipsub::Message| tx_message_id(&m.data))
        .max_transmit_size(config.max_tx_size + FRAME_OVERHEAD)
        .max_transmit_size_for_topic(config.max_tx_size + MESSAGE_ENVELOPE, topic.clone())
        .max_ihave_length(MAX_IHAVE_LENGTH)
        .heartbeat_interval(Duration::from_secs(1))
        .build()
        .map_err(|e| e.to_string())
}

/// Peer scoring: invalid txs (P4) are the main penalty; quiet peers are not punished
/// (P3/P3b off) because tx traffic is bursty.
fn peer_score(topic: &TopicHash) -> (PeerScoreParams, PeerScoreThresholds) {
    let tx_topic = TopicScoreParams {
        topic_weight: 1.0,
        time_in_mesh_weight: 0.01,
        time_in_mesh_quantum: Duration::from_secs(1),
        time_in_mesh_cap: 300.0,
        first_message_deliveries_weight: 1.0,
        first_message_deliveries_decay: 0.9,
        first_message_deliveries_cap: 50.0,
        mesh_message_deliveries_weight: 0.0,
        mesh_failure_penalty_weight: 0.0,
        invalid_message_deliveries_weight: INVALID_MESSAGE_WEIGHT,
        invalid_message_deliveries_decay: 0.99,
        ..TopicScoreParams::default()
    };
    let mut params = PeerScoreParams::default();
    params.topics.insert(topic.clone(), tx_topic);
    let thresholds = PeerScoreThresholds {
        gossip_threshold: -10.0,
        publish_threshold: -50.0,
        graylist_threshold: GRAYLIST_THRESHOLD,
        ..PeerScoreThresholds::default()
    };
    (params, thresholds)
}

fn build_behaviour(
    config: &NetworkConfig,
    chain_id: u64,
    keypair: &Keypair,
    topic: &TopicHash,
) -> Result<Behaviour, NetworkError> {
    let lib = |e: String| NetworkError::Libp2p(e);
    let max_peers = u32::try_from(config.max_peers).unwrap_or(u32::MAX);
    let limits = ConnectionLimits::default()
        .with_max_pending_incoming(Some(max_peers))
        .with_max_established(Some(max_peers))
        .with_max_established_per_peer(Some(MAX_CONNECTIONS_PER_PEER));

    let mut gossipsub = GossipBehaviour::new_with_subscription_filter(
        MessageAuthenticity::Signed(keypair.clone()),
        gossipsub_config(config, topic).map_err(lib)?,
        WhitelistSubscriptionFilter(HashSet::from([topic.clone()])),
    )
    .map_err(|e| lib(e.to_string()))?;
    let (params, thresholds) = peer_score(topic);
    gossipsub.with_peer_score(params, thresholds).map_err(lib)?;

    let local = keypair.public().to_peer_id();
    let kad = config.discovery.then(|| {
        let mut k = kad::Behaviour::with_config(
            local,
            kad::store::MemoryStore::new(local),
            kad::Config::new(kad_protocol(chain_id)),
        );
        // Answer queries even before an external address is confirmed (private devnets).
        k.set_mode(Some(kad::Mode::Server));
        k
    });

    Ok(Behaviour {
        limits: connection_limits::Behaviour::new(limits),
        blocked: allow_block_list::Behaviour::default(),
        gossipsub,
        identify: identify::Behaviour::new(
            identify::Config::new(identify_protocol(chain_id), keypair.public())
                .with_agent_version(format!("xdevs/{}", env!("CARGO_PKG_VERSION"))),
        ),
        ping: ping::Behaviour::new(ping::Config::new()),
        kad: Toggle::from(kad),
    })
}

fn build_swarm(
    config: &NetworkConfig,
    keypair: Keypair,
    behaviour: Behaviour,
) -> Result<Swarm<Behaviour>, NetworkError> {
    let lib = |e: &dyn std::fmt::Display| NetworkError::Libp2p(e.to_string());
    let swarm_cfg =
        |c: libp2p::swarm::Config| c.with_idle_connection_timeout(IDLE_CONNECTION_TIMEOUT);
    let builder = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::new().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| lib(&e))?;
    let swarm = if config.quic {
        builder
            .with_quic()
            .with_dns()
            .map_err(|e| lib(&e))?
            .with_behaviour(|_| behaviour)
            .map_err(|e| lib(&e))?
            .with_swarm_config(swarm_cfg)
            .build()
    } else {
        builder
            .with_dns()
            .map_err(|e| lib(&e))?
            .with_behaviour(|_| behaviour)
            .map_err(|e| lib(&e))?
            .with_swarm_config(swarm_cfg)
            .build()
    };
    Ok(swarm)
}

/// The addresses to listen on: `listen` itself, plus the same IP and port over QUIC when
/// `quic` is on and `listen` is a TCP address.
pub(crate) fn listen_addrs(config: &NetworkConfig) -> Result<Vec<Multiaddr>, NetworkError> {
    let listen = config.listen_addr()?;
    if is_quic(&listen) || !config.quic {
        return Ok(vec![listen]);
    }
    let quic: Multiaddr = listen
        .iter()
        .map(|p| match p {
            Protocol::Tcp(port) => Protocol::Udp(port),
            other => other,
        })
        .chain(std::iter::once(Protocol::QuicV1))
        .collect();
    Ok(vec![listen, quic])
}

/// `addr` without a trailing `/p2p/<peer id>`, and that peer id.
fn split_p2p(addr: &Multiaddr) -> (Multiaddr, Option<PeerId>) {
    let mut base = addr.clone();
    match base.pop() {
        Some(Protocol::P2p(peer)) => (base, Some(peer)),
        _ => (addr.clone(), None),
    }
}

/// A configured bootnode; `peer` is learnt on first connect when the address has no
/// `/p2p/` suffix.
struct Bootnode {
    addr: Multiaddr,
    base: Multiaddr,
    peer: Option<PeerId>,
}

/// Build the swarm, start listening, dial bootnodes and spawn the event loop.
pub(crate) async fn spawn(
    config: NetworkConfig,
    chain_id: u64,
    keypair: Keypair,
    validator: TxValidator,
) -> Result<NetworkHandle, NetworkError> {
    let local_peer_id = keypair.public().to_peer_id();
    let topic = tx_topic(chain_id);
    let behaviour = build_behaviour(&config, chain_id, &keypair, &topic.hash())?;
    let mut swarm = build_swarm(&config, keypair, behaviour)?;
    swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic)
        .map_err(|e| NetworkError::Libp2p(e.to_string()))?;

    let bootnodes = config
        .bootnode_addrs()?
        .into_iter()
        .map(|addr| {
            let (base, peer) = split_p2p(&addr);
            Bootnode { addr, base, peer }
        })
        .filter(|b| b.peer != Some(local_peer_id))
        .collect();

    let (cmd_tx, cmd_rx) = mpsc::channel(config.channel_capacity);
    let shared = Arc::new(Shared::default());
    let mut event_loop = EventLoop {
        swarm,
        topic: topic.hash(),
        identify_protocol: identify_protocol(chain_id),
        kad_protocol: kad_protocol(chain_id),
        validator,
        shared: shared.clone(),
        commands: cmd_rx,
        bootnodes,
        banned: BTreeMap::new(),
        ticks: 0,
        max_peers: config.max_peers,
        max_tx_size: config.max_tx_size,
    };
    event_loop.listen(listen_addrs(&config)?).await?;
    event_loop.add_bootnodes_to_kad();
    event_loop.dial_bootnodes();
    info!(
        target: "xdevs::network",
        peer_id = %local_peer_id,
        addrs = ?shared.listen_addrs(),
        topic = %topic,
        "tx gossip started"
    );

    let task = tokio::spawn(event_loop.run());
    Ok(NetworkHandle::new(
        local_peer_id,
        cmd_tx,
        shared,
        config.channel_capacity,
        config.max_tx_size,
        task,
    ))
}

/// Owns the swarm; driven by [`EventLoop::run`] on its own task.
struct EventLoop {
    swarm: Swarm<Behaviour>,
    topic: TopicHash,
    identify_protocol: String,
    kad_protocol: StreamProtocol,
    validator: TxValidator,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<Command>,
    bootnodes: Vec<Bootnode>,
    /// Banned peers and when their ban ends.
    banned: BTreeMap<PeerId, Instant>,
    ticks: u32,
    max_peers: usize,
    max_tx_size: usize,
}

impl EventLoop {
    /// Bind every listener and wait until each reports its first address.
    async fn listen(&mut self, addrs: Vec<Multiaddr>) -> Result<(), NetworkError> {
        let mut pending = Vec::new();
        for addr in addrs {
            let id = self
                .swarm
                .listen_on(addr.clone())
                .map_err(|e| NetworkError::Libp2p(format!("listen on {addr}: {e}")))?;
            pending.push(id);
        }
        let wait = async {
            while !pending.is_empty() {
                match self.swarm.select_next_some().await {
                    SwarmEvent::NewListenAddr {
                        listener_id,
                        address,
                    } => {
                        pending.retain(|l| *l != listener_id);
                        self.shared.add_listen_addr(address);
                    }
                    SwarmEvent::ListenerClosed {
                        listener_id,
                        reason,
                        ..
                    } if pending.contains(&listener_id) => {
                        return Err(NetworkError::Libp2p(format!("listener closed: {reason:?}")));
                    }
                    SwarmEvent::ListenerError { listener_id, error }
                        if pending.contains(&listener_id) =>
                    {
                        return Err(NetworkError::Libp2p(format!("listener error: {error}")));
                    }
                    other => self.on_swarm_event(other),
                }
            }
            Ok(())
        };
        tokio::time::timeout(LISTEN_TIMEOUT, wait)
            .await
            .map_err(|_| NetworkError::Libp2p("timed out binding listeners".into()))?
    }

    async fn run(mut self) {
        let mut tick = tokio::time::interval(MAINTENANCE_INTERVAL);
        tick.tick().await; // the first tick fires immediately; bootnodes were just dialled
        loop {
            tokio::select! {
                cmd = self.commands.recv() => match cmd {
                    Some(Command::PublishTx(tx)) => self.publish(tx),
                    #[cfg(test)]
                    Some(Command::PeerScore(peer, reply)) => {
                        let _ = reply.send(self.swarm.behaviour().gossipsub.peer_score(&peer));
                    }
                    // Shutdown, or every handle dropped.
                    Some(Command::Shutdown) | None => break,
                },
                event = self.swarm.select_next_some() => self.on_swarm_event(event),
                _ = tick.tick() => self.maintenance(),
            }
        }
        self.shutdown().await;
    }

    /// Unsubscribe, disconnect every peer and close all inbound subscriptions.
    async fn shutdown(mut self) {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .unsubscribe(&tx_topic_from_hash(&self.topic));
        let swarm = &mut self.swarm;
        // Give the unsubscribe a moment to flush, then disconnect.
        let _ = tokio::time::timeout(SHUTDOWN_GRACE / 4, async {
            loop {
                swarm.select_next_some().await;
            }
        })
        .await;
        let peers: Vec<PeerId> = swarm.connected_peers().copied().collect();
        for peer in peers {
            let _ = swarm.disconnect_peer_id(peer);
        }
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, async {
            while swarm.connected_peers().next().is_some() {
                swarm.select_next_some().await;
            }
        })
        .await;
        self.shared.peers.store(0, Ordering::Relaxed);
        self.shared.topic_peers.store(0, Ordering::Relaxed);
        self.shared.close();
        info!(target: "xdevs::network", "tx gossip stopped");
    }

    fn publish(&mut self, tx: Bytes) {
        match self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic.clone(), tx.to_vec())
        {
            Ok(_) => {}
            // No peers yet, or a tx we already saw: nothing to do.
            Err(
                e @ (gossipsub::PublishError::NoPeersSubscribedToTopic
                | gossipsub::PublishError::Duplicate),
            ) => {
                debug!(target: "xdevs::network", "tx not published: {e}")
            }
            Err(e) => warn!(target: "xdevs::network", "tx publish failed: {e}"),
        }
    }

    fn on_swarm_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(e)) => self.on_gossip(e),
            SwarmEvent::Behaviour(BehaviourEvent::Identify(e)) => self.on_identify(e),
            SwarmEvent::Behaviour(BehaviourEvent::Kad(e)) => self.on_kad(e),
            SwarmEvent::Behaviour(BehaviourEvent::Ping(_)) => {}
            SwarmEvent::Behaviour(BehaviourEvent::Limits(never))
            | SwarmEvent::Behaviour(BehaviourEvent::Blocked(never)) => match never {},
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                if let ConnectedPoint::Dialer { address, .. } = &endpoint {
                    let base = split_p2p(address).0;
                    for b in self.bootnodes.iter_mut().filter(|b| b.base == base) {
                        b.peer.get_or_insert(peer_id);
                    }
                }
                debug!(target: "xdevs::network", %peer_id, ?endpoint, "connected");
                self.update_peer_counts();
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                debug!(target: "xdevs::network", %peer_id, ?cause, "connection closed");
                self.update_peer_counts();
            }
            SwarmEvent::NewListenAddr { address, .. } => self.shared.add_listen_addr(address),
            SwarmEvent::ExpiredListenAddr { address, .. } => {
                self.shared.remove_listen_addr(&address)
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                debug!(target: "xdevs::network", ?peer_id, "dial failed: {error}")
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                debug!(target: "xdevs::network", "inbound connection failed: {error}")
            }
            SwarmEvent::ListenerError { error, .. } => {
                warn!(target: "xdevs::network", "listener error: {error}")
            }
            SwarmEvent::ListenerClosed { reason, .. } => {
                warn!(target: "xdevs::network", "listener closed: {reason:?}")
            }
            _ => {}
        }
    }

    fn on_gossip(&mut self, event: gossipsub::Event) {
        match event {
            gossipsub::Event::Message {
                propagation_source,
                message_id,
                message,
            } => self.on_message(propagation_source, message_id, message),
            gossipsub::Event::Subscribed { .. } | gossipsub::Event::Unsubscribed { .. } => {
                self.update_peer_counts()
            }
            other => debug!(target: "xdevs::network", ?other, "gossipsub event"),
        }
    }

    /// Strict validation of one inbound tx: only accepted txs are delivered and forwarded.
    fn on_message(&mut self, from: PeerId, id: MessageId, message: gossipsub::Message) {
        let verdict = if message.topic != self.topic || message.data.len() > self.max_tx_size {
            Verdict::Reject
        } else {
            let validator = &self.validator;
            catch_unwind(AssertUnwindSafe(|| validator(&message.data))).unwrap_or_else(|_| {
                warn!(target: "xdevs::network", "tx validator panicked; ignoring tx");
                Verdict::Ignore
            })
        };
        self.shared.stats.record(verdict);
        self.swarm
            .behaviour_mut()
            .gossipsub
            .report_message_validation_result(&id, &from, verdict.into());
        match verdict {
            Verdict::Accept => self.shared.deliver(from, &Bytes::from(message.data)),
            Verdict::Ignore => {}
            Verdict::Reject => {
                debug!(target: "xdevs::network", %from, "rejected invalid tx");
                self.ban_if_graylisted(from);
            }
        }
    }

    fn on_identify(&mut self, event: identify::Event) {
        let identify::Event::Received { peer_id, info, .. } = event else {
            return;
        };
        if info.protocol_version != self.identify_protocol {
            debug!(
                target: "xdevs::network",
                %peer_id,
                theirs = %info.protocol_version,
                "peer is on another network; disconnecting"
            );
            let _ = self.swarm.disconnect_peer_id(peer_id);
            return;
        }
        let speaks_kad = info.protocols.contains(&self.kad_protocol);
        if let Some(kad) = self.swarm.behaviour_mut().kad.as_mut() {
            if speaks_kad {
                for addr in info.listen_addrs {
                    kad.add_address(&peer_id, addr);
                }
            }
        }
    }

    fn on_kad(&mut self, event: kad::Event) {
        if let kad::Event::RoutingUpdated {
            peer, addresses, ..
        } = event
        {
            if !self.swarm.is_connected(&peer)
                && self.has_room()
                && !self.banned.contains_key(&peer)
            {
                let opts = DialOpts::peer_id(peer)
                    .addresses(addresses.into_vec())
                    .condition(PeerCondition::DisconnectedAndNotDialing)
                    .build();
                if let Err(e) = self.swarm.dial(opts) {
                    debug!(target: "xdevs::network", %peer, "discovery dial failed: {e}");
                }
            }
        }
    }

    fn has_room(&self) -> bool {
        self.swarm.connected_peers().count() < self.max_peers
    }

    fn update_peer_counts(&self) {
        let peers = self.swarm.connected_peers().count();
        let topic_peers = self
            .swarm
            .behaviour()
            .gossipsub
            .all_peers()
            .filter(|(_, topics)| topics.contains(&&self.topic))
            .count();
        self.shared.peers.store(peers, Ordering::Relaxed);
        self.shared
            .topic_peers
            .store(topic_peers, Ordering::Relaxed);
    }

    fn add_bootnodes_to_kad(&mut self) {
        let known: Vec<_> = self
            .bootnodes
            .iter()
            .filter_map(|b| b.peer.map(|p| (p, b.base.clone())))
            .collect();
        if let Some(kad) = self.swarm.behaviour_mut().kad.as_mut() {
            for (peer, addr) in known {
                kad.add_address(&peer, addr);
            }
        }
    }

    /// Dial every bootnode we are not connected to (and have not banned).
    fn dial_bootnodes(&mut self) {
        for i in 0..self.bootnodes.len() {
            let (addr, peer) = (self.bootnodes[i].addr.clone(), self.bootnodes[i].peer);
            let opts = match peer {
                Some(p) if self.swarm.is_connected(&p) || self.banned.contains_key(&p) => continue,
                Some(p) => DialOpts::peer_id(p)
                    .addresses(vec![addr.clone()])
                    .condition(PeerCondition::DisconnectedAndNotDialing)
                    .build(),
                None => DialOpts::unknown_peer_id().address(addr.clone()).build(),
            };
            if !self.has_room() {
                return;
            }
            if let Err(e) = self.swarm.dial(opts) {
                debug!(target: "xdevs::network", %addr, "bootnode dial failed: {e}");
            }
        }
    }

    /// Ban `peer` when its gossip score is below the graylist threshold.
    fn ban_if_graylisted(&mut self, peer: PeerId) {
        let score = self.swarm.behaviour().gossipsub.peer_score(&peer);
        if score.is_some_and(|s| s < GRAYLIST_THRESHOLD) {
            self.ban(peer);
        }
    }

    fn ban(&mut self, peer: PeerId) {
        if self.banned.len() >= MAX_BANNED && !self.banned.contains_key(&peer) {
            if let Some(oldest) = self
                .banned
                .iter()
                .min_by_key(|(_, until)| **until)
                .map(|(p, _)| *p)
            {
                self.banned.remove(&oldest);
                self.swarm.behaviour_mut().blocked.unblock_peer(oldest);
            }
        }
        warn!(target: "xdevs::network", %peer, "banning peer for invalid tx gossip");
        self.banned.insert(peer, Instant::now() + BAN_DURATION);
        self.swarm.behaviour_mut().blocked.block_peer(peer);
        self.shared.stats.banned.fetch_add(1, Ordering::Relaxed);
    }

    /// Periodic: expire bans, ban graylisted peers, redial bootnodes, kad bootstrap.
    fn maintenance(&mut self) {
        let now = Instant::now();
        let expired: Vec<PeerId> = self
            .banned
            .iter()
            .filter(|(_, until)| **until <= now)
            .map(|(p, _)| *p)
            .collect();
        for peer in expired {
            self.banned.remove(&peer);
            self.swarm.behaviour_mut().blocked.unblock_peer(peer);
        }

        // Catches peers penalised by gossipsub itself (oversized or badly signed messages).
        let peers: Vec<PeerId> = self.swarm.connected_peers().copied().collect();
        for peer in peers {
            self.ban_if_graylisted(peer);
        }

        self.dial_bootnodes();

        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks % KAD_BOOTSTRAP_TICKS == 1 {
            if let Some(kad) = self.swarm.behaviour_mut().kad.as_mut() {
                let _ = kad.bootstrap(); // Err only when the routing table is empty
            }
        }
    }
}

/// The [`IdentTopic`] for a topic hash (identity hashing: the hash is the topic string).
fn tx_topic_from_hash(hash: &TopicHash) -> IdentTopic {
    IdentTopic::new(hash.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_and_message_id() {
        assert_eq!(tx_topic(7117).to_string(), "/xdevs/7117/tx/1");
        assert_eq!(tx_message_id(b"abc"), tx_message_id(b"abc"));
        assert_ne!(tx_message_id(b"abc"), tx_message_id(b"abd"));
        assert_eq!(tx_message_id(b"").0, keccak256([]).to_vec());
        assert_eq!(
            tx_topic_from_hash(&tx_topic(5).hash()).hash(),
            tx_topic(5).hash()
        );
    }

    #[test]
    fn verdict_maps_to_acceptance() {
        assert!(matches!(
            MessageAcceptance::from(Verdict::Accept),
            MessageAcceptance::Accept
        ));
        assert!(matches!(
            MessageAcceptance::from(Verdict::Ignore),
            MessageAcceptance::Ignore
        ));
        assert!(matches!(
            MessageAcceptance::from(Verdict::Reject),
            MessageAcceptance::Reject
        ));
    }

    #[test]
    fn keypair_from_seed_is_deterministic() {
        let a = Keypair::ed25519_from_bytes([3u8; 32]).unwrap();
        let b = Keypair::ed25519_from_bytes([3u8; 32]).unwrap();
        assert_eq!(a.public().to_peer_id(), b.public().to_peer_id());
    }

    #[test]
    fn quic_listen_address_is_derived() {
        let mut c = NetworkConfig {
            listen: "/ip4/127.0.0.1/tcp/30334".into(),
            ..NetworkConfig::default()
        };
        assert_eq!(listen_addrs(&c).unwrap().len(), 1);
        c.quic = true;
        let addrs = listen_addrs(&c).unwrap();
        assert_eq!(addrs[1].to_string(), "/ip4/127.0.0.1/udp/30334/quic-v1");
        c.listen = "/ip4/127.0.0.1/udp/9/quic-v1".into();
        assert_eq!(listen_addrs(&c).unwrap(), vec![c.listen_addr().unwrap()]);
    }

    #[test]
    fn p2p_suffix_is_split() {
        let peer = PeerId::random();
        let a: Multiaddr = format!("/ip4/1.2.3.4/tcp/5/p2p/{peer}").parse().unwrap();
        assert_eq!(
            split_p2p(&a),
            ("/ip4/1.2.3.4/tcp/5".parse().unwrap(), Some(peer))
        );
        let b: Multiaddr = "/dns4/val1/tcp/30334".parse().unwrap();
        assert_eq!(split_p2p(&b), (b.clone(), None));
    }

    #[test]
    fn behaviour_config_is_valid() {
        let topic = tx_topic(1).hash();
        let (params, thresholds) = peer_score(&topic);
        params.validate().unwrap();
        thresholds.validate().unwrap();
        // Three invalid txs graylist a peer; two do not.
        const { assert!(INVALID_MESSAGE_WEIGHT * 9.0 < GRAYLIST_THRESHOLD) };
        const { assert!(INVALID_MESSAGE_WEIGHT * 4.0 > GRAYLIST_THRESHOLD) };
        let cfg = gossipsub_config(&NetworkConfig::default(), &topic).unwrap();
        assert_eq!(
            cfg.max_transmit_size_for_topic(&topic),
            crate::MAX_TX_SIZE + 1024
        );
        assert!(cfg.validate_messages());
        let kp = Keypair::generate_ed25519();
        build_behaviour(&NetworkConfig::default(), 1, &kp, &topic).unwrap();
    }
}

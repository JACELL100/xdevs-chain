//! The `AppMsg` loop (research doc §6.3): drives a [`ConsensusApp`] from Malachite's
//! channel-based interface, plus the config mapping (§6.10, §7) and engine start / stop.

use super::codec::{
    body_matches_header, decode_block, encode_block, to_malachite_cert, to_xdevs_cert, XdevsCodec,
};
use super::context::{
    round_u32, BlockValue, Height, MValidatorSet, ProposalPart, Vote, XdevsContext,
};
use super::signer::{vote_bytes, XdevsSigner, XdevsVerifier};
use super::store::{ConsensusStore, ProposalMeta};
use super::streaming::{
    make_parts, stream_messages, CompleteStream, PartStreams, PublisherClass, CHUNK_SIZE,
};
use super::{linear_timeouts, private_key, PrivateKey};
use crate::app::{BlockOrigin, ConsensusApp};
use crate::config::EngineConfig;
use crate::engine::{EngineKeys, EngineStatus};
use crate::types::{
    parts_sign_bytes, ConflictingVotes, Evidence, SignedVote, ValidatorAddress, ValidatorKey,
    ValidatorSet, VoteType,
};
use anyhow::{anyhow, bail, Context as _};
use arc_malachitebft_app_channel::app::engine::util::events::{Event, RxEvent};
use arc_malachitebft_app_channel::app::{
    config::{
        ChannelNames, ConsensusConfig, GossipSubConfig, NodeConfig, ProtocolNames, PubSubProtocol,
        ValuePayload, ValueSyncConfig,
    },
    engine::host::{HeightParams, Next, SyncedValueOutcome},
    streaming::{StreamContent, StreamId, StreamMessage},
    types::{
        codec::Codec,
        core::{
            NilOrVal, Round, SignedMessage, ValidatorSet as _, Validity as MValidity,
            VoteType as MVoteType,
        },
        Keypair, LocallyProposedValue, MisbehaviorEvidence, PeerId, ProposedValue,
        SignedConsensusMsg,
    },
};
use arc_malachitebft_app_channel::{
    start_engine, AppMsg, Channels, ConsensusContext, EngineHandle, NetworkContext,
    NetworkIdentity, NetworkMsg, NetworkRequest, RequestContext, Signer as _, SyncContext,
    WalContext,
};
use arc_malachitebft_sync::RawDecidedValue;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, Semaphore};
use tracing::{debug, error, info, warn};
use xdevs_primitives::{keccak256, Block, B256};

const LOG: &str = "xdevs::consensus";
/// Sync responses are capped well below Malachite's 10 MiB `max_response_size`.
const MAX_SYNC_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Config mapping (§6.10, §7)
// ---------------------------------------------------------------------------

/// Malachite `NodeConfig` built from the xdevs [`EngineConfig`].
#[derive(Clone, Debug)]
pub(crate) struct XdevsNodeConfig {
    moniker: String,
    consensus: ConsensusConfig,
    value_sync: ValueSyncConfig,
}

impl NodeConfig for XdevsNodeConfig {
    fn moniker(&self) -> &str {
        &self.moniker
    }

    fn consensus(&self) -> &ConsensusConfig {
        &self.consensus
    }

    fn consensus_mut(&mut self) -> &mut ConsensusConfig {
        &mut self.consensus
    }

    fn value_sync(&self) -> &ValueSyncConfig {
        &self.value_sync
    }

    fn value_sync_mut(&mut self) -> &mut ValueSyncConfig {
        &mut self.value_sync
    }
}

/// `[consensus]` / `[p2p]` → Malachite `ConsensusConfig` + `ValueSyncConfig`.
pub(crate) fn node_config(c: &EngineConfig) -> anyhow::Result<XdevsNodeConfig> {
    c.timeouts.validate().map_err(anyhow::Error::msg)?;
    let mut consensus = ConsensusConfig {
        enabled: true,
        value_payload: ValuePayload::ProposalAndParts,
        ..Default::default()
    };
    let p2p = &mut consensus.p2p;
    p2p.listen_addr = c
        .p2p
        .listen
        .parse()
        .map_err(|e| anyhow!("p2p.listen {:?}: {e}", c.p2p.listen))?;
    p2p.persistent_peers = c
        .p2p
        .persistent_peers
        .iter()
        .map(|a| {
            a.parse()
                .map_err(|e| anyhow!("p2p.persistent_peers {a:?}: {e}"))
        })
        .collect::<anyhow::Result<_>>()?;
    p2p.discovery.enabled = c.p2p.discovery;
    // Explicit peering: always forward to persistent peers (small validator sets).
    p2p.protocol = PubSubProtocol::GossipSub(GossipSubConfig::new(6, 12, 4, 2, false, true, true));
    // Proposal parts carry ≤ 256 KiB chunks plus framing (§6.4).
    p2p.pubsub_max_size_per_topic.proposal_parts = Some(
        format!("{} KiB", CHUNK_SIZE / 1024 + 4)
            .parse()
            .map_err(|e| anyhow!("byte size: {e}"))?,
    );
    p2p.channel_names = ChannelNames {
        consensus: c.channel_name("consensus"),
        proposal_parts: c.channel_name("proposal_parts"),
        sync: c.channel_name("sync"),
        liveness: c.channel_name("liveness"),
    };
    let proto = |name: &str| format!("/xdevs/{}/{name}/1", c.chain_id);
    p2p.protocol_names = ProtocolNames {
        consensus: proto("consensus"),
        discovery_kad: proto("kad"),
        discovery_regres: proto("discovery"),
        sync: proto("sync"),
        validator_proof: proto("validator-proof"),
    };
    let value_sync = ValueSyncConfig {
        enabled: c.value_sync,
        // Tip gossip scales with the block time: 5 blocks, within [1 s, 10 s].
        status_update_interval: (c.timeouts.block_time() * 5)
            .clamp(Duration::from_secs(1), Duration::from_secs(10)),
        ..Default::default()
    };
    Ok(XdevsNodeConfig {
        moniker: c.moniker.clone(),
        consensus,
        value_sync,
    })
}

// ---------------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------------

/// Retry a fallible start step: after a restart in the same process the previous
/// instance's WAL thread / redb handle / listener may take a moment to go away.
async fn retry<T, F, Fut>(what: &str, mut f: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    const ATTEMPTS: u32 = 25;
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                debug!(target: LOG, attempt, "{what} failed: {e:#}");
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(last
        .expect("at least one attempt")
        .context(format!("{what} failed")))
}

struct Started {
    channels: Channels<XdevsContext>,
    handle: EngineHandle,
    key: Option<PrivateKey>,
    address: ValidatorAddress,
}

async fn start(
    config: &EngineConfig,
    node_cfg: &XdevsNodeConfig,
    keys: &EngineKeys,
    store: &Arc<ConsensusStore>,
) -> anyhow::Result<Started> {
    let chain_id = config.chain_id;
    let codec = XdevsCodec;
    let mut node_seed = zeroize::Zeroizing::new(*keys.node_key_seed);
    let keypair =
        Keypair::ed25519_from_bytes(&mut node_seed[..]).map_err(|e| anyhow!("node key: {e}"))?;
    let verifier = Box::new(XdevsVerifier::new(chain_id));
    let (consensus_ctx, identity, key, address) = match &keys.consensus_seed {
        Some(seed) => {
            let key = private_key(seed);
            let public = *key.public_key().as_bytes();
            let address = ValidatorKey(public).address();
            let signer = XdevsSigner::new(chain_id, key.clone(), store.clone());
            let peer_id = keypair.public().to_peer_id().to_bytes();
            let proof = signer
                .sign_validator_proof(public.to_vec(), peer_id)
                .await
                .map_err(|e| anyhow!("validator proof: {e}"))?;
            let proof_bytes = Codec::encode(&codec, &proof)?;
            let identity = NetworkIdentity::new_validator(
                config.moniker.clone(),
                keypair,
                address.to_string(),
                proof_bytes,
            );
            (
                ConsensusContext::new_validator(address, verifier, Box::new(signer)),
                identity,
                Some(key),
                address,
            )
        }
        None => {
            let address = ValidatorAddress::default();
            (
                ConsensusContext::new_full_node(address, verifier),
                NetworkIdentity::new(config.moniker.clone(), keypair, None),
                None,
                address,
            )
        }
    };
    let (channels, handle) = start_engine(
        XdevsContext::new(chain_id),
        node_cfg.clone(),
        WalContext::new(config.wal_path(), codec),
        NetworkContext::new(identity, codec),
        consensus_ctx,
        SyncContext::new(codec),
        RequestContext::new(100),
    )
    .await
    .map_err(|e| anyhow!("start malachite: {e:#}"))?;
    Ok(Started {
        channels,
        handle,
        key,
        address,
    })
}

async fn stop(handle: EngineHandle) {
    let timeout = Some(Duration::from_secs(5));
    let EngineHandle { actor, handle } = handle;
    actor
        .get_cell()
        .stop_children_and_wait(Some("xdevs shutdown".into()), timeout)
        .await;
    let _ = actor
        .stop_and_wait(Some("xdevs shutdown".into()), timeout)
        .await;
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

/// `MalachiteEngine::run`: start Malachite, loop on `AppMsg` until `shutdown` (or a
/// fatal error), then stop every actor.
pub(crate) async fn run(
    config: EngineConfig,
    keys: EngineKeys,
    app: Arc<dyn ConsensusApp>,
    shutdown: watch::Receiver<bool>,
    status: EngineStatus,
) -> anyhow::Result<()> {
    run_inner(config, keys, app, shutdown, status, false).await
}

/// [`run`], optionally as a Byzantine validator that floods junk proposal streams
/// (tests only; `byzantine` is ignored otherwise).
pub(crate) async fn run_inner(
    config: EngineConfig,
    keys: EngineKeys,
    app: Arc<dyn ConsensusApp>,
    mut shutdown: watch::Receiver<bool>,
    status: EngineStatus,
    byzantine: bool,
) -> anyhow::Result<()> {
    #[cfg(not(test))]
    let _ = byzantine;
    let node_cfg = node_config(&config)?;
    if *shutdown.borrow() {
        return Ok(());
    }
    std::fs::create_dir_all(&config.consensus_dir)
        .with_context(|| format!("create {}", config.consensus_dir.display()))?;
    let db_path = config.consensus_db_path();
    let store = Arc::new(
        retry("open consensus.redb", || async {
            ConsensusStore::open(&db_path)
        })
        .await?,
    );
    if let Some(last) = store.last_signed()? {
        info!(target: LOG, ?last, "SignGuard state loaded");
    }
    let started = retry("start malachite", || {
        start(&config, &node_cfg, &keys, &store)
    })
    .await?;
    info!(
        target: LOG,
        moniker = %config.moniker,
        validator = started.key.is_some(),
        address = %started.address,
        "consensus engine started"
    );
    let Channels {
        consensus: rx,
        network,
        events,
        net_requests,
        ..
    } = started.channels;
    status.set_running(true);
    let peer_validators = PeerValidators::default();
    let monitor = tokio::spawn(monitor_status(
        status.clone(),
        events.subscribe(),
        net_requests,
        peer_validators.clone(),
    ));
    let mut host = Host::new(
        config.chain_id,
        app,
        store,
        started.key,
        started.address,
        config.timeouts.clone(),
        network,
        peer_validators,
    );
    #[cfg(test)]
    {
        host.byzantine = byzantine;
    }
    let result = host.run(rx, events.subscribe(), &mut shutdown).await;
    if let Err(e) = &result {
        error!(target: LOG, "consensus engine failed: {e:#}");
    }
    monitor.abort();
    status.set_running(false);
    status.set_peer_count(0);
    drop(host);
    stop(started.handle).await;
    info!(target: LOG, moniker = %config.moniker, "consensus engine stopped");
    result
}

/// libp2p identities of connected peers whose validator proof Malachite verified,
/// mapped to the validator address of the proven consensus key. Refreshed every second
/// from the network state; used to bind a proposal stream's publisher to a validator.
type PeerValidators = Arc<RwLock<HashMap<PeerId, ValidatorAddress>>>;

/// Keep [`EngineStatus`] current: heights from engine events, the peer count (and the
/// peer → validator bindings) from a network state dump every second. Aborted when the
/// engine stops.
async fn monitor_status(
    status: EngineStatus,
    mut events: RxEvent<XdevsContext>,
    net: mpsc::Sender<NetworkRequest>,
    peer_validators: PeerValidators,
) {
    use tokio::sync::broadcast::error::RecvError;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut events_open = true;
    loop {
        tokio::select! {
            ev = events.recv(), if events_open => match ev {
                Ok(Event::StartedHeight(h, _)) => status.set_height(h.0),
                Ok(Event::Received(msg)) => status.observe_height(msg.height().0),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => events_open = false,
            },
            _ = tick.tick() => {
                let dump = tokio::time::timeout(
                    Duration::from_millis(500),
                    NetworkRequest::dump_state(&net),
                )
                .await;
                if let Ok(Ok(Some(dump))) = dump {
                    status.set_peer_count(dump.peers.len());
                    let bound: HashMap<PeerId, ValidatorAddress> = dump
                        .peers
                        .iter()
                        .filter_map(|(id, info)| {
                            let pk: [u8; 32] = info.consensus_public_key.as_deref()?.try_into().ok()?;
                            let peer = PeerId::from_bytes(&id.to_bytes()).ok()?;
                            Some((peer, ValidatorKey(pk).address()))
                        })
                        .collect();
                    if let Ok(mut map) = peer_validators.write() {
                        *map = bound;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stream admission
// ---------------------------------------------------------------------------

/// Proposal streams are accepted for heights `committed + 1 ..= committed + HEIGHT_WINDOW`.
const HEIGHT_WINDOW: u64 = 2;
/// Rounds accepted ahead of the current round (of the next height: ahead of round 0).
const MAX_ROUND_AHEAD: u32 = 8;
/// Sync requests served concurrently (each on its own task, off the consensus loop).
const SYNC_SERVE_CONCURRENCY: usize = 4;
/// Most decided values in one sync response (besides the byte cap).
const MAX_SYNC_VALUES: u64 = 64;
/// Commit attempts of a decided block at `Finalized` before the engine halts.
const COMMIT_ATTEMPTS: u32 = 6;
/// First delay between commit attempts (doubles each time: ~3 s in total).
const COMMIT_BACKOFF: Duration = Duration::from_millis(100);

/// Why a proposal stream is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not in `committed + 1 ..= committed + HEIGHT_WINDOW`.
    HeightOutsideWindow,
    /// More than [`MAX_ROUND_AHEAD`] rounds ahead.
    RoundTooFarAhead,
    /// `Round::Nil` (the codec already refuses it).
    NilRound,
    /// The publisher proved another validator's key.
    PublisherIsAnotherValidator,
    /// The claimed proposer is not the §6.6 proposer of `(height, round)`.
    NotTheProposer,
    /// The validator set of the height is unavailable.
    NoValidatorSet,
}

/// The admission rule for a stream's `Init` (re-checked when the stream completes):
/// height in the window, round not too far ahead, the claimed proposer is the §6.6
/// proposer of `(height, round)` and, if the publisher's libp2p identity is bound to a
/// validator, that validator. Malachite keeps at most two values per `(height, round)`,
/// so without this any validator could fill every round with junk.
pub(crate) fn admit(
    committed: u64,
    current: (u64, u32),
    height: u64,
    round: Round,
    proposer: &ValidatorAddress,
    publisher_validator: Option<&ValidatorAddress>,
    set: &ValidatorSet,
) -> Result<(), Refusal> {
    if !in_window(committed, height) {
        return Err(Refusal::HeightOutsideWindow);
    }
    let Some(round) = round.as_u32() else {
        return Err(Refusal::NilRound);
    };
    let base = if current.0 == height { current.1 } else { 0 };
    if round > base.saturating_add(MAX_ROUND_AHEAD) {
        return Err(Refusal::RoundTooFarAhead);
    }
    if publisher_validator.is_some_and(|v| v != proposer) {
        return Err(Refusal::PublisherIsAnotherValidator);
    }
    if set.select_proposer(height, round).address != *proposer {
        return Err(Refusal::NotTheProposer);
    }
    Ok(())
}

fn in_window(committed: u64, height: u64) -> bool {
    height > committed && height <= committed.saturating_add(HEIGHT_WINDOW)
}

// ---------------------------------------------------------------------------
// Catching up (pacing)
// ---------------------------------------------------------------------------

/// The highest height at which each validator has a validly signed vote (live gossip).
///
/// A correct validator votes at `h + 1` only after deciding `h`, so votes above `h` from
/// more than 1/3 of the voting power (hence from at least one correct validator) prove
/// that the network has decided `h`: a node starting `h` is then catching up. A
/// Byzantine minority cannot fake this, unlike a single streamed proposal.
#[derive(Debug, Default)]
pub(crate) struct VoteHeights {
    max: HashMap<ValidatorAddress, u64>,
}

impl VoteHeights {
    /// Record `vote` if it is a validly signed vote of a member of `set` above the
    /// validator's highest so far (one signature check per new height per validator).
    pub fn observe(
        &mut self,
        chain_id: u64,
        set: &MValidatorSet,
        vote: &SignedMessage<XdevsContext, Vote>,
    ) -> bool {
        let v = &vote.message;
        let height = v.height.0;
        if self
            .max
            .get(&v.validator_address)
            .is_some_and(|m| *m >= height)
        {
            return false;
        }
        let Some(validator) = set.get_by_address(&v.validator_address) else {
            return false;
        };
        if validator
            .public_key
            .verify(&vote_bytes(chain_id, v), &vote.signature)
            .is_err()
        {
            return false;
        }
        self.max.insert(v.validator_address, height);
        true
    }

    /// `true` if validators with more than 1/3 of the power voted above `height`.
    pub fn network_is_past(&self, height: u64, set: &ValidatorSet) -> bool {
        let power: u128 = set
            .validators()
            .iter()
            .filter(|v| self.max.get(&v.address).is_some_and(|m| *m > height))
            .map(|v| u128::from(v.voting_power))
            .sum();
        power * 3 > u128::from(set.total_voting_power())
    }
}

// ---------------------------------------------------------------------------
// The host: AppMsg → ConsensusApp
// ---------------------------------------------------------------------------

fn round_enc(r: Round) -> u64 {
    (r.as_i64() + 1) as u64
}

fn round_dec(v: u64) -> Round {
    if v == 0 {
        Round::Nil
    } else {
        Round::new(v.saturating_sub(1).min(u64::from(u32::MAX)) as u32)
    }
}

/// Malachite `target_time` of a height: the block time, or none while catching up
/// (see `Host::height_params`).
fn target_time(t: &crate::config::ConsensusTimeouts, catching_up: bool) -> Option<Duration> {
    (!catching_up).then(|| t.block_time())
}

fn to_m_validity(v: crate::Validity) -> MValidity {
    MValidity::from_bool(v.is_valid())
}

fn meta_validity(v: crate::Validity) -> u8 {
    if v.is_valid() {
        ProposalMeta::VALID
    } else {
        ProposalMeta::INVALID
    }
}

/// The validation origin of a proposal. A re-proposal of a value that already has a
/// polka (`pol_round` set) is checked like a synced block: more than 2/3 already
/// prevoted it, so the subjective "timestamp not too far in the future" rule must not
/// split correct nodes with skewed clocks (Malachite only acts on such a proposal when
/// it holds that polka itself).
fn origin_for(pol_round: Round) -> BlockOrigin {
    if pol_round == Round::Nil {
        BlockOrigin::Consensus
    } else {
        BlockOrigin::Sync
    }
}

struct Host {
    chain_id: u64,
    app: Arc<dyn ConsensusApp>,
    store: Arc<ConsensusStore>,
    key: Option<PrivateKey>,
    address: ValidatorAddress,
    timeouts: crate::config::ConsensusTimeouts,
    network: mpsc::Sender<NetworkMsg<XdevsContext>>,
    streams: PartStreams,
    /// Validator sets of the heights in the admission window only.
    vsets: BTreeMap<u64, MValidatorSet>,
    /// Last committed `(height, hash)`.
    committed: (u64, B256),
    /// Height and round of the last `StartedRound`.
    current: (u64, u32),
    /// A decision whose commit failed at `Decided`; retried at `Finalized`.
    pending_commit: Option<(Block, crate::CommitCertificate)>,
    stream_nonce: u64,
    peer_validators: PeerValidators,
    vote_heights: VoteHeights,
    /// `(height, round)`s whose proposer was already reported for equivocation.
    equivocations: BTreeSet<(u64, u32)>,
    sync_permits: Arc<Semaphore>,
    commit_backoff: Duration,
    /// Tests: flood junk proposal streams (see `publish_junk`).
    #[cfg(test)]
    byzantine: bool,
}

impl Host {
    #[allow(clippy::too_many_arguments)]
    fn new(
        chain_id: u64,
        app: Arc<dyn ConsensusApp>,
        store: Arc<ConsensusStore>,
        key: Option<PrivateKey>,
        address: ValidatorAddress,
        timeouts: crate::config::ConsensusTimeouts,
        network: mpsc::Sender<NetworkMsg<XdevsContext>>,
        peer_validators: PeerValidators,
    ) -> Self {
        Self {
            chain_id,
            app,
            store,
            key,
            address,
            timeouts,
            network,
            streams: PartStreams::new(),
            vsets: BTreeMap::new(),
            committed: (0, B256::ZERO),
            current: (0, 0),
            pending_commit: None,
            stream_nonce: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            peer_validators,
            vote_heights: VoteHeights::default(),
            equivocations: BTreeSet::new(),
            sync_permits: Arc::new(Semaphore::new(SYNC_SERVE_CONCURRENCY)),
            commit_backoff: COMMIT_BACKOFF,
            #[cfg(test)]
            byzantine: false,
        }
    }

    async fn run(
        &mut self,
        mut rx: mpsc::Receiver<AppMsg<XdevsContext>>,
        mut events: RxEvent<XdevsContext>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        use tokio::sync::broadcast::error::RecvError;
        let mut events_open = true;
        loop {
            tokio::select! {
                biased;
                // A dropped sender counts as a shutdown request.
                _ = async { let _ = shutdown.wait_for(|stop| *stop).await; } => return Ok(()),
                msg = rx.recv() => match msg {
                    Some(msg) => self.handle(msg).await?,
                    None => bail!("Malachite stopped (consensus channel closed)"),
                },
                ev = events.recv(), if events_open => match ev {
                    Ok(Event::Received(SignedConsensusMsg::Vote(vote))) => self.observe_vote(&vote),
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => events_open = false,
                },
            }
        }
    }

    /// A live vote from the network (not yet verified by Malachite): only votes above
    /// the next height matter (pacing evidence), checked against the next height's set.
    fn observe_vote(&mut self, vote: &SignedMessage<XdevsContext, Vote>) {
        let next = self.committed.0 + 1;
        if vote.message.height.0 <= next {
            return;
        }
        if let Some(set) = self.vsets.get(&next) {
            self.vote_heights.observe(self.chain_id, set, vote);
        }
    }

    /// The validator set of `height`, cached only for heights in the admission window
    /// (an attacker-chosen height is never cached).
    async fn validator_set(&mut self, height: u64) -> anyhow::Result<MValidatorSet> {
        if let Some(s) = self.vsets.get(&height) {
            return Ok(s.clone());
        }
        let set = MValidatorSet::new(self.app.validator_set(height).await?)?;
        if in_window(self.committed.0, height) {
            self.vsets.insert(height, set.clone());
        }
        Ok(set)
    }

    /// The §6.6 proposer of `(height, round)`.
    async fn proposer_of(&mut self, height: u64, round: u32) -> anyhow::Result<ValidatorAddress> {
        Ok(self
            .validator_set(height)
            .await?
            .inner()
            .select_proposer(height, round)
            .address)
    }

    /// Parameters of `height`. The block time is Malachite's `target_time` (minimum
    /// height duration) only while this node is at the tip. If validators with more
    /// than 1/3 of the power already vote above `height` (see [`VoteHeights`]), the
    /// network has decided `height` and this node is catching up: it must finalize as
    /// soon as it decides, or a node that decides from buffered live votes (instead of
    /// ValueSync) advances exactly at the chain's pace and never closes its lag. Pacing
    /// only, never safety.
    async fn height_params(&mut self, height: u64) -> anyhow::Result<HeightParams<XdevsContext>> {
        let set = self.validator_set(height).await?;
        let catching_up = self.vote_heights.network_is_past(height, set.inner());
        if catching_up {
            debug!(target: LOG, height, "catching up: no minimum height duration");
        }
        Ok(HeightParams::new(
            set,
            linear_timeouts(&self.timeouts),
            target_time(&self.timeouts, catching_up),
        ))
    }

    async fn handle(&mut self, msg: AppMsg<XdevsContext>) -> anyhow::Result<()> {
        match msg {
            AppMsg::ConsensusReady { reply } => {
                let (h, hash) = self.app.head().await.context("app.head")?;
                self.committed = (h, hash);
                self.store.prune(h)?;
                let params = self.height_params(h + 1).await?;
                info!(target: LOG, height = h + 1, "consensus ready");
                let _ = reply.send((Height(h + 1), params));
            }

            AppMsg::StartedRound {
                height,
                round,
                reply_value,
                ..
            } => {
                debug!(target: LOG, %height, %round, "started round");
                self.current = (height.0, round_u32(round));
                self.streams.prune(Instant::now());
                let committed = self.committed.0;
                self.vsets.retain(|h, _| in_window(committed, *h));
                self.equivocations.retain(|(h, _)| *h > committed);
                self.validate_pending(height.0).await?;
                let values = self.stored_values(height.0, round).await?;
                let _ = reply_value.send(values);
                #[cfg(test)]
                if self.byzantine {
                    self.publish_junk(height.0, round).await;
                }
            }

            AppMsg::GetValue {
                height,
                round,
                timeout,
                reply,
            } => self.get_value(height.0, round, timeout, reply).await?,

            AppMsg::ExtendVote { reply, .. } => {
                let _ = reply.send(None);
            }

            AppMsg::VerifyVoteExtensions {
                extensions, reply, ..
            } => {
                let _ = reply.send(extensions.into_iter().map(|(a, _)| (a, Ok(()))).collect());
            }

            AppMsg::RestreamProposal {
                height,
                round,
                valid_round,
                value_id,
                ..
            } => match self.store.get_block(height.0, value_id)? {
                Some(block) => self.stream(height.0, round, valid_round, &block).await,
                None => warn!(target: LOG, %height, %value_id, "restream: block not held"),
            },

            AppMsg::GetHistoryMinHeight { reply } => {
                let h = match self.app.earliest_height().await {
                    Ok(h) => h,
                    Err(e) => {
                        warn!(target: LOG, "earliest_height: {e:#}");
                        self.committed.0 + 1
                    }
                };
                let _ = reply.send(Height(h));
            }

            AppMsg::ReceivedProposalPart {
                from,
                published_by,
                part,
                reply,
            } => {
                let publisher = published_by.unwrap_or(from);
                let value = self.on_part(from, publisher, part).await?;
                let _ = reply.send(value);
            }

            AppMsg::Decided {
                certificate, reply, ..
            } => {
                let h = certificate.height.0;
                let cert = to_xdevs_cert(&certificate);
                let block = self.decided_block(h, certificate.value_id).await?;
                match self.app.commit(block.clone(), cert.clone()).await {
                    Ok(()) => {
                        info!(target: LOG, height = h, hash = %cert.block_hash, round = cert.round,
                              signatures = cert.signatures.len(), "committed");
                        self.committed = (h, cert.block_hash);
                        self.pending_commit = None;
                    }
                    Err(e) => {
                        error!(target: LOG, height = h, "commit failed (retried at Finalized): {e:#}");
                        self.pending_commit = Some((block, cert));
                    }
                }
                let _ = reply.send(());
            }

            AppMsg::Finalized {
                certificate,
                evidence,
                reply,
                ..
            } => {
                for e in convert_evidence(&evidence) {
                    self.report(e).await;
                }
                let next = self.on_finalized(&certificate).await?;
                let _ = reply.send(next);
            }

            AppMsg::GetDecidedValues { range, reply } => {
                // Served off the consensus loop: up to 8 MB of reads per request.
                let app = self.app.clone();
                let permits = self.sync_permits.clone();
                let (start, end) = (range.start().0, range.end().0);
                tokio::spawn(async move {
                    let Ok(_permit) = permits.acquire_owned().await else {
                        return;
                    };
                    let values = decided_values(app.as_ref(), start, end).await;
                    let _ = reply.send(values);
                });
            }

            AppMsg::ProcessSyncedValue {
                height,
                round,
                proposer,
                value_bytes,
                reply,
            } => {
                let outcome = self
                    .synced_value(height.0, round, proposer, &value_bytes)
                    .await?;
                let _ = reply.send(outcome);
            }
        }
        Ok(())
    }

    async fn report(&self, e: Evidence) {
        warn!(target: LOG, validator = %e.validator(), height = e.height(), "equivocation evidence");
        if let Err(err) = self.app.report_evidence(e).await {
            warn!(target: LOG, "report_evidence: {err:#}");
        }
    }

    /// `Finalized`: make sure the decided block is committed, then start the next
    /// height. A decided height is never restarted: Malachite would reset its WAL and
    /// rerun round 0, which the SignGuard refuses (a validator would halt, a full node
    /// loop). A commit that keeps failing is retried with backoff, then the engine halts
    /// with an explicit error.
    async fn on_finalized(
        &mut self,
        certificate: &arc_malachitebft_app_channel::app::types::core::CommitCertificate<
            XdevsContext,
        >,
    ) -> anyhow::Result<Next<XdevsContext>> {
        let h = certificate.height.0;
        let cert = to_xdevs_cert(certificate);
        match self.pending_commit.take() {
            Some((block, _)) => {
                self.commit_with_retry(h, block, cert).await?;
                info!(target: LOG, height = h, hash = %certificate.value_id, "committed (retry)");
                self.committed = (h, certificate.value_id);
            }
            None => {
                // Certificate upgrade only (the app keeps the larger one).
                if let Some(block) = self.store.get_block(h, certificate.value_id)? {
                    if let Err(e) = self.app.commit(block, cert).await {
                        warn!(target: LOG, height = h, "certificate upgrade: {e:#}");
                    }
                }
            }
        }
        self.store.prune(h)?;
        Ok(Next::Start(Height(h + 1), self.height_params(h + 1).await?))
    }

    async fn commit_with_retry(
        &self,
        height: u64,
        block: Block,
        cert: crate::CommitCertificate,
    ) -> anyhow::Result<()> {
        let mut delay = self.commit_backoff;
        let mut last = None;
        for attempt in 1..=COMMIT_ATTEMPTS {
            match self.app.commit(block.clone(), cert.clone()).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    error!(target: LOG, height, attempt, "commit retry failed: {e:#}");
                    last = Some(e);
                }
            }
            if attempt < COMMIT_ATTEMPTS {
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
        }
        Err(last.expect("at least one attempt").context(format!(
            "commit of decided height {height} failed {COMMIT_ATTEMPTS} times; halting \
             (a decided height is never restarted)"
        )))
    }

    /// Admission of a stream's `Init` (and re-check of a complete stream).
    async fn check_stream(
        &mut self,
        height: u64,
        round: Round,
        proposer: &ValidatorAddress,
        publisher_validator: Option<&ValidatorAddress>,
    ) -> Result<(), Refusal> {
        // Cheap checks first: no validator-set lookup outside the window.
        if !in_window(self.committed.0, height) {
            return Err(Refusal::HeightOutsideWindow);
        }
        let set = match self.validator_set(height).await {
            Ok(set) => set,
            Err(e) => {
                warn!(target: LOG, height, "validator set: {e:#}");
                return Err(Refusal::NoValidatorSet);
            }
        };
        admit(
            self.committed.0,
            self.current,
            height,
            round,
            proposer,
            publisher_validator,
            set.inner(),
        )
    }

    /// The accounting class of `publisher`, and the validator its identity proved (if
    /// that validator is in the next height's set).
    fn publisher_class(&self, publisher: &PeerId) -> (PublisherClass, Option<ValidatorAddress>) {
        let bound = self
            .peer_validators
            .read()
            .ok()
            .and_then(|m| m.get(publisher).copied());
        let next = self.committed.0 + 1;
        match bound {
            Some(addr)
                if self
                    .vsets
                    .get(&next)
                    .is_some_and(|s| s.inner().get(&addr).is_some()) =>
            {
                (PublisherClass::Validator, Some(addr))
            }
            _ => (PublisherClass::Unbound, None),
        }
    }

    /// One proposal part from the network.
    async fn on_part(
        &mut self,
        from: PeerId,
        publisher: PeerId,
        part: StreamMessage<ProposalPart>,
    ) -> anyhow::Result<Option<ProposedValue<XdevsContext>>> {
        let now = Instant::now();
        let (class, bound) = self.publisher_class(&publisher);
        if let StreamContent::Data(ProposalPart::Init {
            height,
            round,
            proposer,
            ..
        }) = &part.content
        {
            // Checked before anything of the stream is buffered.
            if let Err(why) = self
                .check_stream(height.0, *round, proposer, bound.as_ref())
                .await
            {
                debug!(target: LOG, %from, %publisher, height = height.0, %round, ?why, "refused proposal stream");
                self.streams.reject(publisher, &part.stream_id, now);
                return Ok(None);
            }
        }
        match self.streams.insert(publisher, class, part, now) {
            Ok(Some(stream)) => self.on_stream(stream).await,
            Ok(None) => Ok(None),
            Err(reason) => {
                debug!(target: LOG, %from, %publisher, ?reason, "dropped proposal part");
                Ok(None)
            }
        }
    }

    /// Validated proposals stored for `(height, round)` from that round's proposer
    /// (crash recovery). An invalid verdict is re-checked (it may have depended on the
    /// clock); only valid verdicts are trusted as stored.
    async fn stored_values(
        &mut self,
        height: u64,
        round: Round,
    ) -> anyhow::Result<Vec<ProposedValue<XdevsContext>>> {
        let r = round_u32(round);
        let expected = self.proposer_of(height, r).await?;
        let mut out = Vec::new();
        for (pr, hash, mut m) in self.store.proposals_at_height(height)? {
            if pr != r || m.proposer != expected || m.validity == ProposalMeta::PENDING {
                continue;
            }
            if m.validity == ProposalMeta::INVALID && height == self.committed.0 + 1 {
                if let Some(block) = self.store.get_block(height, hash)? {
                    if let Ok(v) = self
                        .verdict(height, hash, &block, round_dec(m.valid_round))
                        .await
                    {
                        m.validity = meta_validity(v);
                        self.store.put_proposal(height, r, hash, m)?;
                    }
                }
            }
            out.push(ProposedValue {
                height: Height(height),
                round,
                valid_round: round_dec(m.valid_round),
                proposer: m.proposer,
                value: BlockValue::new(hash),
                validity: MValidity::from_bool(m.validity == ProposalMeta::VALID),
            });
        }
        Ok(out)
    }

    /// Validate blocks received for `height` before it became the next height (only
    /// those streamed by the proposer of their round).
    async fn validate_pending(&mut self, height: u64) -> anyhow::Result<()> {
        if height != self.committed.0 + 1 {
            return Ok(());
        }
        for (round, hash, meta) in self.store.proposals_at_height(height)? {
            if meta.validity != ProposalMeta::PENDING
                || meta.proposer != self.proposer_of(height, round).await?
            {
                continue;
            }
            let Some(block) = self.store.get_block(height, hash)? else {
                continue;
            };
            match self
                .verdict(height, hash, &block, round_dec(meta.valid_round))
                .await
            {
                Ok(v) => self.store.put_proposal(
                    height,
                    round,
                    hash,
                    ProposalMeta {
                        validity: meta_validity(v),
                        ..meta
                    },
                )?,
                Err(e) => warn!(target: LOG, height, %hash, "validate pending: {e:#}"),
            }
        }
        Ok(())
    }

    /// `app.validate` of a proposal (POL round `pol_round`). Only a stored VALID verdict
    /// is reused: an invalid one may be clock-dependent (the app caches its
    /// deterministic verdicts itself). A valid verdict reached without the clock rule
    /// (a re-proposal) is not reused for a fresh proposal.
    async fn verdict(
        &self,
        height: u64,
        hash: B256,
        block: &Block,
        pol_round: Round,
    ) -> anyhow::Result<crate::Validity> {
        let origin = origin_for(pol_round);
        let known_valid = self
            .store
            .proposals_at_height(height)?
            .into_iter()
            .any(|(_, h, m)| {
                h == hash
                    && m.validity == ProposalMeta::VALID
                    && (origin == BlockOrigin::Sync || m.valid_round == round_enc(Round::Nil))
            });
        if known_valid {
            return Ok(crate::Validity::Valid);
        }
        self.app.validate(height, block, origin).await
    }

    /// Our own block for `height` built on the current head, if we already made one
    /// (re-proposing it keeps restarts deterministic: same value, same signature).
    fn own_block(&self, height: u64) -> anyhow::Result<Option<Block>> {
        for (_, hash, meta) in self.store.proposals_at_height(height)? {
            if meta.proposer != self.address {
                continue;
            }
            if let Some(block) = self.store.get_block(height, hash)? {
                if block.header.parent_hash == self.committed.1 {
                    return Ok(Some(block));
                }
            }
        }
        Ok(None)
    }

    async fn get_value(
        &mut self,
        height: u64,
        round: Round,
        timeout: Duration,
        reply: tokio::sync::oneshot::Sender<LocallyProposedValue<XdevsContext>>,
    ) -> anyhow::Result<()> {
        if self.key.is_none() {
            warn!(target: LOG, height, "GetValue on a full node; ignored");
            return Ok(());
        }
        let block = match self.own_block(height)? {
            Some(b) => b,
            None => {
                let deadline = Instant::now() + timeout.mul_f64(0.8);
                match tokio::time::timeout(
                    timeout,
                    self.app.propose(height, round_u32(round), deadline),
                )
                .await
                {
                    Ok(Ok(b)) => b,
                    Ok(Err(e)) => {
                        warn!(target: LOG, height, %round, "propose failed: {e:#}");
                        return Ok(());
                    }
                    Err(_) => {
                        warn!(target: LOG, height, %round, "propose timed out");
                        return Ok(());
                    }
                }
            }
        };
        if block.number() != height {
            warn!(target: LOG, height, got = block.number(), "app proposed a block for another height");
            return Ok(());
        }
        if !body_matches_header(&block) {
            warn!(target: LOG, height, "app proposed a block whose body does not match its header");
            return Ok(());
        }
        let hash = block.hash();
        self.store.put_block(height, hash, &block)?;
        self.store.put_proposal(
            height,
            round_u32(round),
            hash,
            ProposalMeta {
                valid_round: round_enc(Round::Nil),
                proposer: self.address,
                validity: ProposalMeta::VALID,
            },
        )?;
        debug!(target: LOG, height, %round, %hash, "proposing");
        let _ = reply.send(LocallyProposedValue::new(
            Height(height),
            round,
            BlockValue::new(hash),
        ));
        self.stream(height, round, Round::Nil, &block).await;
        Ok(())
    }

    /// Publish `block` as a signed part stream for `(height, round)`.
    async fn stream(&mut self, height: u64, round: Round, pol_round: Round, block: &Block) {
        let Some(key) = &self.key else {
            return;
        };
        let chain_id = self.chain_id;
        let bytes = encode_block(block);
        let parts = make_parts(height, round, pol_round, self.address, &bytes, |data| {
            key.sign(&parts_sign_bytes(
                chain_id,
                height,
                round_u32(round),
                keccak256(data),
            ))
            .to_bytes()
        });
        self.publish(height, round, parts).await;
    }

    async fn publish(&mut self, height: u64, round: Round, parts: Vec<ProposalPart>) {
        self.stream_nonce = self.stream_nonce.wrapping_add(1);
        let mut id = Vec::with_capacity(20);
        id.extend_from_slice(&height.to_be_bytes());
        id.extend_from_slice(&round_u32(round).to_be_bytes());
        id.extend_from_slice(&self.stream_nonce.to_be_bytes());
        for msg in stream_messages(StreamId::new(id.into()), parts) {
            if self
                .network
                .send(NetworkMsg::PublishProposalPart(msg))
                .await
                .is_err()
            {
                warn!(target: LOG, height, "network channel closed while streaming");
                return;
            }
        }
    }

    /// A complete part stream: re-check admission, authenticate, decode (the body must
    /// match the header), keep one block per `(height, round, proposer)`, validate, store.
    async fn on_stream(
        &mut self,
        s: CompleteStream,
    ) -> anyhow::Result<Option<ProposedValue<XdevsContext>>> {
        let height = s.height;
        // `committed` may have moved since the Init was admitted.
        if let Err(why) = self.check_stream(height, s.round, &s.proposer, None).await {
            debug!(target: LOG, height, round = %s.round, ?why, "complete stream no longer admissible");
            return Ok(None);
        }
        let set = self.validator_set(height).await?;
        let Some(streamer) = set.inner().get(&s.proposer) else {
            return Ok(None); // unreachable: the proposer is a member
        };
        let round = round_u32(s.round);
        let msg = parts_sign_bytes(self.chain_id, height, round, keccak256(&s.data));
        if !streamer.public_key.verify(&msg, &s.signature) {
            warn!(target: LOG, height, round, proposer = %s.proposer, "bad proposal-parts signature");
            return Ok(None);
        }
        let block = match decode_block(&s.data) {
            Ok(b) if b.number() == height => b,
            Ok(b) => {
                warn!(target: LOG, height, got = b.number(), "streamed block for another height");
                return Ok(None);
            }
            Err(e) => {
                warn!(target: LOG, height, proposer = %s.proposer, "invalid streamed block: {e}");
                return Ok(None);
            }
        };
        let hash = block.hash();
        // One block per (height, round, proposer): a different second one is equivocation.
        let earlier = self
            .store
            .proposals_at_height(height)?
            .into_iter()
            .find(|(r, _, m)| *r == round && m.proposer == s.proposer);
        let mut meta = ProposalMeta {
            valid_round: round_enc(s.pol_round),
            proposer: s.proposer,
            validity: ProposalMeta::PENDING,
        };
        if let Some((_, first, first_meta)) = earlier {
            if first != hash {
                warn!(target: LOG, height, round, proposer = %s.proposer, %first, second = %hash,
                      "proposer streamed two different blocks for one round; keeping the first");
                if self.equivocations.insert((height, round)) {
                    self.report(Evidence::DoubleProposal {
                        validator: s.proposer,
                        height,
                        round,
                        block_hashes: (first, hash),
                    })
                    .await;
                }
                return Ok(None);
            }
            // The same block again (another stream id): keep the first metadata.
            meta.valid_round = first_meta.valid_round;
        }
        let pol_round = round_dec(meta.valid_round);
        self.store.put_block(height, hash, &block)?;
        if height > self.committed.0 + 1 {
            // Its parent is not committed yet: validated when the height starts.
            debug!(target: LOG, height, %hash, "buffered proposal for the next height");
            self.store.put_proposal(height, round, hash, meta)?;
            return Ok(None);
        }
        let validity = match self.verdict(height, hash, &block, pol_round).await {
            Ok(v) => v,
            Err(e) => {
                // No verdict: Malachite prevotes nil when timeout_propose fires.
                warn!(target: LOG, height, %hash, "validate failed: {e:#}");
                return Ok(None);
            }
        };
        meta.validity = meta_validity(validity);
        self.store.put_proposal(height, round, hash, meta)?;
        Ok(Some(ProposedValue {
            height: Height(height),
            round: s.round,
            valid_round: pol_round,
            proposer: s.proposer,
            value: BlockValue::new(hash),
            validity: to_m_validity(validity),
        }))
    }

    /// The decided block: from the undecided store (only blocks whose body matches
    /// their header are ever stored), else from the app (committed before a crash).
    async fn decided_block(&self, height: u64, hash: B256) -> anyhow::Result<Block> {
        if let Some(b) = self.store.get_block(height, hash)? {
            return Ok(b);
        }
        match self.app.decided(height).await {
            Ok(Some((b, _))) if b.hash() == hash => Ok(b),
            _ => bail!("decided block {hash} at height {height} is not available"),
        }
    }

    async fn synced_value(
        &mut self,
        height: u64,
        round: Round,
        proposer: ValidatorAddress,
        value_bytes: &[u8],
    ) -> anyhow::Result<SyncedValueOutcome<XdevsContext>> {
        // Undecodable, another height, or a body that does not match the header: the
        // peer served a wrong value.
        let block = match decode_block(value_bytes) {
            Ok(b) if b.number() == height => b,
            Ok(b) => {
                warn!(target: LOG, height, got = b.number(), "synced value for another height");
                return Ok(SyncedValueOutcome::PeerFault);
            }
            Err(e) => {
                warn!(target: LOG, height, "invalid synced value: {e}");
                return Ok(SyncedValueOutcome::PeerFault);
            }
        };
        if height != self.committed.0 + 1 {
            return Ok(SyncedValueOutcome::LocalTransientError);
        }
        let validity = match self.app.validate(height, &block, BlockOrigin::Sync).await {
            Ok(v) => v,
            Err(e) => {
                warn!(target: LOG, height, "validate synced block: {e:#}");
                return Ok(SyncedValueOutcome::LocalTransientError);
            }
        };
        let hash = block.hash();
        if validity.is_valid() {
            self.store.put_block(height, hash, &block)?;
        }
        debug!(target: LOG, height, %hash, ?validity, "synced value");
        Ok(SyncedValueOutcome::Verdict(ProposedValue {
            height: Height(height),
            round,
            valid_round: Round::Nil,
            proposer,
            value: BlockValue::new(hash),
            validity: to_m_validity(validity),
        }))
    }

    /// Tests: a Byzantine validator. On every round it publishes junk part streams:
    /// two different signed blocks per round claiming itself as proposer (wrong
    /// proposer, or equivocation where it is the proposer), a block claiming the real
    /// proposer but signed by itself, a body that does not match its header, the same for
    /// the next two heights (all ahead of the honest proposals), far-future and
    /// `u64::MAX` heights, and (every 4th height) an oversized stream.
    #[cfg(test)]
    async fn publish_junk(&mut self, height: u64, round: Round) {
        let Some(key) = self.key.clone() else {
            return;
        };
        let r = round_u32(round);
        let me = self.address;
        let junk = |h: u64, tag: u8, r: u32, mismatched: bool| {
            let mut extra = vec![0xEE, tag];
            extra.extend_from_slice(&r.to_be_bytes());
            let mut header = xdevs_primitives::Header {
                number: h,
                timestamp: h,
                parent_hash: B256::repeat_byte(tag),
                extra_data: extra.into(),
                ..Default::default()
            };
            if mismatched {
                header.transactions_root = B256::repeat_byte(0xAB);
            }
            encode_block(&Block::new(header, vec![]))
        };
        let mut streams: Vec<(u64, u32, ValidatorAddress, Vec<u8>)> = Vec::new();
        // Two per round, ahead of the honest proposals, for both heights of the window.
        for rr in r..r + 6 {
            let expected = match self.proposer_of(height, rr).await {
                Ok(p) => p,
                Err(_) => return,
            };
            streams.push((height, rr, me, junk(height, 1, rr, false)));
            streams.push((height, rr, me, junk(height, 2, rr, false)));
            streams.push((height, rr, expected, junk(height, 3, rr, false)));
            streams.push((height, rr, me, junk(height, 4, rr, true)));
        }
        for ahead in [1, 2] {
            for rr in 0..6 {
                let h = height + ahead;
                streams.push((h, rr, me, junk(h, 5, rr, false)));
                streams.push((h, rr, me, junk(h, 6, rr, false)));
            }
        }
        streams.push((height + 5, 0, me, junk(height + 5, 7, 0, false)));
        streams.push((u64::MAX, 0, me, junk(u64::MAX, 8, 0, false)));
        if r == 0 && height.is_multiple_of(4) {
            let big = vec![0x5A; super::streaming::MAX_BLOCK_BYTES + 2 * CHUNK_SIZE];
            streams.push((height, 0, me, big));
        }
        let chain_id = self.chain_id;
        for (h, rr, claimed, data) in streams {
            let parts = make_parts(h, Round::new(rr), Round::Nil, claimed, &data, |d| {
                key.sign(&parts_sign_bytes(chain_id, h, rr, keccak256(d)))
                    .to_bytes()
            });
            self.publish(h, Round::new(rr), parts).await;
        }
    }
}

/// Serve block sync: decided values `start..=end` (at most [`MAX_SYNC_VALUES`] and
/// [`MAX_SYNC_RESPONSE_BYTES`]). Runs on its own task.
async fn decided_values(
    app: &dyn ConsensusApp,
    start: u64,
    end: u64,
) -> Vec<RawDecidedValue<XdevsContext>> {
    let earliest = app.earliest_height().await.unwrap_or(u64::MAX);
    let head = match app.head().await {
        Ok((h, _)) => h,
        Err(_) => return Vec::new(),
    };
    // Malachite does not check the range; values must start at `start`.
    if start < earliest || start > end {
        return Vec::new();
    }
    let last = end.min(head).min(start.saturating_add(MAX_SYNC_VALUES - 1));
    let mut out = Vec::new();
    let mut total = 0usize;
    for h in start..=last {
        let Ok(Some((block, cert))) = app.decided(h).await else {
            break;
        };
        let bytes = encode_block(&block);
        total += bytes.len();
        if !out.is_empty() && total > MAX_SYNC_RESPONSE_BYTES {
            break;
        }
        out.push(RawDecidedValue::new(bytes.into(), to_malachite_cert(&cert)));
    }
    out
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

fn to_signed_vote(v: &SignedMessage<XdevsContext, Vote>) -> SignedVote {
    SignedVote {
        vote_type: match v.vote_type {
            MVoteType::Prevote => VoteType::Prevote,
            MVoteType::Precommit => VoteType::Precommit,
        },
        height: v.height.0,
        round: round_u32(v.round),
        block_hash: match v.value {
            NilOrVal::Nil => None,
            NilOrVal::Val(h) => Some(h),
        },
        validator: v.validator_address,
        signature: v.signature.to_bytes(),
    }
}

/// Malachite misbehaviour evidence → xdevs [`Evidence`].
pub(crate) fn convert_evidence(ev: &MisbehaviorEvidence<XdevsContext>) -> Vec<Evidence> {
    let mut out = Vec::new();
    for (_, votes) in ev.votes.iter() {
        for (a, b) in votes {
            out.push(Evidence::DoubleVote(ConflictingVotes {
                first: to_signed_vote(a),
                second: to_signed_vote(b),
            }));
        }
    }
    for (validator, proposals) in ev.proposals.iter() {
        for (a, b) in proposals {
            out.push(Evidence::DoubleProposal {
                validator: *validator,
                height: a.height.0,
                round: round_u32(a.round),
                block_hashes: (a.value.hash, b.value.hash),
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

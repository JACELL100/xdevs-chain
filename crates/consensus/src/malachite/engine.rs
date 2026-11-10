//! The `AppMsg` loop (research doc §6.3): drives a [`ConsensusApp`] from Malachite's
//! channel-based interface, plus the config mapping (§6.10, §7) and engine start / stop.

use super::codec::{decode_block, encode_block, to_malachite_cert, to_xdevs_cert, XdevsCodec};
use super::context::{round_u32, BlockValue, Height, MValidatorSet, Vote, XdevsContext};
use super::signer::{XdevsSigner, XdevsVerifier};
use super::store::{ConsensusStore, ProposalMeta};
use super::streaming::{make_parts, stream_messages, CompleteStream, PartStreams, CHUNK_SIZE};
use super::{linear_timeouts, private_key, PrivateKey};
use crate::app::{BlockOrigin, ConsensusApp};
use crate::config::EngineConfig;
use crate::engine::{EngineKeys, EngineStatus};
use crate::types::{
    parts_sign_bytes, ConflictingVotes, Evidence, SignedVote, ValidatorAddress, ValidatorKey,
    VoteType,
};
use anyhow::{anyhow, bail, Context as _};
use arc_malachitebft_app_channel::app::engine::util::events::{Event, RxEvent};
use arc_malachitebft_app_channel::app::{
    config::{
        ChannelNames, ConsensusConfig, GossipSubConfig, NodeConfig, ProtocolNames, PubSubProtocol,
        ValuePayload, ValueSyncConfig,
    },
    engine::host::{HeightParams, Next, SyncedValueOutcome},
    streaming::StreamId,
    types::{
        codec::Codec,
        core::{NilOrVal, Round, SignedMessage, Validity as MValidity, VoteType as MVoteType},
        Keypair, LocallyProposedValue, MisbehaviorEvidence, ProposedValue,
    },
};
use arc_malachitebft_app_channel::{
    start_engine, AppMsg, Channels, ConsensusContext, EngineHandle, NetworkContext,
    NetworkIdentity, NetworkMsg, NetworkRequest, RequestContext, Signer as _, SyncContext,
    WalContext,
};
use arc_malachitebft_sync::RawDecidedValue;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
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
    mut shutdown: watch::Receiver<bool>,
    status: EngineStatus,
) -> anyhow::Result<()> {
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
    let monitor = tokio::spawn(monitor_status(
        status.clone(),
        events.subscribe(),
        net_requests,
    ));
    let mut host = Host {
        chain_id: config.chain_id,
        app,
        store,
        key: started.key,
        address: started.address,
        timeouts: config.timeouts.clone(),
        network,
        streams: PartStreams::new(),
        vsets: BTreeMap::new(),
        committed: (0, B256::ZERO),
        pending_commit: None,
        stream_nonce: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    };
    let result = host.run(rx, &mut shutdown).await;
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

/// Keep [`EngineStatus`] current: heights from engine events, the peer count from a
/// network state dump every second. Aborted when the engine stops.
async fn monitor_status(
    status: EngineStatus,
    mut events: RxEvent<XdevsContext>,
    net: mpsc::Sender<NetworkRequest>,
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
                }
            }
        }
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

struct Host {
    chain_id: u64,
    app: Arc<dyn ConsensusApp>,
    store: Arc<ConsensusStore>,
    key: Option<PrivateKey>,
    address: ValidatorAddress,
    timeouts: crate::config::ConsensusTimeouts,
    network: mpsc::Sender<NetworkMsg<XdevsContext>>,
    streams: PartStreams,
    vsets: BTreeMap<u64, MValidatorSet>,
    /// Last committed `(height, hash)`.
    committed: (u64, B256),
    /// A decision whose commit failed at `Decided`; retried at `Finalized`.
    pending_commit: Option<(Block, crate::CommitCertificate)>,
    stream_nonce: u64,
}

impl Host {
    async fn run(
        &mut self,
        mut rx: mpsc::Receiver<AppMsg<XdevsContext>>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                biased;
                // A dropped sender counts as a shutdown request.
                _ = async { let _ = shutdown.wait_for(|stop| *stop).await; } => return Ok(()),
                msg = rx.recv() => match msg {
                    Some(msg) => self.handle(msg).await?,
                    None => bail!("Malachite stopped (consensus channel closed)"),
                },
            }
        }
    }

    async fn validator_set(&mut self, height: u64) -> anyhow::Result<MValidatorSet> {
        if let Some(s) = self.vsets.get(&height) {
            return Ok(s.clone());
        }
        let set = MValidatorSet::new(self.app.validator_set(height).await?)?;
        self.vsets.insert(height, set.clone());
        Ok(set)
    }

    /// Parameters of `height`. The block time is Malachite's `target_time` (minimum
    /// height duration) only while this node is at the tip. If it already holds a
    /// proposal for `height + 1` (stored only after the streamer's signature verified;
    /// a proposer builds on a committed parent), the network has decided `height` and
    /// this node is catching up: it must finalize as soon as it decides, or a node that
    /// decides from buffered live votes (instead of ValueSync) advances exactly at the
    /// chain's pace and never closes its lag. Only the next height counts, so a stray
    /// far-future stream cannot switch pacing off. Pacing only, never safety.
    async fn height_params(&mut self, height: u64) -> anyhow::Result<HeightParams<XdevsContext>> {
        let catching_up = self.store.has_proposal_at(height.saturating_add(1))?;
        if catching_up {
            debug!(target: LOG, height, "catching up: no minimum height duration");
        }
        Ok(HeightParams::new(
            self.validator_set(height).await?,
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
                self.streams.prune(height.0, Instant::now());
                self.vsets.retain(|h, _| *h >= height.0);
                self.validate_pending(height.0).await?;
                let values = self.stored_values(height.0, round)?;
                let _ = reply_value.send(values);
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
                let value = match self.streams.insert(from, publisher, part, Instant::now()) {
                    Ok(Some(stream)) => self.on_stream(stream).await?,
                    Ok(None) => None,
                    Err(reason) => {
                        debug!(target: LOG, %from, ?reason, "dropped proposal part");
                        None
                    }
                };
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
                        error!(target: LOG, height = h, "commit failed: {e:#}");
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
                let h = certificate.height.0;
                let cert = to_xdevs_cert(&certificate);
                let committed = match self.pending_commit.take() {
                    Some((block, _)) => match self.app.commit(block, cert).await {
                        Ok(()) => {
                            self.committed = (h, certificate.value_id);
                            true
                        }
                        Err(e) => {
                            error!(target: LOG, height = h, "commit retry failed: {e:#}");
                            false
                        }
                    },
                    None => {
                        // Certificate upgrade only (the app keeps the larger one).
                        if let Some(block) = self.store.get_block(h, certificate.value_id)? {
                            if let Err(e) = self.app.commit(block, cert).await {
                                warn!(target: LOG, height = h, "certificate upgrade: {e:#}");
                            }
                        }
                        true
                    }
                };
                for e in convert_evidence(&evidence) {
                    warn!(target: LOG, validator = %e.validator(), height = e.height(), "equivocation evidence");
                    if let Err(err) = self.app.report_evidence(e).await {
                        warn!(target: LOG, "report_evidence: {err:#}");
                    }
                }
                let next = if committed {
                    self.store.prune(h)?;
                    Next::Start(Height(h + 1), self.height_params(h + 1).await?)
                } else {
                    Next::Restart(Height(h), self.height_params(h).await?)
                };
                let _ = reply.send(next);
            }

            AppMsg::GetDecidedValues { range, reply } => {
                let values = self.decided_values(range.start().0, range.end().0).await;
                let _ = reply.send(values);
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

    /// Validated proposals stored for `(height, round)` (crash recovery).
    fn stored_values(
        &self,
        height: u64,
        round: Round,
    ) -> anyhow::Result<Vec<ProposedValue<XdevsContext>>> {
        let r = round_u32(round);
        Ok(self
            .store
            .proposals_at_height(height)?
            .into_iter()
            .filter(|(pr, _, m)| *pr == r && m.validity != ProposalMeta::PENDING)
            .map(|(_, hash, m)| ProposedValue {
                height: Height(height),
                round,
                valid_round: round_dec(m.valid_round),
                proposer: m.proposer,
                value: BlockValue::new(hash),
                validity: MValidity::from_bool(m.validity == ProposalMeta::VALID),
            })
            .collect())
    }

    /// Validate blocks received for `height` before it became the next height.
    async fn validate_pending(&mut self, height: u64) -> anyhow::Result<()> {
        if height != self.committed.0 + 1 {
            return Ok(());
        }
        for (round, hash, meta) in self.store.proposals_at_height(height)? {
            if meta.validity != ProposalMeta::PENDING {
                continue;
            }
            let Some(block) = self.store.get_block(height, hash)? else {
                continue;
            };
            match self.verdict(height, hash, &block).await {
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

    /// `app.validate` (Consensus origin), reusing an earlier verdict for the same block.
    async fn verdict(
        &self,
        height: u64,
        hash: B256,
        block: &Block,
    ) -> anyhow::Result<crate::Validity> {
        let known = self
            .store
            .proposals_at_height(height)?
            .into_iter()
            .find(|(_, h, m)| *h == hash && m.validity != ProposalMeta::PENDING);
        if let Some((_, _, m)) = known {
            return Ok(crate::Validity::from_bool(
                m.validity == ProposalMeta::VALID,
            ));
        }
        self.app
            .validate(height, block, BlockOrigin::Consensus)
            .await
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

    /// A complete part stream: authenticate, decode, validate, store.
    async fn on_stream(
        &mut self,
        s: CompleteStream,
    ) -> anyhow::Result<Option<ProposedValue<XdevsContext>>> {
        let height = s.height;
        if height <= self.committed.0 {
            return Ok(None);
        }
        let set = match self.validator_set(height).await {
            Ok(set) => set,
            Err(e) => {
                warn!(target: LOG, height, "validator set: {e:#}");
                return Ok(None);
            }
        };
        let Some(streamer) = set.inner().get(&s.proposer) else {
            warn!(target: LOG, height, proposer = %s.proposer, "stream from a non-validator");
            return Ok(None);
        };
        let msg = parts_sign_bytes(
            self.chain_id,
            height,
            round_u32(s.round),
            keccak256(&s.data),
        );
        if !streamer.public_key.verify(&msg, &s.signature) {
            warn!(target: LOG, height, proposer = %s.proposer, "bad proposal-parts signature");
            return Ok(None);
        }
        let block = match decode_block(&s.data) {
            Ok(b) if b.number() == height => b,
            Ok(b) => {
                warn!(target: LOG, height, got = b.number(), "streamed block for another height");
                return Ok(None);
            }
            Err(e) => {
                warn!(target: LOG, height, "undecodable streamed block: {e}");
                return Ok(None);
            }
        };
        let hash = block.hash();
        self.store.put_block(height, hash, &block)?;
        let mut meta = ProposalMeta {
            valid_round: round_enc(s.pol_round),
            proposer: s.proposer,
            validity: ProposalMeta::PENDING,
        };
        let round = round_u32(s.round);
        if height > self.committed.0 + 1 {
            // Its parent is not committed yet: validated when the height starts.
            debug!(target: LOG, height, %hash, "buffered proposal for a future height");
            self.store.put_proposal(height, round, hash, meta)?;
            return Ok(None);
        }
        let validity = match self.verdict(height, hash, &block).await {
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
            valid_round: s.pol_round,
            proposer: s.proposer,
            value: BlockValue::new(hash),
            validity: to_m_validity(validity),
        }))
    }

    /// The decided block: from the undecided store, else from the app (already
    /// committed before a crash).
    async fn decided_block(&self, height: u64, hash: B256) -> anyhow::Result<Block> {
        if let Some(b) = self.store.get_block(height, hash)? {
            return Ok(b);
        }
        match self.app.decided(height).await {
            Ok(Some((b, _))) if b.hash() == hash => Ok(b),
            _ => bail!("decided block {hash} at height {height} is not available"),
        }
    }

    async fn decided_values(&self, start: u64, end: u64) -> Vec<RawDecidedValue<XdevsContext>> {
        let earliest = self.app.earliest_height().await.unwrap_or(u64::MAX);
        let head = match self.app.head().await {
            Ok((h, _)) => h,
            Err(_) => return Vec::new(),
        };
        // Malachite does not check the range; values must start at `start`.
        if start < earliest || start > end {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut total = 0usize;
        for h in start..=end.min(head) {
            let Ok(Some((block, cert))) = self.app.decided(h).await else {
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

    async fn synced_value(
        &mut self,
        height: u64,
        round: Round,
        proposer: ValidatorAddress,
        value_bytes: &[u8],
    ) -> anyhow::Result<SyncedValueOutcome<XdevsContext>> {
        let block = match decode_block(value_bytes) {
            Ok(b) if b.number() == height => b,
            _ => {
                warn!(target: LOG, height, "undecodable synced value");
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
        self.store.put_block(height, hash, &block)?;
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
mod tests {
    use super::*;
    use crate::config::ConsensusTimeouts;

    #[test]
    fn config_mapping() {
        let mut c = EngineConfig::new(7117, "/data");
        c.p2p.listen = "/ip4/127.0.0.1/tcp/30333".into();
        c.p2p.persistent_peers = vec!["/ip4/127.0.0.1/tcp/30334".into()];
        let n = node_config(&c).unwrap();
        assert_eq!(n.moniker(), "xdevs");
        let p2p = &n.consensus().p2p;
        assert_eq!(p2p.channel_names.consensus, "/xdevs/7117/consensus");
        assert_eq!(
            p2p.channel_names.proposal_parts,
            "/xdevs/7117/proposal_parts"
        );
        assert_eq!(p2p.channel_names.sync, "/xdevs/7117/sync");
        assert_eq!(p2p.channel_names.liveness, "/xdevs/7117/liveness");
        p2p.channel_names.validate().unwrap();
        assert!(p2p.protocol_names.sync.starts_with("/xdevs/7117/"));
        assert_eq!(p2p.persistent_peers.len(), 1);
        assert!(!p2p.discovery.enabled);
        assert_eq!(
            p2p.pubsub_max_size_per_topic
                .proposal_parts
                .unwrap()
                .as_u64(),
            (260 * 1024) as u64
        );
        assert_eq!(n.consensus().value_payload, ValuePayload::ProposalAndParts);
        assert!(n.value_sync().enabled);
        assert_eq!(
            n.value_sync().status_update_interval,
            Duration::from_secs(10)
        );

        c.timeouts = ConsensusTimeouts::fast_for_tests(100);
        c.value_sync = false;
        let n = node_config(&c).unwrap();
        assert_eq!(
            n.value_sync().status_update_interval,
            Duration::from_secs(1)
        );
        assert!(!n.value_sync().enabled);

        c.p2p.listen = "not a multiaddr".into();
        assert!(node_config(&c).is_err());
        c.p2p.listen = "/ip4/127.0.0.1/tcp/1".into();
        c.timeouts.timeout_propose_ms = 0;
        assert!(node_config(&c).is_err());
    }

    #[test]
    fn no_target_time_while_catching_up() {
        let t = ConsensusTimeouts::fast_for_tests(400);
        assert_eq!(target_time(&t, false), Some(Duration::from_millis(400)));
        assert_eq!(target_time(&t, true), None);
    }

    #[test]
    fn rounds_encode_for_meta() {
        assert_eq!(round_dec(round_enc(Round::Nil)), Round::Nil);
        assert_eq!(round_dec(round_enc(Round::new(7))), Round::new(7));
    }
}

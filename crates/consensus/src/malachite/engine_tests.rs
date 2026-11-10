//! Unit tests of the Malachite host (`engine.rs`): config mapping and the security
//! rules of the `AppMsg` loop, driven directly on a [`Host`] with a mock app.

use super::*;
use crate::config::ConsensusTimeouts;
use crate::malachite::context::tests::test_set;
use crate::malachite::streaming::tests::rp;
use crate::Validity;
use arc_malachitebft_app_channel::app::types::core::{
    CommitCertificate as MCommitCertificate, Context as _,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use xdevs_primitives::Header;

const CHAIN: u64 = 7117;

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

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A scripted app: every block is valid unless its hash is in `invalid`.
struct MockApp {
    set: ValidatorSet,
    head: Mutex<(u64, B256)>,
    invalid: Mutex<HashSet<B256>>,
    validations: Mutex<Vec<(u64, B256, BlockOrigin)>>,
    /// Commits still to fail.
    commit_failures: Mutex<usize>,
    commit_attempts: AtomicUsize,
    evidence: Mutex<Vec<Evidence>>,
    decided: Mutex<BTreeMap<u64, (Block, crate::CommitCertificate)>>,
    /// While set, `decided` waits for `gate`.
    block_decided: AtomicBool,
    gate: tokio::sync::Notify,
}

impl MockApp {
    fn new(set: ValidatorSet) -> Self {
        Self {
            set,
            head: Mutex::new((0, B256::ZERO)),
            invalid: Mutex::default(),
            validations: Mutex::default(),
            commit_failures: Mutex::new(0),
            commit_attempts: AtomicUsize::new(0),
            evidence: Mutex::default(),
            decided: Mutex::default(),
            block_decided: AtomicBool::new(false),
            gate: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl ConsensusApp for MockApp {
    async fn head(&self) -> anyhow::Result<(u64, B256)> {
        Ok(*self.head.lock().unwrap())
    }

    async fn validator_set(&self, _height: u64) -> anyhow::Result<ValidatorSet> {
        Ok(self.set.clone())
    }

    async fn propose(&self, height: u64, _round: u32, _deadline: Instant) -> anyhow::Result<Block> {
        Ok(block(height, 0))
    }

    async fn validate(
        &self,
        height: u64,
        block: &Block,
        origin: BlockOrigin,
    ) -> anyhow::Result<Validity> {
        let hash = block.hash();
        self.validations
            .lock()
            .unwrap()
            .push((height, hash, origin));
        Ok(Validity::from_bool(
            !self.invalid.lock().unwrap().contains(&hash),
        ))
    }

    async fn commit(&self, block: Block, _cert: crate::CommitCertificate) -> anyhow::Result<()> {
        self.commit_attempts.fetch_add(1, Ordering::SeqCst);
        {
            let mut left = self.commit_failures.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                anyhow::bail!("disk on fire");
            }
        }
        *self.head.lock().unwrap() = (block.number(), block.hash());
        Ok(())
    }

    async fn decided(
        &self,
        height: u64,
    ) -> anyhow::Result<Option<(Block, crate::CommitCertificate)>> {
        if self.block_decided.load(Ordering::SeqCst) {
            self.gate.notified().await;
        }
        Ok(self.decided.lock().unwrap().get(&height).cloned())
    }

    async fn earliest_height(&self) -> anyhow::Result<u64> {
        Ok(1)
    }

    async fn report_evidence(&self, e: Evidence) -> anyhow::Result<()> {
        self.evidence.lock().unwrap().push(e);
        Ok(())
    }
}

struct Harness {
    host: Host,
    app: Arc<MockApp>,
    keys: Vec<PrivateKey>,
    set: MValidatorSet,
    _net: mpsc::Receiver<NetworkMsg<XdevsContext>>,
    _dir: tempfile::TempDir,
}

fn addr(k: &PrivateKey) -> ValidatorAddress {
    ValidatorKey(*k.public_key().as_bytes()).address()
}

fn block(height: u64, tag: u8) -> Block {
    Block::new(
        Header {
            number: height,
            timestamp: height,
            extra_data: vec![tag].into(),
            ..Default::default()
        },
        vec![],
    )
}

/// A host at height 1 (committed 0, round 0) with four validators; it is validator 0.
fn harness() -> Harness {
    let (keys, set) = test_set(4);
    let app = Arc::new(MockApp::new(set.inner().clone()));
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ConsensusStore::open(&dir.path().join("c.redb")).unwrap());
    let (tx, rx) = mpsc::channel(4096);
    let mut host = Host::new(
        CHAIN,
        app.clone(),
        store,
        Some(keys[0].clone()),
        addr(&keys[0]),
        ConsensusTimeouts::fast_for_tests(100),
        tx,
        PeerValidators::default(),
    );
    host.commit_backoff = Duration::from_millis(1);
    host.current = (1, 0);
    Harness {
        host,
        app,
        keys,
        set,
        _net: rx,
        _dir: dir,
    }
}

impl Harness {
    /// The key of the §6.6 proposer of `(height, round)`.
    fn proposer_key(&self, height: u64, round: u32) -> PrivateKey {
        let p = self.set.inner().select_proposer(height, round).address;
        self.keys.iter().find(|k| addr(k) == p).unwrap().clone()
    }

    fn other_key(&self, height: u64, round: u32) -> PrivateKey {
        let p = self.set.inner().select_proposer(height, round).address;
        self.keys.iter().find(|k| addr(k) != p).unwrap().clone()
    }

    async fn feed(
        &mut self,
        publisher: PeerId,
        msgs: Vec<StreamMessage<ProposalPart>>,
    ) -> Option<ProposedValue<XdevsContext>> {
        let mut last = None;
        for m in msgs {
            last = self.host.on_part(publisher, publisher, m).await.unwrap();
        }
        last
    }

    fn stored(&self, height: u64) -> Vec<(u32, B256, ProposalMeta)> {
        self.host.store.proposals_at_height(height).unwrap()
    }
}

/// A part stream for `(height, round)` claiming `claimed`, signed by `signer`.
fn signed_stream(
    signer: &PrivateKey,
    height: u64,
    round: u32,
    pol_round: Round,
    claimed: ValidatorAddress,
    data: &[u8],
) -> Vec<StreamMessage<ProposalPart>> {
    use std::sync::atomic::AtomicU32;
    static ID: AtomicU32 = AtomicU32::new(0);
    let parts = make_parts(height, Round::new(round), pol_round, claimed, data, |d| {
        signer
            .sign(&parts_sign_bytes(CHAIN, height, round, keccak256(d)))
            .to_bytes()
    });
    let id = ID.fetch_add(1, Ordering::Relaxed).to_be_bytes().to_vec();
    stream_messages(StreamId::new(id.into()), parts)
}

fn proposal(
    signer: &PrivateKey,
    height: u64,
    round: u32,
    pol_round: Round,
    b: &Block,
) -> Vec<StreamMessage<ProposalPart>> {
    signed_stream(
        signer,
        height,
        round,
        pol_round,
        addr(signer),
        &encode_block(b),
    )
}

// ---------------------------------------------------------------------------
// Finding 1: stream admission
// ---------------------------------------------------------------------------

#[test]
fn admission_rule() {
    let (keys, set) = test_set(4);
    let set = set.inner();
    let p = |h, r| set.select_proposer(h, r).address;
    let other = |h, r| {
        keys.iter()
            .map(addr)
            .find(|a| *a != set.select_proposer(h, r).address)
            .unwrap()
    };
    let ok = |c, cur, h, r: u32, who| admit(c, cur, h, Round::new(r), &who, None, set);
    assert_eq!(ok(4, (5, 0), 5, 0, p(5, 0)), Ok(()));
    assert_eq!(ok(4, (5, 0), 6, 0, p(6, 0)), Ok(()));
    assert_eq!(
        ok(4, (5, 0), 7, 0, p(7, 0)),
        Err(Refusal::HeightOutsideWindow)
    );
    assert_eq!(
        ok(4, (5, 0), 4, 0, p(4, 0)),
        Err(Refusal::HeightOutsideWindow)
    );
    assert_eq!(
        ok(4, (5, 0), u64::MAX, 0, p(u64::MAX, 0)),
        Err(Refusal::HeightOutsideWindow)
    );
    assert_eq!(
        ok(4, (5, 0), 5, 1, other(5, 1)),
        Err(Refusal::NotTheProposer)
    );
    // Rounds: up to MAX_ROUND_AHEAD past the current round (of the next height: 0).
    assert_eq!(ok(4, (5, 3), 5, 3 + MAX_ROUND_AHEAD, p(5, 11)), Ok(()));
    assert_eq!(
        ok(4, (5, 3), 5, 4 + MAX_ROUND_AHEAD, p(5, 12)),
        Err(Refusal::RoundTooFarAhead)
    );
    assert_eq!(
        ok(4, (5, 3), 6, 1 + MAX_ROUND_AHEAD, p(6, 9)),
        Err(Refusal::RoundTooFarAhead)
    );
    assert_eq!(
        admit(4, (5, 0), 5, Round::Nil, &p(5, 0), None, set),
        Err(Refusal::NilRound)
    );
    // A publisher bound to a validator may only stream that validator's proposals.
    let me = p(5, 0);
    assert_eq!(
        admit(4, (5, 0), 5, Round::new(0), &me, Some(&me), set),
        Ok(())
    );
    assert_eq!(
        admit(4, (5, 0), 5, Round::new(0), &me, Some(&other(5, 0)), set),
        Err(Refusal::PublisherIsAnotherValidator)
    );
}

#[tokio::test]
async fn streams_are_admitted_only_from_the_round_proposer() {
    let mut h = harness();
    let publisher = rp();
    // The proposer of (1, 0): accepted and validated.
    let p0 = h.proposer_key(1, 0);
    let b = block(1, 1);
    let v = h.feed(publisher, proposal(&p0, 1, 0, Round::Nil, &b)).await;
    let v = v.expect("proposer's block reaches Malachite");
    assert_eq!((v.proposer, v.value.hash), (addr(&p0), b.hash()));
    assert_eq!(v.validity, MValidity::Valid);

    // Another validator streaming "its own" block for round 1: refused at Init.
    let o1 = h.other_key(1, 1);
    let junk = block(1, 2);
    assert!(h
        .feed(publisher, proposal(&o1, 1, 1, Round::Nil, &junk))
        .await
        .is_none());
    // Two of them per round cannot fill Malachite's 2-entry bucket either.
    assert!(h
        .feed(publisher, proposal(&o1, 1, 1, Round::Nil, &block(1, 3)))
        .await
        .is_none());
    // Claiming the real proposer without its key: bad signature.
    let p1 = addr(&h.proposer_key(1, 1));
    let forged = signed_stream(&o1, 1, 1, Round::Nil, p1, &encode_block(&junk));
    assert!(h.feed(publisher, forged).await.is_none());
    assert!(h.stored(1).iter().all(|(r, _, _)| *r == 0));
    assert_eq!(h.stored(1).len(), 1);

    // Outside the height window / too far ahead: refused before anything is buffered
    // and before any validator-set lookup.
    for (height, round) in [(3u64, 0u32), (u64::MAX, 0), (1, 100)] {
        let k = h.proposer_key(height, round);
        let msgs = proposal(&k, height, round, Round::Nil, &block(height, 4));
        assert!(h.feed(publisher, msgs).await.is_none(), "{height}/{round}");
    }
    assert_eq!(h.host.streams.len(), 0);
    assert_eq!(h.host.streams.charged(&publisher), 0);
    assert!(
        h.host.vsets.keys().all(|k| *k <= 2),
        "{:?}",
        h.host.vsets.keys()
    );

    // The next height: buffered as pending (validated when it starts).
    let n0 = h.proposer_key(2, 0);
    assert!(h
        .feed(publisher, proposal(&n0, 2, 0, Round::Nil, &block(2, 5)))
        .await
        .is_none());
    let pending = h.stored(2);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].2.validity, ProposalMeta::PENDING);
}

#[tokio::test]
async fn stored_values_and_pending_only_from_the_round_proposer() {
    let mut h = harness();
    // Entries written by an older version for a non-proposer: never replayed.
    let o0 = h.other_key(1, 0);
    let b = block(1, 9);
    h.host.store.put_block(1, b.hash(), &b).unwrap();
    for validity in [ProposalMeta::VALID, ProposalMeta::PENDING] {
        h.host
            .store
            .put_proposal(
                1,
                0,
                b.hash(),
                ProposalMeta {
                    valid_round: 0,
                    proposer: addr(&o0),
                    validity,
                },
            )
            .unwrap();
        h.host.validate_pending(1).await.unwrap();
        assert!(h
            .host
            .stored_values(1, Round::new(0))
            .await
            .unwrap()
            .is_empty());
    }
    assert!(h.app.validations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_second_block_from_the_proposer_is_equivocation() {
    let mut h = harness();
    let p0 = h.proposer_key(1, 0);
    let (a, b, c) = (block(1, 1), block(1, 2), block(1, 3));
    let pa = rp();
    assert!(h
        .feed(pa, proposal(&p0, 1, 0, Round::Nil, &a))
        .await
        .is_some());
    // The same block again (another stream / publisher): fine.
    assert!(h
        .feed(rp(), proposal(&p0, 1, 0, Round::Nil, &a))
        .await
        .is_some());
    // A different second block: kept out, reported once.
    assert!(h
        .feed(pa, proposal(&p0, 1, 0, Round::Nil, &b))
        .await
        .is_none());
    assert!(h
        .feed(pa, proposal(&p0, 1, 0, Round::Nil, &c))
        .await
        .is_none());
    let ev = h.app.evidence.lock().unwrap().clone();
    assert_eq!(
        ev,
        vec![Evidence::DoubleProposal {
            validator: addr(&p0),
            height: 1,
            round: 0,
            block_hashes: (a.hash(), b.hash()),
        }]
    );
    let values = h.host.stored_values(1, Round::new(0)).await.unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].value.hash, a.hash());
}

#[tokio::test]
async fn a_publisher_bound_to_a_validator_streams_only_its_own_proposals() {
    let mut h = harness();
    h.host.validator_set(1).await.unwrap();
    let p0 = h.proposer_key(1, 0);
    let o0 = h.other_key(1, 0);
    let (honest, liar) = (rp(), rp());
    {
        let mut m = h.host.peer_validators.write().unwrap();
        m.insert(honest, addr(&p0));
        m.insert(liar, addr(&o0));
    }
    assert_eq!(
        h.host.publisher_class(&honest),
        (PublisherClass::Validator, Some(addr(&p0)))
    );
    assert_eq!(
        h.host.publisher_class(&rp()),
        (PublisherClass::Unbound, None)
    );
    // The liar relays/re-publishes p0's signed proposal as its own publication.
    let b = block(1, 1);
    assert!(h
        .feed(liar, proposal(&p0, 1, 0, Round::Nil, &b))
        .await
        .is_none());
    assert!(h
        .feed(honest, proposal(&p0, 1, 0, Round::Nil, &b))
        .await
        .is_some());
}

// ---------------------------------------------------------------------------
// Finding 5: bodies that do not match their header
// ---------------------------------------------------------------------------

fn mismatched(height: u64) -> Block {
    let mut b = block(height, 7);
    b.header.transactions_root = B256::repeat_byte(0xAB);
    b
}

#[tokio::test]
async fn a_streamed_body_that_does_not_match_its_header_is_never_stored() {
    let mut h = harness();
    let p0 = h.proposer_key(1, 0);
    let bad = mismatched(1);
    assert!(h
        .feed(rp(), proposal(&p0, 1, 0, Round::Nil, &bad))
        .await
        .is_none());
    assert!(h.stored(1).is_empty());
    assert_eq!(h.host.store.get_block(1, bad.hash()).unwrap(), None);
}

#[tokio::test]
async fn synced_values_bad_body_is_a_peer_fault_and_invalid_is_not_stored() {
    let mut h = harness();
    let bad = mismatched(1);
    let out = h
        .host
        .synced_value(1, Round::new(0), addr(&h.keys[1]), &encode_block(&bad))
        .await
        .unwrap();
    assert!(matches!(out, SyncedValueOutcome::PeerFault), "{out:?}");
    assert_eq!(h.host.store.get_block(1, bad.hash()).unwrap(), None);

    let invalid = block(1, 8);
    h.app.invalid.lock().unwrap().insert(invalid.hash());
    let out = h
        .host
        .synced_value(1, Round::new(0), addr(&h.keys[1]), &encode_block(&invalid))
        .await
        .unwrap();
    assert!(
        matches!(&out, SyncedValueOutcome::Verdict(v) if v.validity == MValidity::Invalid),
        "{out:?}"
    );
    assert_eq!(h.host.store.get_block(1, invalid.hash()).unwrap(), None);

    let good = block(1, 9);
    let out = h
        .host
        .synced_value(1, Round::new(0), addr(&h.keys[1]), &encode_block(&good))
        .await
        .unwrap();
    assert!(matches!(&out, SyncedValueOutcome::Verdict(v) if v.validity == MValidity::Valid));
    assert_eq!(
        h.host.store.get_block(1, good.hash()).unwrap(),
        Some(good.clone())
    );
    // The decided block comes from the store (and only a matching one is there).
    assert_eq!(h.host.decided_block(1, good.hash()).await.unwrap(), good);
    assert!(h.host.decided_block(1, bad.hash()).await.is_err());
}

// ---------------------------------------------------------------------------
// Finding 6: verdict reuse and the clock rule
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_verdicts_are_not_reused_and_reproposals_skip_the_clock_rule() {
    let mut h = harness();
    let b = block(1, 1);
    // Round 0: rejected (say the proposer's timestamp was ahead of our clock).
    h.app.invalid.lock().unwrap().insert(b.hash());
    let p0 = h.proposer_key(1, 0);
    let v = h
        .feed(rp(), proposal(&p0, 1, 0, Round::Nil, &b))
        .await
        .unwrap();
    assert_eq!(v.validity, MValidity::Invalid);
    assert_eq!(
        h.app.validations.lock().unwrap().last().unwrap().2,
        BlockOrigin::Consensus
    );
    // Time passes: the block is now acceptable.
    h.app.invalid.lock().unwrap().clear();
    // Round 1 re-proposes the polka'd value (pol_round 0): validated again, without
    // the clock rule.
    let p1 = h.proposer_key(1, 1);
    let v = h
        .feed(rp(), proposal(&p1, 1, 1, Round::new(0), &b))
        .await
        .unwrap();
    assert_eq!(v.validity, MValidity::Valid);
    assert_eq!(v.valid_round, Round::new(0));
    assert_eq!(
        h.app.validations.lock().unwrap().last().unwrap().2,
        BlockOrigin::Sync
    );
    // A valid verdict reached without the clock rule is not reused for a fresh
    // (pol_round nil) proposal of the same block ...
    let n = h.app.validations.lock().unwrap().len();
    let p2 = h.proposer_key(1, 2);
    h.feed(rp(), proposal(&p2, 1, 2, Round::Nil, &b))
        .await
        .unwrap();
    let validations = h.app.validations.lock().unwrap().clone();
    assert_eq!(validations.len(), n + 1);
    assert_eq!(validations.last().unwrap().2, BlockOrigin::Consensus);
    // ... but a clock-checked valid verdict is.
    let p3 = h.proposer_key(1, 3);
    h.feed(rp(), proposal(&p3, 1, 3, Round::Nil, &b))
        .await
        .unwrap();
    assert_eq!(h.app.validations.lock().unwrap().len(), n + 1);
    // Crash recovery replays round 0 with a fresh verdict, not the stored invalid one.
    let replay = h.host.stored_values(1, Round::new(0)).await.unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].validity, MValidity::Valid);
}

// ---------------------------------------------------------------------------
// Finding 7: a failed commit of a decided height
// ---------------------------------------------------------------------------

fn m_cert(height: u64, hash: B256) -> MCommitCertificate<XdevsContext> {
    MCommitCertificate {
        height: Height(height),
        round: Round::new(0),
        value_id: hash,
        commit_signatures: vec![],
    }
}

#[tokio::test]
async fn a_failed_commit_is_retried_in_place_never_restarted() {
    let mut h = harness();
    let b = block(1, 1);
    let cert = m_cert(1, b.hash());
    h.host.pending_commit = Some((b.clone(), to_xdevs_cert(&cert)));
    *h.app.commit_failures.lock().unwrap() = 2;
    match h.host.on_finalized(&cert).await.unwrap() {
        Next::Start(next, _) => assert_eq!(next, Height(2)),
        Next::Restart(..) => panic!("a decided height must never restart"),
    }
    assert_eq!(h.app.commit_attempts.load(Ordering::SeqCst), 3);
    assert_eq!(h.host.committed, (1, b.hash()));

    // A commit that never succeeds halts the engine with an explicit error.
    let b2 = block(2, 1);
    let cert = m_cert(2, b2.hash());
    h.host.pending_commit = Some((b2, to_xdevs_cert(&cert)));
    *h.app.commit_failures.lock().unwrap() = usize::MAX;
    let err = h.host.on_finalized(&cert).await.unwrap_err();
    assert!(format!("{err:#}").contains("halting"), "{err:#}");
    assert_eq!(h.host.committed.0, 1);
}

// ---------------------------------------------------------------------------
// Finding 8: catching up needs unforgeable evidence
// ---------------------------------------------------------------------------

fn signed_vote(k: &PrivateKey, height: u64) -> SignedMessage<XdevsContext, Vote> {
    let v =
        XdevsContext::new(CHAIN).new_prevote(Height(height), Round::new(0), NilOrVal::Nil, addr(k));
    let sig = k.sign(&vote_bytes(CHAIN, &v));
    SignedMessage::new(v, sig)
}

#[test]
fn catching_up_needs_votes_from_more_than_a_third() {
    let (keys, set) = test_set(4);
    let mut vh = VoteHeights::default();
    assert!(vh.observe(CHAIN, &set, &signed_vote(&keys[0], 7)));
    // One validator (< 1/3) cannot switch pacing off, however high it votes.
    assert!(vh.observe(CHAIN, &set, &signed_vote(&keys[0], u64::MAX)));
    assert!(!vh.network_is_past(5, set.inner()));
    // Not a member / a bad signature / not higher: ignored.
    let outsider = super::super::private_key(&[99; 32]);
    assert!(!vh.observe(CHAIN, &set, &signed_vote(&outsider, 9)));
    let mut forged = signed_vote(&keys[1], 9);
    forged.signature = keys[2].sign(b"x");
    assert!(!vh.observe(CHAIN, &set, &forged));
    assert!(!vh.observe(CHAIN, &set, &signed_vote(&keys[0], 3)));
    assert!(!vh.network_is_past(5, set.inner()));
    // A second validator above height 5: more than 1/3 has decided 5.
    assert!(vh.observe(CHAIN, &set, &signed_vote(&keys[1], 6)));
    assert!(vh.network_is_past(5, set.inner()));
    assert!(!vh.network_is_past(6, set.inner()));
}

#[tokio::test]
async fn a_streamed_next_height_block_does_not_switch_pacing_off() {
    let mut h = harness();
    // A validator streams a (signed) block for the next height.
    let n0 = h.proposer_key(2, 0);
    h.feed(rp(), proposal(&n0, 2, 0, Round::Nil, &block(2, 1)))
        .await;
    assert_eq!(h.stored(2).len(), 1);
    let params = h.host.height_params(1).await.unwrap();
    assert!(params.target_time.is_some(), "pacing stays on");
    // Votes above height 1 from two of four validators: catching up.
    h.host.validator_set(1).await.unwrap();
    for k in &h.keys[1..3] {
        h.host.observe_vote(&signed_vote(k, 3));
    }
    let params = h.host.height_params(1).await.unwrap();
    assert_eq!(params.target_time, None);
}

// ---------------------------------------------------------------------------
// Findings 4 and 9: caches and sync serving
// ---------------------------------------------------------------------------

#[tokio::test]
async fn validator_sets_are_cached_only_inside_the_window() {
    let mut h = harness();
    for height in [1, 2, 3, 1_000, u64::MAX] {
        h.host.validator_set(height).await.unwrap();
    }
    assert_eq!(h.host.vsets.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
}

#[tokio::test]
async fn sync_requests_are_served_off_the_consensus_loop() {
    let mut h = harness();
    let b = block(1, 1);
    let cert = to_xdevs_cert(&m_cert(1, b.hash()));
    h.app.decided.lock().unwrap().insert(1, (b, cert));
    *h.app.head.lock().unwrap() = (1, B256::ZERO);
    h.app.block_decided.store(true, Ordering::SeqCst);
    let (tx, rx) = tokio::sync::oneshot::channel();
    // The loop is not held while the app reads blocks.
    tokio::time::timeout(
        Duration::from_secs(1),
        h.host.handle(AppMsg::GetDecidedValues {
            range: Height(1)..=Height(1),
            reply: tx,
        }),
    )
    .await
    .expect("handled without waiting for the app")
    .unwrap();
    h.app.gate.notify_one();
    let values = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(values.len(), 1);
}

#[tokio::test]
async fn sync_responses_are_capped() {
    let (_, set) = test_set(1);
    let app = MockApp::new(set.inner().clone());
    for n in 1..=100u64 {
        let b = block(n, 0);
        let cert = to_xdevs_cert(&m_cert(n, b.hash()));
        app.decided.lock().unwrap().insert(n, (b, cert));
    }
    *app.head.lock().unwrap() = (100, B256::ZERO);
    assert_eq!(
        decided_values(&app, 1, u64::MAX).await.len(),
        MAX_SYNC_VALUES as usize
    );
    assert_eq!(decided_values(&app, 95, 1000).await.len(), 6);
    assert!(decided_values(&app, 0, 5).await.is_empty());
    assert!(decided_values(&app, 6, 5).await.is_empty());
}

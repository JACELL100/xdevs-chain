//! Proposal streaming (research doc §6.4): split a block into `Init`, `Data` chunks
//! (≤ 256 KiB) and a signed `Fin`, and reassemble streams from the network with
//! per-publisher budgets.
//!
//! Streams are keyed by `(publisher, stream_id)`. The publisher is the gossipsub source
//! (authenticated by gossipsub's strict message signing), the same for every part
//! whichever peer relays it. **Every byte and every open stream is charged to the
//! publisher**, never to the relaying neighbour: junk published by one identity and
//! forwarded by an honest proposer only exhausts that identity's own allowance.
//!
//! Publishers bound to a validator (a verified validator proof, see [`PublisherClass`])
//! each get their own allowance; every other publisher also draws from one shared,
//! bounded pool, so any number of Sybil identities cannot crowd out validators. Memory
//! stays bounded by `(validators + 2) * PER_PUBLISHER_BUDGET` bytes and
//! [`MAX_OPEN_STREAMS`] streams. Each stream costs a fixed [`STREAM_OVERHEAD`] (plus its
//! id) on top of its payload, so empty streams are not free.
//!
//! Admission of a stream (height window, expected proposer) is the host's job: it checks
//! the `Init` part before it is buffered and calls [`PartStreams::reject`] on failure.

use super::context::{Height, ProposalPart};
use crate::types::ValidatorAddress;
use arc_malachitebft_app_channel::app::{
    streaming::{StreamContent, StreamId, StreamMessage},
    types::{core::Round, PeerId},
};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// Largest block body accepted (Phase-2 constant, §6.3 rule 4).
pub(crate) const MAX_BLOCK_BYTES: usize = 4 * 1024 * 1024;
/// `Data` chunk size.
pub(crate) const CHUNK_SIZE: usize = 256 * 1024;
/// Bytes of incomplete streams one publisher may make us buffer.
pub(crate) const PER_PUBLISHER_BUDGET: usize = 2 * (MAX_BLOCK_BYTES + CHUNK_SIZE);
/// Bytes all publishers not bound to a validator may make us buffer together.
pub(crate) const UNBOUND_POOL_BUDGET: usize = 2 * PER_PUBLISHER_BUDGET;
/// Incomplete streams kept per publisher.
pub(crate) const MAX_STREAMS_PER_PUBLISHER: usize = 4;
/// Incomplete streams kept for all publishers not bound to a validator together.
pub(crate) const MAX_UNBOUND_STREAMS: usize = 32;
/// Hard cap on incomplete streams overall.
pub(crate) const MAX_OPEN_STREAMS: usize = 1024;
/// Fixed bookkeeping cost charged per open stream (plus the stream id length).
pub(crate) const STREAM_OVERHEAD: usize = 256;
/// Longest stream id accepted (ours are 20 bytes).
pub(crate) const MAX_STREAM_ID_LEN: usize = 32;
/// Messages per stream: Init + chunks + Fin part + stream Fin, with slack. A sequence
/// number at or above this is malformed.
pub(crate) const MAX_MESSAGES_PER_STREAM: usize = MAX_BLOCK_BYTES / CHUNK_SIZE + 8;
/// Incomplete streams older than this are dropped; finished-stream markers expire too.
pub(crate) const STREAM_TTL: Duration = Duration::from_secs(60);
/// Finished-stream markers kept (late duplicates of a finished stream are ignored).
pub(crate) const MAX_FINISHED: usize = 4096;

/// The parts of one proposal stream, in order (without the stream-level `Fin`).
pub(crate) fn make_parts(
    height: u64,
    round: Round,
    pol_round: Round,
    proposer: ValidatorAddress,
    block_bytes: &[u8],
    sign_fin: impl FnOnce(&[u8]) -> [u8; 64],
) -> Vec<ProposalPart> {
    let mut parts = vec![ProposalPart::Init {
        height: Height(height),
        round,
        pol_round,
        proposer,
    }];
    parts.extend(
        block_bytes
            .chunks(CHUNK_SIZE)
            .map(|c| ProposalPart::Data(c.to_vec())),
    );
    parts.push(ProposalPart::Fin {
        signature: sign_fin(block_bytes),
    });
    parts
}

/// Wrap parts into stream messages: data at sequences `0..n`, stream `Fin` at `n`.
pub(crate) fn stream_messages(
    stream_id: StreamId,
    parts: Vec<ProposalPart>,
) -> Vec<StreamMessage<ProposalPart>> {
    let n = parts.len() as u64;
    let mut out: Vec<_> = parts
        .into_iter()
        .enumerate()
        .map(|(i, p)| StreamMessage::new(stream_id.clone(), i as u64, StreamContent::Data(p)))
        .collect();
    out.push(StreamMessage::new(stream_id, n, StreamContent::Fin));
    out
}

/// A fully received, well-formed stream (signature not yet checked).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompleteStream {
    pub height: u64,
    pub round: Round,
    pub pol_round: Round,
    pub proposer: ValidatorAddress,
    /// Concatenated `Data` chunks (the RLP block).
    pub data: Vec<u8>,
    pub signature: [u8; 64],
}

/// Why a part was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropReason {
    PublisherBudget,
    TooManyStreams,
    StreamTooLarge,
    Malformed,
}

/// How a publisher is accounted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublisherClass {
    /// Its libp2p identity proved a key of the current validator set: its own
    /// allowance only.
    Validator,
    /// Anyone else: its own allowance AND the shared unbound pool.
    Unbound,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Usage {
    bytes: usize,
    streams: usize,
}

impl Usage {
    fn release(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_sub(bytes);
        self.streams = self.streams.saturating_sub(1);
    }
}

#[derive(Debug)]
struct StreamBuf {
    msgs: BTreeMap<u64, StreamContent<ProposalPart>>,
    fin_seq: Option<u64>,
    /// Payload bytes (parts as sized by [`ProposalPart::size_hint`]).
    payload: usize,
    /// Everything charged to the publisher: payload plus the per-stream overhead.
    charged: usize,
    class: PublisherClass,
    created: Instant,
}

type Key = (PeerId, StreamId);

/// Reassembly state for all in-flight proposal streams.
#[derive(Debug, Default)]
pub(crate) struct PartStreams {
    streams: HashMap<Key, StreamBuf>,
    per_publisher: HashMap<PeerId, Usage>,
    unbound: Usage,
    /// Streams already completed or rejected (late duplicates are ignored), by finish
    /// time. Bounded by [`MAX_FINISHED`] and expired after [`STREAM_TTL`].
    finished: HashMap<Key, Instant>,
}

fn msg_size(c: &StreamContent<ProposalPart>) -> usize {
    match c {
        StreamContent::Data(p) => p.size_hint(),
        StreamContent::Fin => 16,
    }
}

impl PartStreams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes currently charged to `publisher`.
    #[cfg(test)]
    pub fn charged(&self, publisher: &PeerId) -> usize {
        self.per_publisher.get(publisher).map_or(0, |u| u.bytes)
    }

    /// Incomplete streams held.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Finished-stream markers held.
    #[cfg(test)]
    pub fn finished_len(&self) -> usize {
        self.finished.len()
    }

    fn mark_finished(&mut self, key: Key, now: Instant) {
        if self.finished.len() >= MAX_FINISHED {
            // Keep the newer half: amortised O(1) per insert.
            let mut times: Vec<Instant> = self.finished.values().copied().collect();
            times.sort_unstable();
            let cutoff = times[times.len() / 2];
            self.finished.retain(|_, t| *t > cutoff);
        }
        self.finished.insert(key, now);
    }

    fn release(&mut self, key: &Key, now: Instant) {
        if let Some(buf) = self.streams.remove(key) {
            if let Some(u) = self.per_publisher.get_mut(&key.0) {
                u.release(buf.charged);
                if u.streams == 0 {
                    self.per_publisher.remove(&key.0);
                }
            }
            if buf.class == PublisherClass::Unbound {
                self.unbound.release(buf.charged);
            }
        }
        self.mark_finished(key.clone(), now);
    }

    /// Refuse a whole stream (its `Init` failed admission): free what it holds and
    /// ignore its remaining parts.
    pub fn reject(&mut self, publisher: PeerId, stream_id: &StreamId, now: Instant) {
        self.release(&(publisher, stream_id.clone()), now);
    }

    /// Add one message published by `publisher`. `Ok(Some(_))` when it completes a
    /// well-formed stream.
    pub fn insert(
        &mut self,
        publisher: PeerId,
        class: PublisherClass,
        msg: StreamMessage<ProposalPart>,
        now: Instant,
    ) -> Result<Option<CompleteStream>, DropReason> {
        let id_len = msg.stream_id.to_bytes().len();
        if id_len > MAX_STREAM_ID_LEN {
            return Err(DropReason::Malformed);
        }
        let key = (publisher, msg.stream_id.clone());
        if self.finished.contains_key(&key) {
            return Ok(None);
        }
        if msg.sequence >= MAX_MESSAGES_PER_STREAM as u64 {
            self.release(&key, now);
            return Err(DropReason::StreamTooLarge);
        }
        let size = msg_size(&msg.content);
        let usage = self
            .per_publisher
            .get(&publisher)
            .copied()
            .unwrap_or_default();
        let (charge, class, new) = match self.streams.get(&key) {
            Some(buf) => {
                if buf.msgs.contains_key(&msg.sequence) {
                    return Ok(None); // duplicate (gossip relays)
                }
                if buf.payload + size > MAX_BLOCK_BYTES + CHUNK_SIZE {
                    self.release(&key, now);
                    return Err(DropReason::StreamTooLarge);
                }
                (size, buf.class, false)
            }
            None => {
                if self.streams.len() >= MAX_OPEN_STREAMS
                    || usage.streams >= MAX_STREAMS_PER_PUBLISHER
                    || (class == PublisherClass::Unbound
                        && self.unbound.streams >= MAX_UNBOUND_STREAMS)
                {
                    return Err(DropReason::TooManyStreams);
                }
                (size + STREAM_OVERHEAD + id_len, class, true)
            }
        };
        if usage.bytes + charge > PER_PUBLISHER_BUDGET
            || (class == PublisherClass::Unbound
                && self.unbound.bytes + charge > UNBOUND_POOL_BUDGET)
        {
            return Err(DropReason::PublisherBudget);
        }

        let buf = self
            .streams
            .entry(key.clone())
            .or_insert_with(|| StreamBuf {
                msgs: BTreeMap::new(),
                fin_seq: None,
                payload: 0,
                charged: 0,
                class,
                created: now,
            });
        if let StreamContent::Fin = msg.content {
            buf.fin_seq = Some(msg.sequence);
        }
        buf.payload += size;
        buf.charged += charge;
        buf.msgs.insert(msg.sequence, msg.content);
        let u = self.per_publisher.entry(publisher).or_default();
        u.bytes += charge;
        u.streams += usize::from(new);
        if class == PublisherClass::Unbound {
            self.unbound.bytes += charge;
            self.unbound.streams += usize::from(new);
        }

        let buf = self.streams.get_mut(&key).expect("just inserted");
        let Some(fin) = buf.fin_seq else {
            return Ok(None);
        };
        if buf.msgs.keys().next_back().is_some_and(|&last| last > fin) {
            self.release(&key, now);
            return Err(DropReason::Malformed);
        }
        // `fin < MAX_MESSAGES_PER_STREAM` (checked above): no overflow.
        if buf.msgs.len() as u64 != fin + 1 {
            return Ok(None);
        }
        let msgs = std::mem::take(&mut buf.msgs);
        self.release(&key, now);
        assemble(msgs).map(Some).ok_or(DropReason::Malformed)
    }

    /// Drop streams older than [`STREAM_TTL`] and expired finished-stream markers.
    /// (Streams for heights that leave the admission window fail the host's re-check
    /// when they complete.)
    pub fn prune(&mut self, now: Instant) {
        let stale: Vec<_> = self
            .streams
            .iter()
            .filter(|(_, b)| now.saturating_duration_since(b.created) > STREAM_TTL)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            self.release(&k, now);
        }
        self.finished
            .retain(|_, t| now.saturating_duration_since(*t) <= STREAM_TTL);
    }
}

/// `[Init, Data*, Fin]` then the stream `Fin` → [`CompleteStream`].
fn assemble(msgs: BTreeMap<u64, StreamContent<ProposalPart>>) -> Option<CompleteStream> {
    let mut it = msgs.into_values();
    let Some(StreamContent::Data(ProposalPart::Init {
        height,
        round,
        pol_round,
        proposer,
    })) = it.next()
    else {
        return None;
    };
    let mut data = Vec::new();
    let mut signature = None;
    for c in it {
        match (c, signature) {
            (StreamContent::Data(ProposalPart::Data(chunk)), None) => data.extend(chunk),
            (StreamContent::Data(ProposalPart::Fin { signature: s }), None) => signature = Some(s),
            (StreamContent::Fin, Some(_)) => {}
            _ => return None,
        }
    }
    Some(CompleteStream {
        height: height.0,
        round,
        pol_round,
        proposer,
        data,
        signature: signature?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bytes::Bytes;
    use PublisherClass::{Unbound, Validator};

    /// A fresh peer id (from a counter-derived libp2p key).
    pub(crate) fn rp() -> PeerId {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(1);
        let mut seed = [0xA5; 32];
        seed[..4].copy_from_slice(&N.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        let kp = arc_malachitebft_app_channel::app::types::Keypair::ed25519_from_bytes(&mut seed)
            .unwrap();
        PeerId::from_bytes(&kp.public().to_peer_id().to_bytes()).unwrap()
    }

    fn sid(n: u8) -> StreamId {
        StreamId::new(Bytes::from(vec![n]))
    }

    fn msgs(height: u64, len: usize, n: u8) -> (Vec<u8>, Vec<StreamMessage<ProposalPart>>) {
        let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let parts = make_parts(
            height,
            Round::new(1),
            Round::Nil,
            ValidatorAddress([9; 20]),
            &data,
            |_| [7; 64],
        );
        (data, stream_messages(sid(n), parts))
    }

    #[test]
    fn reassembles_out_of_order_with_duplicates() {
        let (data, mut m) = msgs(5, CHUNK_SIZE * 2 + 10, 1);
        assert_eq!(m.len(), 1 + 3 + 1 + 1);
        m.reverse();
        let publisher = rp();
        let mut s = PartStreams::new();
        let now = Instant::now();
        let mut done = None;
        for (i, msg) in m.iter().enumerate() {
            let r = s.insert(publisher, Unbound, msg.clone(), now).unwrap();
            if i + 1 < m.len() {
                assert!(r.is_none());
                // Duplicates are ignored.
                assert!(s
                    .insert(publisher, Unbound, msg.clone(), now)
                    .unwrap()
                    .is_none());
            } else {
                done = r;
            }
        }
        let c = done.expect("complete");
        assert_eq!(c.data, data);
        assert_eq!(
            (c.height, c.round, c.signature),
            (5, Round::new(1), [7; 64])
        );
        assert_eq!(s.charged(&publisher), 0);
        assert_eq!(s.len(), 0);
        // A late duplicate of a finished stream is ignored, not re-buffered.
        assert!(s
            .insert(publisher, Unbound, m[0].clone(), now)
            .unwrap()
            .is_none());
        assert_eq!(s.len(), 0);
    }

    /// Finding 2: bytes are charged to the publisher, so junk from one publisher can
    /// never use up the allowance of another (e.g. the next proposer relaying it).
    #[test]
    fn budget_is_charged_to_the_publisher_not_the_relay() {
        let now = Instant::now();
        let mut s = PartStreams::new();
        let (attacker, proposer) = (rp(), rp());
        // The attacker publishes far more than its budget (relayed by anyone).
        let mut dropped = 0;
        for n in 0..8u8 {
            let (_, m) = msgs(5, MAX_BLOCK_BYTES, n);
            for msg in &m[..m.len() - 1] {
                match s.insert(attacker, Validator, msg.clone(), now) {
                    Err(DropReason::PublisherBudget | DropReason::TooManyStreams) => dropped += 1,
                    Err(e) => panic!("{e:?}"),
                    Ok(_) => {}
                }
            }
        }
        assert!(dropped > 0);
        assert!(s.charged(&attacker) <= PER_PUBLISHER_BUDGET);
        // The proposer's full-size block still goes through.
        let (data, m) = msgs(5, MAX_BLOCK_BYTES, 77);
        let mut res = None;
        for msg in m {
            res = s.insert(proposer, Validator, msg, now).unwrap();
        }
        assert_eq!(res.expect("proposer's stream completes").data, data);
        assert_eq!(s.charged(&proposer), 0);
    }

    #[test]
    fn stream_limits_per_publisher_pool_and_overhead() {
        let now = Instant::now();
        // Too many concurrent streams from one publisher.
        let publisher = rp();
        let mut s = PartStreams::new();
        for n in 0..MAX_STREAMS_PER_PUBLISHER as u8 {
            let (_, m) = msgs(5, 10, n);
            s.insert(publisher, Validator, m[0].clone(), now).unwrap();
        }
        // Each open stream costs the fixed overhead plus its id.
        assert_eq!(
            s.charged(&publisher),
            MAX_STREAMS_PER_PUBLISHER * (64 + STREAM_OVERHEAD + 1)
        );
        let (_, m) = msgs(5, 10, 200);
        assert_eq!(
            s.insert(publisher, Validator, m[0].clone(), now),
            Err(DropReason::TooManyStreams)
        );
        // Another validator publisher is unaffected.
        s.insert(rp(), Validator, m[0].clone(), now).unwrap();

        // Unbound publishers (Sybils) share one pool of streams ...
        let mut s = PartStreams::new();
        let mut refused = 0;
        for _ in 0..MAX_UNBOUND_STREAMS + 10 {
            if s.insert(rp(), Unbound, m[0].clone(), now).is_err() {
                refused += 1;
            }
        }
        assert_eq!(refused, 10);
        assert_eq!(s.len(), MAX_UNBOUND_STREAMS);
        // ... which never blocks a validator.
        s.insert(rp(), Validator, m[0].clone(), now).unwrap();

        // TTL pruning frees streams and expires finished markers.
        s.prune(now + STREAM_TTL + Duration::from_secs(1));
        assert_eq!(s.len(), 0);
        assert_eq!(s.finished_len(), MAX_UNBOUND_STREAMS + 1);
        s.prune(now + 3 * STREAM_TTL);
        assert_eq!(s.finished_len(), 0);
    }

    #[test]
    fn unbound_pool_caps_bytes_across_publishers() {
        let now = Instant::now();
        let mut s = PartStreams::new();
        let mut dropped = false;
        for n in 0..8u8 {
            let (_, m) = msgs(5, MAX_BLOCK_BYTES, n);
            let p = rp();
            for msg in &m[..m.len() - 1] {
                if let Err(e) = s.insert(p, Unbound, msg.clone(), now) {
                    assert_eq!(e, DropReason::PublisherBudget);
                    dropped = true;
                }
            }
        }
        assert!(dropped);
        assert!(s.unbound.bytes <= UNBOUND_POOL_BUDGET);
        // Validators still have their own allowance.
        let (_, m) = msgs(5, 100, 99);
        let validator = rp();
        let mut res = None;
        for msg in m {
            res = s.insert(validator, Validator, msg, now).unwrap();
        }
        assert!(res.is_some());
    }

    #[test]
    fn rejects_oversized_and_malformed_streams() {
        let now = Instant::now();
        let publisher = rp();
        let mut s = PartStreams::new();
        let (_, m) = msgs(5, MAX_BLOCK_BYTES + 2 * CHUNK_SIZE, 1);
        let errs: Vec<_> = m
            .into_iter()
            .filter_map(|msg| s.insert(publisher, Validator, msg, now).err())
            .collect();
        assert_eq!(errs.first(), Some(&DropReason::StreamTooLarge));
        assert_eq!(s.charged(&publisher), 0);

        // Data before Init.
        let parts = vec![
            ProposalPart::Data(vec![1]),
            ProposalPart::Fin { signature: [0; 64] },
        ];
        let mut res = Ok(None);
        for msg in stream_messages(sid(2), parts) {
            res = s.insert(publisher, Validator, msg, now);
        }
        assert_eq!(res, Err(DropReason::Malformed));
        // Missing Fin part.
        let parts = vec![ProposalPart::Init {
            height: Height(1),
            round: Round::new(0),
            pol_round: Round::Nil,
            proposer: ValidatorAddress::default(),
        }];
        let mut res = Ok(None);
        for msg in stream_messages(sid(3), parts) {
            res = s.insert(publisher, Validator, msg, now);
        }
        assert_eq!(res, Err(DropReason::Malformed));
        assert_eq!(s.charged(&publisher), 0);
    }

    /// Finding 4: huge sequence numbers / stream Fin positions never overflow (debug
    /// builds) and long stream ids are refused.
    #[test]
    fn rejects_huge_sequences_and_long_ids() {
        let now = Instant::now();
        let publisher = rp();
        let mut s = PartStreams::new();
        let fin = StreamMessage::new(sid(1), u64::MAX, StreamContent::Fin);
        assert_eq!(
            s.insert(publisher, Validator, fin, now),
            Err(DropReason::StreamTooLarge)
        );
        let fin = StreamMessage::new(sid(2), MAX_MESSAGES_PER_STREAM as u64, StreamContent::Fin);
        assert_eq!(
            s.insert(publisher, Validator, fin, now),
            Err(DropReason::StreamTooLarge)
        );
        let long = StreamId::new(Bytes::from(vec![0u8; MAX_STREAM_ID_LEN + 1]));
        let msg = StreamMessage::new(long, 0, StreamContent::Fin);
        assert_eq!(
            s.insert(publisher, Validator, msg, now),
            Err(DropReason::Malformed)
        );
        assert_eq!((s.len(), s.charged(&publisher)), (0, 0));
    }

    /// Finding 4: the finished-stream markers are bounded whatever the attacker sends.
    #[test]
    fn finished_markers_are_bounded() {
        let now = Instant::now();
        let mut s = PartStreams::new();
        let p = rp();
        for i in 0..(MAX_FINISHED as u32 * 2) {
            let id = StreamId::new(Bytes::from(i.to_be_bytes().to_vec()));
            s.reject(p, &id, now + Duration::from_micros(u64::from(i)));
        }
        assert!(s.finished_len() <= MAX_FINISHED);
        assert!(s.finished_len() >= MAX_FINISHED / 2);
    }

    #[test]
    fn reject_frees_a_buffered_stream() {
        let now = Instant::now();
        let p = rp();
        let mut s = PartStreams::new();
        let (_, m) = msgs(5, 1000, 4);
        s.insert(p, Unbound, m[1].clone(), now).unwrap();
        assert!(s.charged(&p) > 0);
        s.reject(p, &m[0].stream_id, now);
        assert_eq!(
            (s.len(), s.charged(&p), s.unbound),
            (0, 0, Usage::default())
        );
        // Its remaining parts are ignored.
        assert_eq!(s.insert(p, Unbound, m[2].clone(), now), Ok(None));
        assert_eq!(s.len(), 0);
    }
}

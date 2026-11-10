//! Proposal streaming (research doc §6.4): split a block into `Init`, `Data` chunks
//! (≤ 256 KiB) and a signed `Fin`, and reassemble streams from the network with
//! per-peer byte budgets.
//!
//! Streams are keyed by `(publisher, stream_id)` (the publisher is the same for every
//! part whichever peer relays it); buffered bytes are charged to the peer that delivered
//! them (`from`), so one peer can never hold more than [`PER_PEER_BUDGET`] of our memory.

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
/// Bytes of incomplete streams one delivering peer may make us buffer.
pub(crate) const PER_PEER_BUDGET: usize = 2 * (MAX_BLOCK_BYTES + CHUNK_SIZE);
/// Incomplete streams kept per publisher.
pub(crate) const MAX_STREAMS_PER_PUBLISHER: usize = 4;
/// Messages per stream: Init + chunks + Fin part + stream Fin, with slack.
pub(crate) const MAX_MESSAGES_PER_STREAM: usize = MAX_BLOCK_BYTES / CHUNK_SIZE + 8;
/// Incomplete streams older than this are dropped.
pub(crate) const STREAM_TTL: Duration = Duration::from_secs(60);

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
    PeerBudget,
    TooManyStreams,
    StreamTooLarge,
    Malformed,
}

#[derive(Debug)]
struct StreamBuf {
    msgs: BTreeMap<u64, StreamContent<ProposalPart>>,
    fin_seq: Option<u64>,
    bytes: usize,
    charges: HashMap<PeerId, usize>,
    height: Option<u64>,
    created: Instant,
}

/// Reassembly state for all in-flight proposal streams.
#[derive(Debug, Default)]
pub(crate) struct PartStreams {
    streams: HashMap<(PeerId, StreamId), StreamBuf>,
    per_peer: HashMap<PeerId, usize>,
    /// Streams already completed or rejected (late duplicates are ignored); by height.
    finished: HashMap<(PeerId, StreamId), u64>,
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

    /// Bytes currently charged to `peer`.
    pub fn charged(&self, peer: &PeerId) -> usize {
        self.per_peer.get(peer).copied().unwrap_or(0)
    }

    /// Incomplete streams held.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    fn release(&mut self, key: &(PeerId, StreamId), finished_height: Option<u64>) {
        if let Some(buf) = self.streams.remove(key) {
            for (peer, n) in buf.charges {
                if let Some(c) = self.per_peer.get_mut(&peer) {
                    *c = c.saturating_sub(n);
                    if *c == 0 {
                        self.per_peer.remove(&peer);
                    }
                }
            }
            self.finished
                .insert(key.clone(), finished_height.or(buf.height).unwrap_or(0));
        }
    }

    /// Add one message. `Ok(Some(_))` when it completes a well-formed stream.
    pub fn insert(
        &mut self,
        from: PeerId,
        publisher: PeerId,
        msg: StreamMessage<ProposalPart>,
        now: Instant,
    ) -> Result<Option<CompleteStream>, DropReason> {
        let key = (publisher, msg.stream_id.clone());
        if self.finished.contains_key(&key) {
            return Ok(None);
        }
        let size = msg_size(&msg.content);
        if !self.streams.contains_key(&key) {
            let open = self.streams.keys().filter(|(p, _)| *p == publisher).count();
            if open >= MAX_STREAMS_PER_PUBLISHER {
                return Err(DropReason::TooManyStreams);
            }
        }
        if self.charged(&from) + size > PER_PEER_BUDGET {
            return Err(DropReason::PeerBudget);
        }
        let buf = self
            .streams
            .entry(key.clone())
            .or_insert_with(|| StreamBuf {
                msgs: BTreeMap::new(),
                fin_seq: None,
                bytes: 0,
                charges: HashMap::new(),
                height: None,
                created: now,
            });
        if buf.msgs.contains_key(&msg.sequence) {
            return Ok(None); // duplicate (gossip relays)
        }
        if buf.bytes + size > MAX_BLOCK_BYTES + CHUNK_SIZE
            || buf.msgs.len() >= MAX_MESSAGES_PER_STREAM
        {
            self.release(&key, None);
            return Err(DropReason::StreamTooLarge);
        }
        match &msg.content {
            StreamContent::Fin => buf.fin_seq = Some(msg.sequence),
            StreamContent::Data(ProposalPart::Init { height, .. }) => buf.height = Some(height.0),
            StreamContent::Data(_) => {}
        }
        buf.bytes += size;
        *buf.charges.entry(from).or_default() += size;
        *self.per_peer.entry(from).or_default() += size;
        buf.msgs.insert(msg.sequence, msg.content);

        let Some(fin) = buf.fin_seq else {
            return Ok(None);
        };
        if buf.msgs.keys().next_back().is_some_and(|&last| last > fin) {
            self.release(&key, None);
            return Err(DropReason::Malformed);
        }
        if buf.msgs.len() as u64 != fin + 1 {
            return Ok(None);
        }
        let msgs = std::mem::take(&mut buf.msgs);
        self.release(&key, None);
        assemble(msgs).map(Some).ok_or(DropReason::Malformed)
    }

    /// Drop streams below `min_height` or older than [`STREAM_TTL`].
    pub fn prune(&mut self, min_height: u64, now: Instant) {
        let stale: Vec<_> = self
            .streams
            .iter()
            .filter(|(_, b)| {
                b.height.is_some_and(|h| h < min_height)
                    || now.saturating_duration_since(b.created) > STREAM_TTL
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            self.release(&k, None);
        }
        self.finished.retain(|_, h| *h >= min_height);
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

    /// A fresh peer id (from a counter-derived libp2p key).
    pub(crate) fn rp() -> PeerId {
        use std::sync::atomic::{AtomicU8, Ordering};
        static N: AtomicU8 = AtomicU8::new(1);
        let mut seed = [N.fetch_add(1, Ordering::Relaxed); 32];
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
        let (from, publisher) = (rp(), rp());
        let mut s = PartStreams::new();
        let now = Instant::now();
        let mut done = None;
        for (i, msg) in m.iter().enumerate() {
            let r = s.insert(from, publisher, msg.clone(), now).unwrap();
            if i + 1 < m.len() {
                assert!(r.is_none());
                // Duplicates are ignored.
                assert!(s
                    .insert(from, publisher, msg.clone(), now)
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
        assert_eq!(s.charged(&from), 0);
        assert_eq!(s.len(), 0);
        // A late duplicate of a finished stream is ignored, not re-buffered.
        assert!(s
            .insert(from, publisher, m[0].clone(), now)
            .unwrap()
            .is_none());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn enforces_per_peer_budget_and_stream_limits() {
        let from = rp();
        let now = Instant::now();
        let mut s = PartStreams::new();
        // Many publishers relayed by one peer: the budget caps what it can make us hold.
        let mut dropped = false;
        for n in 0..40u8 {
            let (_, m) = msgs(5, MAX_BLOCK_BYTES, n);
            let publisher = rp();
            // Withhold the last message so nothing completes.
            for msg in &m[..m.len() - 1] {
                if let Err(e) = s.insert(from, publisher, msg.clone(), now) {
                    assert_eq!(e, DropReason::PeerBudget);
                    dropped = true;
                }
            }
        }
        assert!(dropped);
        assert!(s.charged(&from) <= PER_PEER_BUDGET);
        // Another peer is unaffected.
        let other = rp();
        let (_, m) = msgs(5, 100, 99);
        let p = rp();
        let mut res = None;
        for msg in m {
            res = s.insert(other, p, msg, now).unwrap();
        }
        assert!(res.is_some());

        // Too many concurrent streams from one publisher.
        let publisher = rp();
        let mut s = PartStreams::new();
        for n in 0..MAX_STREAMS_PER_PUBLISHER as u8 {
            let (_, m) = msgs(5, 10, n);
            s.insert(rp(), publisher, m[0].clone(), now).unwrap();
        }
        let (_, m) = msgs(5, 10, 200);
        assert_eq!(
            s.insert(rp(), publisher, m[0].clone(), now),
            Err(DropReason::TooManyStreams)
        );
        // Pruning by height frees them.
        s.prune(6, now);
        assert_eq!(s.len(), 0);
        s.insert(rp(), publisher, m[0].clone(), now).unwrap();
        s.prune(0, now + STREAM_TTL + Duration::from_secs(1));
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn rejects_oversized_and_malformed_streams() {
        let now = Instant::now();
        let (from, publisher) = (rp(), rp());
        let mut s = PartStreams::new();
        let (_, m) = msgs(5, MAX_BLOCK_BYTES + 2 * CHUNK_SIZE, 1);
        let errs: Vec<_> = m
            .into_iter()
            .filter_map(|msg| s.insert(from, publisher, msg, now).err())
            .collect();
        assert_eq!(errs.first(), Some(&DropReason::StreamTooLarge));
        assert_eq!(s.charged(&from), 0);

        // Data before Init.
        let parts = vec![
            ProposalPart::Data(vec![1]),
            ProposalPart::Fin { signature: [0; 64] },
        ];
        let mut res = Ok(None);
        for msg in stream_messages(sid(2), parts) {
            res = s.insert(from, publisher, msg, now);
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
            res = s.insert(from, publisher, msg, now);
        }
        assert_eq!(res, Err(DropReason::Malformed));
        assert_eq!(s.charged(&from), 0);
    }
}

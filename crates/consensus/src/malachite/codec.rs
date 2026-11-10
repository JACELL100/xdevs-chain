//! `XdevsCodec`: one codec for Malachite's network, WAL and sync (research doc §6.4).
//!
//! Every message is `VERSION:u8 ‖ rlp(wire struct)` (alloy-rlp; no protobuf, no `protoc`).
//! The leading tag byte versions the WAL and wire format. Rounds are encoded as
//! `round + 1` so `Round::Nil` (= -1) is `0`; nil-or-value hashes as an empty or 32-byte
//! string. Vote extensions are unsupported in Phase 2 and are rejected on encode.
//!
//! Blocks (proposal `Data` chunks and sync `value_bytes`) use [`encode_block`]: the plain
//! RLP list `[header, [tx_2718, ...]]`.

use super::context::{BlockValue, Height, Proposal, ProposalPart, Vote, XdevsContext};
use super::streaming::MAX_STREAM_ID_LEN;
use super::Signature;
use crate::types::ValidatorAddress;
use alloy_rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable};
use arc_malachitebft_app_channel::app::{
    consensus::LivenessMsg,
    streaming::{StreamContent, StreamId, StreamMessage},
    types::{
        codec::{Codec, HasEncodedLen},
        core::{
            CommitCertificate, ExtendedCommitCertificate, ExtendedCommitSignature, NilOrVal,
            PolkaCertificate, PolkaSignature, Round, RoundCertificate, RoundCertificateType,
            RoundSignature, SignedMessage, ValidatorProof, Validity, VoteType as MVoteType,
        },
        sync::{RawDecidedValue, Request, Response, Status},
        PeerId, ProposedValue, SignedConsensusMsg,
    },
};
use arc_malachitebft_sync::{ValueRequest, ValueResponse};
use bytes::Bytes as RawBytes;
use xdevs_primitives::{Block, Bytes, Decodable2718, Encodable2718, Header, TxEnvelope, B256};

/// Current wire / WAL format version (leading byte of every encoded message).
pub(crate) const CODEC_VERSION: u8 = 1;

/// Encoding or decoding failure.
#[derive(Debug, thiserror::Error)]
#[error("xdevs codec: {0}")]
pub(crate) struct CodecError(pub String);

impl CodecError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl From<alloy_rlp::Error> for CodecError {
    fn from(e: alloy_rlp::Error) -> Self {
        Self(e.to_string())
    }
}

/// The codec handed to Malachite for network, WAL and sync.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XdevsCodec;

// ---------------------------------------------------------------------------
// Primitive helpers
// ---------------------------------------------------------------------------

fn enc<T: Encodable>(w: &T) -> RawBytes {
    let mut out = Vec::with_capacity(1 + w.length());
    out.push(CODEC_VERSION);
    w.encode(&mut out);
    RawBytes::from(out)
}

fn dec<T: Decodable>(bytes: &[u8]) -> Result<T, CodecError> {
    match bytes.split_first() {
        Some((&CODEC_VERSION, rest)) => Ok(alloy_rlp::decode_exact(rest)?),
        Some((v, _)) => Err(CodecError(format!("unsupported codec version {v}"))),
        None => Err(CodecError::new("empty message")),
    }
}

fn round_enc(r: Round) -> u64 {
    // as_i64 >= -1, so this never underflows.
    (r.as_i64() + 1) as u64
}

/// A round that may be `Nil` (POL / valid rounds).
fn round_dec(v: u64) -> Result<Round, CodecError> {
    match v {
        0 => Ok(Round::Nil),
        v if v <= u64::from(u32::MAX) + 1 => Ok(Round::new((v - 1) as u32)),
        _ => Err(CodecError::new("round out of range")),
    }
}

/// The round of a vote, proposal, part stream or certificate: never `Nil` on the wire.
fn round_dec_some(v: u64) -> Result<Round, CodecError> {
    match round_dec(v)? {
        Round::Nil => Err(CodecError::new("nil round")),
        r => Ok(r),
    }
}

fn nil_enc(v: &NilOrVal<B256>) -> Bytes {
    match v {
        NilOrVal::Nil => Bytes::new(),
        NilOrVal::Val(h) => Bytes::copy_from_slice(h.as_slice()),
    }
}

fn nil_dec(b: &[u8]) -> Result<NilOrVal<B256>, CodecError> {
    match b.len() {
        0 => Ok(NilOrVal::Nil),
        32 => Ok(NilOrVal::Val(B256::from_slice(b))),
        n => Err(CodecError(format!("bad value id length {n}"))),
    }
}

fn vt_enc(t: MVoteType) -> u8 {
    match t {
        MVoteType::Prevote => 1,
        MVoteType::Precommit => 2,
    }
}

fn vt_dec(t: u8) -> Result<MVoteType, CodecError> {
    match t {
        1 => Ok(MVoteType::Prevote),
        2 => Ok(MVoteType::Precommit),
        _ => Err(CodecError(format!("bad vote type {t}"))),
    }
}

fn sig_enc(s: &Signature) -> [u8; 64] {
    s.to_bytes()
}

fn sig_dec(s: [u8; 64]) -> Signature {
    Signature::from_bytes(s)
}

// ---------------------------------------------------------------------------
// Wire structs
// ---------------------------------------------------------------------------

#[derive(RlpEncodable, RlpDecodable)]
struct WTagged {
    tag: u8,
    body: Bytes,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WVote {
    vote_type: u8,
    height: u64,
    round: u64,
    value: Bytes,
    validator: ValidatorAddress,
}

impl WVote {
    fn from_vote(v: &Vote) -> Self {
        Self {
            vote_type: vt_enc(v.vote_type),
            height: v.height.0,
            round: round_enc(v.round),
            value: nil_enc(&v.value),
            validator: v.validator_address,
        }
    }

    fn into_vote(self) -> Result<Vote, CodecError> {
        Ok(Vote {
            vote_type: vt_dec(self.vote_type)?,
            height: Height(self.height),
            round: round_dec_some(self.round)?,
            value: nil_dec(&self.value)?,
            validator_address: self.validator,
        })
    }
}

#[derive(RlpEncodable, RlpDecodable)]
struct WSignedVote {
    vote: WVote,
    signature: [u8; 64],
}

type MSignedVote = SignedMessage<XdevsContext, Vote>;
type MSignedProposal = SignedMessage<XdevsContext, Proposal>;

impl WSignedVote {
    fn from_signed(v: &MSignedVote) -> Self {
        Self {
            vote: WVote::from_vote(&v.message),
            signature: sig_enc(&v.signature),
        }
    }

    fn into_signed(self) -> Result<MSignedVote, CodecError> {
        Ok(SignedMessage::new(
            self.vote.into_vote()?,
            sig_dec(self.signature),
        ))
    }
}

#[derive(RlpEncodable, RlpDecodable)]
struct WProposal {
    height: u64,
    round: u64,
    value: B256,
    pol_round: u64,
    validator: ValidatorAddress,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WSignedProposal {
    proposal: WProposal,
    signature: [u8; 64],
}

impl WSignedProposal {
    fn from_signed(p: &MSignedProposal) -> Self {
        let m = &p.message;
        Self {
            proposal: WProposal {
                height: m.height.0,
                round: round_enc(m.round),
                value: m.value.hash,
                pol_round: round_enc(m.pol_round),
                validator: m.validator_address,
            },
            signature: sig_enc(&p.signature),
        }
    }

    fn into_signed(self) -> Result<MSignedProposal, CodecError> {
        let w = self.proposal;
        Ok(SignedMessage::new(
            Proposal {
                height: Height(w.height),
                round: round_dec_some(w.round)?,
                value: BlockValue::new(w.value),
                pol_round: round_dec(w.pol_round)?,
                validator_address: w.validator,
            },
            sig_dec(self.signature),
        ))
    }
}

#[derive(RlpEncodable, RlpDecodable)]
struct WInit {
    height: u64,
    round: u64,
    pol_round: u64,
    proposer: ValidatorAddress,
}

const PART_INIT: u8 = 0;
const PART_DATA: u8 = 1;
const PART_FIN: u8 = 2;

fn part_to_wire(p: &ProposalPart) -> WTagged {
    match p {
        ProposalPart::Init {
            height,
            round,
            pol_round,
            proposer,
        } => WTagged {
            tag: PART_INIT,
            body: alloy_rlp::encode(WInit {
                height: height.0,
                round: round_enc(*round),
                pol_round: round_enc(*pol_round),
                proposer: *proposer,
            })
            .into(),
        },
        ProposalPart::Data(chunk) => WTagged {
            tag: PART_DATA,
            body: Bytes::copy_from_slice(chunk),
        },
        ProposalPart::Fin { signature } => WTagged {
            tag: PART_FIN,
            body: Bytes::copy_from_slice(signature),
        },
    }
}

fn part_from_wire(w: WTagged) -> Result<ProposalPart, CodecError> {
    match w.tag {
        PART_INIT => {
            let i: WInit = alloy_rlp::decode_exact(&w.body)?;
            Ok(ProposalPart::Init {
                height: Height(i.height),
                round: round_dec_some(i.round)?,
                pol_round: round_dec(i.pol_round)?,
                proposer: i.proposer,
            })
        }
        PART_DATA => Ok(ProposalPart::Data(w.body.to_vec())),
        PART_FIN => {
            let signature: [u8; 64] = w.body[..]
                .try_into()
                .map_err(|_| CodecError::new("bad Fin signature length"))?;
            Ok(ProposalPart::Fin { signature })
        }
        t => Err(CodecError(format!("bad proposal part tag {t}"))),
    }
}

#[derive(RlpEncodable, RlpDecodable)]
struct WStream {
    stream_id: Bytes,
    sequence: u64,
    /// tag 0 = data (body = rlp part), tag 1 = stream fin (empty body).
    content: WTagged,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WSig {
    address: ValidatorAddress,
    signature: [u8; 64],
}

#[derive(RlpEncodable, RlpDecodable)]
struct WPolka {
    height: u64,
    round: u64,
    value_id: B256,
    signatures: Vec<WSig>,
}

impl WPolka {
    fn from_cert(c: &PolkaCertificate<XdevsContext>) -> Self {
        Self {
            height: c.height.0,
            round: round_enc(c.round),
            value_id: c.value_id,
            signatures: c
                .polka_signatures
                .iter()
                .map(|s| WSig {
                    address: s.address,
                    signature: sig_enc(&s.signature),
                })
                .collect(),
        }
    }

    fn into_cert(self) -> Result<PolkaCertificate<XdevsContext>, CodecError> {
        Ok(PolkaCertificate {
            height: Height(self.height),
            round: round_dec_some(self.round)?,
            value_id: self.value_id,
            polka_signatures: self
                .signatures
                .into_iter()
                .map(|s| PolkaSignature::new(s.address, sig_dec(s.signature)))
                .collect(),
        })
    }
}

#[derive(RlpEncodable, RlpDecodable)]
struct WRoundSig {
    vote_type: u8,
    value: Bytes,
    address: ValidatorAddress,
    signature: [u8; 64],
}

#[derive(RlpEncodable, RlpDecodable)]
struct WRoundCert {
    height: u64,
    round: u64,
    cert_type: u8,
    signatures: Vec<WRoundSig>,
}

impl WRoundCert {
    fn from_cert(c: &RoundCertificate<XdevsContext>) -> Self {
        Self {
            height: c.height.0,
            round: round_enc(c.round),
            cert_type: match c.cert_type {
                RoundCertificateType::Skip => 0,
                RoundCertificateType::Precommit => 1,
            },
            signatures: c
                .round_signatures
                .iter()
                .map(|s| WRoundSig {
                    vote_type: vt_enc(s.vote_type),
                    value: nil_enc(&s.value_id),
                    address: s.address,
                    signature: sig_enc(&s.signature),
                })
                .collect(),
        }
    }

    fn into_cert(self) -> Result<RoundCertificate<XdevsContext>, CodecError> {
        Ok(RoundCertificate {
            height: Height(self.height),
            round: round_dec_some(self.round)?,
            cert_type: match self.cert_type {
                0 => RoundCertificateType::Skip,
                1 => RoundCertificateType::Precommit,
                t => return Err(CodecError(format!("bad round certificate type {t}"))),
            },
            round_signatures: self
                .signatures
                .into_iter()
                .map(|s| {
                    Ok(RoundSignature::new(
                        vt_dec(s.vote_type)?,
                        nil_dec(&s.value)?,
                        s.address,
                        sig_dec(s.signature),
                    ))
                })
                .collect::<Result<_, CodecError>>()?,
        })
    }
}

#[derive(RlpEncodable, RlpDecodable)]
struct WProof {
    public_key: Bytes,
    peer_id: Bytes,
    signature: [u8; 64],
}

#[derive(RlpEncodable, RlpDecodable)]
struct WProposedValue {
    height: u64,
    round: u64,
    valid_round: u64,
    proposer: ValidatorAddress,
    value: B256,
    valid: bool,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WStatus {
    peer_id: Bytes,
    tip_height: u64,
    history_min_height: u64,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WRequest {
    start: u64,
    end: u64,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WCommitCert {
    height: u64,
    round: u64,
    value_id: B256,
    signatures: Vec<WSig>,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WRawDecided {
    value_bytes: Bytes,
    certificate: WCommitCert,
}

#[derive(RlpEncodable, RlpDecodable)]
struct WResponse {
    start_height: u64,
    values: Vec<WRawDecided>,
}

fn peer_dec(b: &[u8]) -> Result<PeerId, CodecError> {
    PeerId::from_bytes(b).map_err(|e| CodecError(format!("bad peer id: {e}")))
}

// ---------------------------------------------------------------------------
// Codec impls
// ---------------------------------------------------------------------------

impl Codec<ProposalPart> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<ProposalPart, CodecError> {
        part_from_wire(dec(&bytes)?)
    }

    fn encode(&self, msg: &ProposalPart) -> Result<RawBytes, CodecError> {
        Ok(enc(&part_to_wire(msg)))
    }
}

const MSG_VOTE: u8 = 0;
const MSG_PROPOSAL: u8 = 1;
const MSG_POLKA: u8 = 2;
const MSG_ROUND_CERT: u8 = 3;

impl Codec<SignedConsensusMsg<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<SignedConsensusMsg<XdevsContext>, CodecError> {
        let w: WTagged = dec(&bytes)?;
        match w.tag {
            MSG_VOTE => Ok(SignedConsensusMsg::Vote(
                alloy_rlp::decode_exact::<WSignedVote>(&w.body)?.into_signed()?,
            )),
            MSG_PROPOSAL => Ok(SignedConsensusMsg::Proposal(
                alloy_rlp::decode_exact::<WSignedProposal>(&w.body)?.into_signed()?,
            )),
            t => Err(CodecError(format!("bad consensus message tag {t}"))),
        }
    }

    fn encode(&self, msg: &SignedConsensusMsg<XdevsContext>) -> Result<RawBytes, CodecError> {
        let w = match msg {
            SignedConsensusMsg::Vote(v) => WTagged {
                tag: MSG_VOTE,
                body: alloy_rlp::encode(WSignedVote::from_signed(v)).into(),
            },
            SignedConsensusMsg::Proposal(p) => WTagged {
                tag: MSG_PROPOSAL,
                body: alloy_rlp::encode(WSignedProposal::from_signed(p)).into(),
            },
        };
        Ok(enc(&w))
    }
}

impl Codec<LivenessMsg<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<LivenessMsg<XdevsContext>, CodecError> {
        let w: WTagged = dec(&bytes)?;
        match w.tag {
            MSG_VOTE => Ok(LivenessMsg::Vote(
                alloy_rlp::decode_exact::<WSignedVote>(&w.body)?.into_signed()?,
            )),
            MSG_POLKA => Ok(LivenessMsg::PolkaCertificate(
                alloy_rlp::decode_exact::<WPolka>(&w.body)?.into_cert()?,
            )),
            MSG_ROUND_CERT => Ok(LivenessMsg::SkipRoundCertificate(
                alloy_rlp::decode_exact::<WRoundCert>(&w.body)?.into_cert()?,
            )),
            t => Err(CodecError(format!("bad liveness message tag {t}"))),
        }
    }

    fn encode(&self, msg: &LivenessMsg<XdevsContext>) -> Result<RawBytes, CodecError> {
        let w = match msg {
            LivenessMsg::Vote(v) => WTagged {
                tag: MSG_VOTE,
                body: alloy_rlp::encode(WSignedVote::from_signed(v)).into(),
            },
            LivenessMsg::PolkaCertificate(c) => WTagged {
                tag: MSG_POLKA,
                body: alloy_rlp::encode(WPolka::from_cert(c)).into(),
            },
            LivenessMsg::SkipRoundCertificate(c) => WTagged {
                tag: MSG_ROUND_CERT,
                body: alloy_rlp::encode(WRoundCert::from_cert(c)).into(),
            },
        };
        Ok(enc(&w))
    }
}

impl Codec<StreamMessage<ProposalPart>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<StreamMessage<ProposalPart>, CodecError> {
        let w: WStream = dec(&bytes)?;
        if w.stream_id.len() > MAX_STREAM_ID_LEN {
            return Err(CodecError::new("stream id too long"));
        }
        let content = match w.content.tag {
            0 => StreamContent::Data(part_from_wire(alloy_rlp::decode_exact(&w.content.body)?)?),
            1 if w.content.body.is_empty() => StreamContent::Fin,
            t => return Err(CodecError(format!("bad stream content tag {t}"))),
        };
        Ok(StreamMessage::new(
            StreamId::new(w.stream_id.0),
            w.sequence,
            content,
        ))
    }

    fn encode(&self, msg: &StreamMessage<ProposalPart>) -> Result<RawBytes, CodecError> {
        let content = match &msg.content {
            StreamContent::Data(p) => WTagged {
                tag: 0,
                body: alloy_rlp::encode(part_to_wire(p)).into(),
            },
            StreamContent::Fin => WTagged {
                tag: 1,
                body: Bytes::new(),
            },
        };
        Ok(enc(&WStream {
            stream_id: Bytes(msg.stream_id.to_bytes()),
            sequence: msg.sequence,
            content,
        }))
    }
}

impl Codec<ValidatorProof<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<ValidatorProof<XdevsContext>, CodecError> {
        let w: WProof = dec(&bytes)?;
        Ok(ValidatorProof::new(
            w.public_key.to_vec(),
            w.peer_id.to_vec(),
            sig_dec(w.signature),
        ))
    }

    fn encode(&self, msg: &ValidatorProof<XdevsContext>) -> Result<RawBytes, CodecError> {
        Ok(enc(&WProof {
            public_key: Bytes::copy_from_slice(&msg.public_key),
            peer_id: Bytes::copy_from_slice(&msg.peer_id),
            signature: sig_enc(&msg.signature),
        }))
    }
}

impl Codec<ProposedValue<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<ProposedValue<XdevsContext>, CodecError> {
        let w: WProposedValue = dec(&bytes)?;
        Ok(ProposedValue {
            height: Height(w.height),
            round: round_dec_some(w.round)?,
            valid_round: round_dec(w.valid_round)?,
            proposer: w.proposer,
            value: BlockValue::new(w.value),
            validity: Validity::from_bool(w.valid),
        })
    }

    fn encode(&self, msg: &ProposedValue<XdevsContext>) -> Result<RawBytes, CodecError> {
        Ok(enc(&WProposedValue {
            height: msg.height.0,
            round: round_enc(msg.round),
            valid_round: round_enc(msg.valid_round),
            proposer: msg.proposer,
            value: msg.value.hash,
            valid: msg.validity.is_valid(),
        }))
    }
}

impl Codec<PolkaCertificate<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<PolkaCertificate<XdevsContext>, CodecError> {
        dec::<WPolka>(&bytes)?.into_cert()
    }

    fn encode(&self, msg: &PolkaCertificate<XdevsContext>) -> Result<RawBytes, CodecError> {
        Ok(enc(&WPolka::from_cert(msg)))
    }
}

impl Codec<Status<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<Status<XdevsContext>, CodecError> {
        let w: WStatus = dec(&bytes)?;
        Ok(Status {
            peer_id: peer_dec(&w.peer_id)?,
            tip_height: Height(w.tip_height),
            history_min_height: Height(w.history_min_height),
        })
    }

    fn encode(&self, msg: &Status<XdevsContext>) -> Result<RawBytes, CodecError> {
        Ok(enc(&WStatus {
            peer_id: msg.peer_id.to_bytes().into(),
            tip_height: msg.tip_height.0,
            history_min_height: msg.history_min_height.0,
        }))
    }
}

impl Codec<Request<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<Request<XdevsContext>, CodecError> {
        let w: WRequest = dec(&bytes)?;
        Ok(Request::ValueRequest(ValueRequest::new(
            Height(w.start)..=Height(w.end),
        )))
    }

    fn encode(&self, msg: &Request<XdevsContext>) -> Result<RawBytes, CodecError> {
        let Request::ValueRequest(r) = msg;
        Ok(enc(&WRequest {
            start: r.range.start().0,
            end: r.range.end().0,
        }))
    }
}

fn cert_to_wire(c: &ExtendedCommitCertificate<XdevsContext>) -> Result<WCommitCert, CodecError> {
    let signatures = c
        .commit_signatures
        .iter()
        .map(|s| {
            if s.extension.is_some() {
                return Err(CodecError::new("vote extensions are not supported"));
            }
            Ok(WSig {
                address: s.address,
                signature: sig_enc(&s.signature),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(WCommitCert {
        height: c.height.0,
        round: round_enc(c.round),
        value_id: c.value_id,
        signatures,
    })
}

fn cert_from_wire(w: WCommitCert) -> Result<ExtendedCommitCertificate<XdevsContext>, CodecError> {
    Ok(ExtendedCommitCertificate {
        height: Height(w.height),
        round: round_dec_some(w.round)?,
        value_id: w.value_id,
        commit_signatures: w
            .signatures
            .into_iter()
            .map(|s| ExtendedCommitSignature::new(s.address, sig_dec(s.signature), None))
            .collect(),
    })
}

fn response_to_wire(msg: &Response<XdevsContext>) -> Result<WResponse, CodecError> {
    let Response::ValueResponse(r) = msg;
    Ok(WResponse {
        start_height: r.start_height.0,
        values: r
            .values
            .iter()
            .map(|v| {
                Ok(WRawDecided {
                    value_bytes: Bytes(v.value_bytes.clone()),
                    certificate: cert_to_wire(&v.certificate)?,
                })
            })
            .collect::<Result<_, CodecError>>()?,
    })
}

impl Codec<Response<XdevsContext>> for XdevsCodec {
    type Error = CodecError;

    fn decode(&self, bytes: RawBytes) -> Result<Response<XdevsContext>, CodecError> {
        let w: WResponse = dec(&bytes)?;
        let values = w
            .values
            .into_iter()
            .map(|v| {
                Ok(RawDecidedValue::new(
                    v.value_bytes.0,
                    cert_from_wire(v.certificate)?,
                ))
            })
            .collect::<Result<_, CodecError>>()?;
        Ok(Response::ValueResponse(ValueResponse::new(
            Height(w.start_height),
            values,
        )))
    }

    fn encode(&self, msg: &Response<XdevsContext>) -> Result<RawBytes, CodecError> {
        Ok(enc(&response_to_wire(msg)?))
    }
}

impl HasEncodedLen<Response<XdevsContext>> for XdevsCodec {
    fn encoded_len(&self, msg: &Response<XdevsContext>) -> Result<usize, CodecError> {
        Ok(1 + response_to_wire(msg)?.length())
    }
}

// ---------------------------------------------------------------------------
// Certificates: Malachite <-> xdevs (§6.5)
// ---------------------------------------------------------------------------

/// Malachite commit certificate → xdevs [`crate::CommitCertificate`] (sorted signatures).
pub(crate) fn to_xdevs_cert(c: &CommitCertificate<XdevsContext>) -> crate::CommitCertificate {
    crate::CommitCertificate::new(
        c.height.0,
        super::context::round_u32(c.round),
        c.value_id,
        c.commit_signatures
            .iter()
            .map(|s| crate::CommitSig {
                validator: s.address,
                signature: s.signature.to_bytes(),
            })
            .collect(),
    )
}

/// xdevs certificate → Malachite extended certificate (no extensions) for sync.
pub(crate) fn to_malachite_cert(
    c: &crate::CommitCertificate,
) -> ExtendedCommitCertificate<XdevsContext> {
    ExtendedCommitCertificate {
        height: Height(c.height),
        round: Round::new(c.round),
        value_id: c.block_hash,
        commit_signatures: c
            .signatures
            .iter()
            .map(|s| ExtendedCommitSignature::new(s.validator, sig_dec(s.signature), None))
            .collect(),
    }
}

/// Plain Malachite commit certificate from the xdevs form (tests: inverse check).
#[cfg(test)]
pub(crate) fn to_malachite_commit(c: &crate::CommitCertificate) -> CommitCertificate<XdevsContext> {
    use arc_malachitebft_app_channel::app::types::core::CommitSignature;
    CommitCertificate {
        height: Height(c.height),
        round: Round::new(c.round),
        value_id: c.block_hash,
        commit_signatures: c
            .signatures
            .iter()
            .map(|s| CommitSignature::new(s.validator, sig_dec(s.signature)))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

#[derive(RlpEncodable, RlpDecodable)]
struct WBlock {
    header: Header,
    txs: Vec<Bytes>,
}

/// `rlp([header, [tx_2718, ...]])`: the block bytes streamed in `Data` parts and served
/// by sync.
pub(crate) fn encode_block(block: &Block) -> Vec<u8> {
    alloy_rlp::encode(WBlock {
        header: block.header.clone(),
        txs: block
            .body
            .iter()
            .map(|tx| Bytes::from(tx.encoded_2718()))
            .collect(),
    })
}

/// Inverse of [`encode_block`]; rejects trailing bytes, non-canonical txs and a body
/// that does not match the header's `transactions_root`. The block hash is the header
/// hash, so only with this check does `(height, hash)` identify one block: a block
/// that passes can be stored and served under its hash.
pub(crate) fn decode_block(bytes: &[u8]) -> Result<Block, CodecError> {
    let w: WBlock = alloy_rlp::decode_exact(bytes)?;
    let body = w
        .txs
        .iter()
        .map(|raw| {
            let mut buf = &raw[..];
            let tx = TxEnvelope::decode_2718(&mut buf)
                .map_err(|e| CodecError(format!("bad transaction: {e}")))?;
            if !buf.is_empty() {
                return Err(CodecError::new("trailing bytes after transaction"));
            }
            Ok(tx)
        })
        .collect::<Result<_, _>>()?;
    let block = Block::new(w.header, body);
    if !body_matches_header(&block) {
        return Err(CodecError::new("transactions root does not match the body"));
    }
    Ok(block)
}

/// `header.transactions_root` commits to the body.
pub(crate) fn body_matches_header(block: &Block) -> bool {
    block.header.transactions_root == block.compute_transactions_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::malachite::context::tests::test_set;
    use crate::malachite::PrivateKey;
    use arc_malachitebft_app_channel::app::types::core::Context as _;

    fn codec() -> XdevsCodec {
        XdevsCodec
    }

    fn rt<T>(msg: &T) -> T
    where
        XdevsCodec: Codec<T>,
        <XdevsCodec as Codec<T>>::Error: std::fmt::Debug,
    {
        let bytes = Codec::<T>::encode(&codec(), msg).unwrap();
        assert_eq!(bytes[0], CODEC_VERSION);
        Codec::<T>::decode(&codec(), bytes).unwrap()
    }

    fn signed_vote(k: &PrivateKey, typ: MVoteType, value: NilOrVal<B256>) -> MSignedVote {
        let ctx = XdevsContext::new(7);
        let addr = crate::types::ValidatorKey(*k.public_key().as_bytes()).address();
        let v = match typ {
            MVoteType::Prevote => ctx.new_prevote(Height(5), Round::new(2), value, addr),
            MVoteType::Precommit => ctx.new_precommit(Height(5), Round::new(2), value, addr),
        };
        SignedMessage::new(v, k.sign(b"x"))
    }

    #[test]
    fn consensus_messages_roundtrip() {
        let (keys, _) = test_set(1);
        let k = &keys[0];
        for v in [
            signed_vote(k, MVoteType::Prevote, NilOrVal::Nil),
            signed_vote(k, MVoteType::Precommit, NilOrVal::Val(B256::repeat_byte(3))),
        ] {
            let m = SignedConsensusMsg::Vote(v.clone());
            assert_eq!(rt(&m), m);
            let l = LivenessMsg::Vote(v);
            assert_eq!(rt(&l), l);
        }
        let p = SignedMessage::new(
            Proposal {
                height: Height(9),
                round: Round::new(1),
                value: BlockValue::new(B256::repeat_byte(4)),
                pol_round: Round::Nil,
                validator_address: ValidatorAddress([1; 20]),
            },
            k.sign(b"p"),
        );
        let m = SignedConsensusMsg::Proposal(p);
        assert_eq!(rt(&m), m);
    }

    #[test]
    fn certificates_roundtrip() {
        let (keys, _) = test_set(2);
        let polka = PolkaCertificate::<XdevsContext> {
            height: Height(3),
            round: Round::new(0),
            value_id: B256::repeat_byte(1),
            polka_signatures: keys
                .iter()
                .map(|k| PolkaSignature::new(ValidatorAddress([2; 20]), k.sign(b"a")))
                .collect(),
        };
        assert_eq!(rt(&polka), polka);
        let l = LivenessMsg::PolkaCertificate(polka);
        assert_eq!(rt(&l), l);
        let rc = RoundCertificate::<XdevsContext> {
            height: Height(3),
            round: Round::new(4),
            cert_type: RoundCertificateType::Skip,
            round_signatures: vec![
                RoundSignature::new(
                    MVoteType::Prevote,
                    NilOrVal::Nil,
                    ValidatorAddress([3; 20]),
                    keys[0].sign(b"b"),
                ),
                RoundSignature::new(
                    MVoteType::Precommit,
                    NilOrVal::Val(B256::repeat_byte(9)),
                    ValidatorAddress([4; 20]),
                    keys[1].sign(b"c"),
                ),
            ],
        };
        let l = LivenessMsg::SkipRoundCertificate(rc);
        assert_eq!(rt(&l), l);
    }

    #[test]
    fn parts_and_streams_roundtrip() {
        let parts = [
            ProposalPart::Init {
                height: Height(1),
                round: Round::new(0),
                pol_round: Round::Nil,
                proposer: ValidatorAddress([5; 20]),
            },
            ProposalPart::Data(vec![1, 2, 3]),
            ProposalPart::Data(vec![]),
            ProposalPart::Fin { signature: [7; 64] },
        ];
        for (i, p) in parts.iter().enumerate() {
            assert_eq!(&rt(p), p);
            let s = StreamMessage::new(
                StreamId::new(RawBytes::from_static(b"sid")),
                i as u64,
                StreamContent::Data(p.clone()),
            );
            assert_eq!(rt(&s), s);
        }
        let fin = StreamMessage::<ProposalPart>::new(
            StreamId::new(RawBytes::from_static(b"sid")),
            4,
            StreamContent::Fin,
        );
        assert_eq!(rt(&fin), fin);
    }

    #[test]
    fn wal_and_sync_types_roundtrip() {
        let (keys, _) = test_set(2);
        let pv = ProposedValue::<XdevsContext> {
            height: Height(4),
            round: Round::new(1),
            valid_round: Round::Nil,
            proposer: ValidatorAddress([6; 20]),
            value: BlockValue::new(B256::repeat_byte(6)),
            validity: Validity::Invalid,
        };
        assert_eq!(rt(&pv), pv);

        let proof =
            ValidatorProof::<XdevsContext>::new(vec![1; 32], vec![2; 38], keys[0].sign(b"q"));
        assert_eq!(rt(&proof), proof);

        let peer = crate::malachite::streaming::tests::rp();
        let st = Status::<XdevsContext> {
            peer_id: peer,
            tip_height: Height(10),
            history_min_height: Height(1),
        };
        assert_eq!(rt(&st), st);

        let req = Request::<XdevsContext>::ValueRequest(ValueRequest::new(Height(2)..=Height(5)));
        assert_eq!(rt(&req), req);

        let cert = crate::CommitCertificate::new(
            4,
            1,
            B256::repeat_byte(8),
            keys.iter()
                .enumerate()
                .map(|(i, k)| crate::CommitSig {
                    validator: ValidatorAddress([i as u8; 20]),
                    signature: k.sign(b"z").to_bytes(),
                })
                .collect(),
        );
        let resp = Response::<XdevsContext>::ValueResponse(ValueResponse::new(
            Height(4),
            vec![RawDecidedValue::new(
                RawBytes::from_static(b"block"),
                to_malachite_cert(&cert),
            )],
        ));
        let bytes = Codec::<Response<XdevsContext>>::encode(&codec(), &resp).unwrap();
        assert_eq!(codec().encoded_len(&resp).unwrap(), bytes.len());
        assert_eq!(rt(&resp), resp);
        // Certificate conversions are inverse.
        assert_eq!(to_xdevs_cert(&to_malachite_commit(&cert)), cert);
    }

    #[test]
    fn rejects_bad_input() {
        let c = codec();
        assert!(Codec::<ProposalPart>::decode(&c, RawBytes::new()).is_err());
        let good = Codec::<ProposalPart>::encode(&c, &ProposalPart::Data(vec![1])).unwrap();
        let mut bad_version = good.to_vec();
        bad_version[0] = 9;
        assert!(Codec::<ProposalPart>::decode(&c, bad_version.into()).is_err());
        let mut trailing = good.to_vec();
        trailing.push(0);
        assert!(Codec::<ProposalPart>::decode(&c, trailing.into()).is_err());
        assert!(round_dec(u64::from(u32::MAX) + 2).is_err());
        assert_eq!(round_dec(round_enc(Round::Nil)).unwrap(), Round::Nil);
        assert_eq!(
            round_dec(round_enc(Round::new(u32::MAX))).unwrap(),
            Round::new(u32::MAX)
        );
    }

    #[test]
    fn block_roundtrip() {
        let mut header = Header {
            number: 3,
            ..Default::default()
        };
        header.extra_data = Bytes::from_static(b"hello");
        let block = Block::new(header, vec![]);
        let bytes = encode_block(&block);
        let back = decode_block(&bytes).unwrap();
        assert_eq!(back.hash(), block.hash());
        assert_eq!(back, block);
        assert!(decode_block(&bytes[..bytes.len() - 1]).is_err());
    }

    /// Finding 5: a body that does not match the header's transactions root is refused
    /// at decode, so it can never be stored under the header's hash.
    #[test]
    fn rejects_a_body_that_does_not_match_the_header() {
        let header = Header {
            number: 3,
            transactions_root: B256::repeat_byte(1),
            ..Default::default()
        };
        let block = Block::new(header, vec![]);
        assert!(!body_matches_header(&block));
        let err = decode_block(&encode_block(&block)).unwrap_err();
        assert!(err.to_string().contains("transactions root"), "{err}");
    }

    /// Finding 10: `Round::Nil` is refused wherever a real round is required.
    #[test]
    fn rejects_nil_rounds_on_the_wire() {
        let c = codec();
        let (keys, _) = test_set(1);
        let mut v = signed_vote(&keys[0], MVoteType::Prevote, NilOrVal::Nil);
        v.message.round = Round::Nil;
        let m = SignedConsensusMsg::Vote(v.clone());
        let bytes = Codec::<SignedConsensusMsg<XdevsContext>>::encode(&c, &m).unwrap();
        assert!(Codec::<SignedConsensusMsg<XdevsContext>>::decode(&c, bytes).is_err());
        let l = LivenessMsg::Vote(v);
        let bytes = Codec::<LivenessMsg<XdevsContext>>::encode(&c, &l).unwrap();
        assert!(Codec::<LivenessMsg<XdevsContext>>::decode(&c, bytes).is_err());

        let p = SignedMessage::new(
            Proposal {
                height: Height(9),
                round: Round::Nil,
                value: BlockValue::new(B256::repeat_byte(4)),
                pol_round: Round::Nil,
                validator_address: ValidatorAddress([1; 20]),
            },
            keys[0].sign(b"p"),
        );
        let m = SignedConsensusMsg::Proposal(p);
        let bytes = Codec::<SignedConsensusMsg<XdevsContext>>::encode(&c, &m).unwrap();
        assert!(Codec::<SignedConsensusMsg<XdevsContext>>::decode(&c, bytes).is_err());

        let init = |round| ProposalPart::Init {
            height: Height(1),
            round,
            pol_round: Round::Nil,
            proposer: ValidatorAddress([5; 20]),
        };
        let bytes = Codec::<ProposalPart>::encode(&c, &init(Round::Nil)).unwrap();
        assert!(Codec::<ProposalPart>::decode(&c, bytes).is_err());
        // A nil POL round stays fine.
        assert_eq!(rt(&init(Round::new(0))), init(Round::new(0)));

        let pv = ProposedValue::<XdevsContext> {
            height: Height(4),
            round: Round::Nil,
            valid_round: Round::Nil,
            proposer: ValidatorAddress([6; 20]),
            value: BlockValue::new(B256::repeat_byte(6)),
            validity: Validity::Valid,
        };
        let bytes = Codec::<ProposedValue<XdevsContext>>::encode(&c, &pv).unwrap();
        assert!(Codec::<ProposedValue<XdevsContext>>::decode(&c, bytes).is_err());
        assert!(round_dec_some(0).is_err());
        assert_eq!(round_dec_some(1).unwrap(), Round::new(0));
    }

    /// Finding 4: stream ids are capped.
    #[test]
    fn rejects_long_stream_ids() {
        let c = codec();
        let ok = StreamMessage::<ProposalPart>::new(
            StreamId::new(RawBytes::from(vec![1u8; MAX_STREAM_ID_LEN])),
            0,
            StreamContent::Fin,
        );
        assert_eq!(rt(&ok), ok);
        let long = StreamMessage::<ProposalPart>::new(
            StreamId::new(RawBytes::from(vec![1u8; MAX_STREAM_ID_LEN + 1])),
            0,
            StreamContent::Fin,
        );
        let bytes = Codec::<StreamMessage<ProposalPart>>::encode(&c, &long).unwrap();
        assert!(Codec::<StreamMessage<ProposalPart>>::decode(&c, bytes).is_err());
    }
}

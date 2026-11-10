//! xdevs consensus types (research doc §6.4–§6.6): validators, validator sets, canonical
//! sign-bytes, commit certificates and evidence. **Frozen**: change only additively.
//!
//! No Malachite types appear here. Everything is deterministic: validators are kept sorted
//! by address, voting-power math is integer-only, and every encoding is fixed-length or RLP.

use alloy_rlp::{RlpDecodable, RlpDecodableWrapper, RlpEncodable, RlpEncodableWrapper};
use sha2::{Digest, Sha256};
use std::fmt;
use xdevs_primitives::{hex, B256};

/// Domain tag of vote sign-bytes.
pub const VOTE_DOMAIN: &[u8] = b"XDEVS/VOTE/1";
/// Domain tag of proposal sign-bytes.
pub const PROPOSAL_DOMAIN: &[u8] = b"XDEVS/PROPOSAL/1";
/// Domain tag of the proposal-parts (`Fin`) signature.
pub const PARTS_DOMAIN: &[u8] = b"XDEVS/PARTS/1";

/// Ed25519 signature length.
pub const SIGNATURE_LEN: usize = 64;

// ---------------------------------------------------------------------------
// Keys and addresses
// ---------------------------------------------------------------------------

/// A consensus validator address: `sha256(ed25519_pubkey)[..20]` (the CometBFT
/// convention; deliberately distinct from EVM addresses).
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    RlpEncodableWrapper,
    RlpDecodableWrapper,
)]
pub struct ValidatorAddress(pub [u8; 20]);

impl ValidatorAddress {
    /// Address of an Ed25519 public key.
    pub fn from_public_key(key: &ValidatorKey) -> Self {
        let digest = Sha256::digest(key.0);
        let mut out = [0u8; 20];
        out.copy_from_slice(&digest[..20]);
        Self(out)
    }

    /// Raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

impl fmt::Debug for ValidatorAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValidatorAddress({self})")
    }
}

impl fmt::Display for ValidatorAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

/// An Ed25519 consensus public key (32 bytes). Signatures are verified with
/// `ed25519-consensus` (ZIP-215 rules), the same scheme Malachite uses.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValidatorKey(pub [u8; 32]);

impl ValidatorKey {
    /// Wrap 32 bytes, checking that they decode to an Ed25519 point.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, ValidatorSetError> {
        ed25519_consensus::VerificationKey::try_from(bytes)
            .map_err(|_| ValidatorSetError::InvalidKey(hex::encode(bytes)))?;
        Ok(Self(bytes))
    }

    /// The validator address of this key.
    pub fn address(&self) -> ValidatorAddress {
        ValidatorAddress::from_public_key(self)
    }

    /// Verify an Ed25519 `signature` over `msg` (ZIP-215). `false` on any failure.
    pub fn verify(&self, msg: &[u8], signature: &[u8; SIGNATURE_LEN]) -> bool {
        let Ok(vk) = ed25519_consensus::VerificationKey::try_from(self.0) else {
            return false;
        };
        vk.verify(&ed25519_consensus::Signature::from(*signature), msg)
            .is_ok()
    }
}

impl fmt::Debug for ValidatorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValidatorKey(0x{})", hex::encode(self.0))
    }
}

// ---------------------------------------------------------------------------
// Validators and validator sets
// ---------------------------------------------------------------------------

/// One validator of a [`ValidatorSet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Validator {
    /// `sha256(public_key)[..20]`.
    pub address: ValidatorAddress,
    /// Ed25519 consensus key.
    pub public_key: ValidatorKey,
    /// Voting power (≥ 1).
    pub voting_power: u64,
}

impl Validator {
    /// A validator with its address derived from `public_key`.
    pub fn new(public_key: ValidatorKey, voting_power: u64) -> Self {
        Self {
            address: public_key.address(),
            public_key,
            voting_power,
        }
    }
}

/// Errors building a [`ValidatorSet`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidatorSetError {
    /// The set has no validators.
    #[error("validator set is empty")]
    Empty,
    /// A validator has zero voting power.
    #[error("validator {0} has zero voting power")]
    ZeroPower(ValidatorAddress),
    /// Two validators share an address (and therefore a key).
    #[error("duplicate validator {0}")]
    Duplicate(ValidatorAddress),
    /// `address != sha256(public_key)[..20]`.
    #[error("validator address {0} does not match its public key")]
    AddressMismatch(ValidatorAddress),
    /// Bytes are not a valid Ed25519 public key.
    #[error("invalid ed25519 public key 0x{0}")]
    InvalidKey(String),
    /// Total voting power overflows `u64`.
    #[error("total voting power overflows u64")]
    PowerOverflow,
}

/// A validator set, sorted by address, with its total voting power (§6.4).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValidatorSet {
    validators: Vec<Validator>,
    total_voting_power: u64,
}

impl ValidatorSet {
    /// Build a set: sorts by address and rejects empty sets, zero power, duplicates,
    /// mismatched addresses, invalid keys and power overflow.
    pub fn new(mut validators: Vec<Validator>) -> Result<Self, ValidatorSetError> {
        if validators.is_empty() {
            return Err(ValidatorSetError::Empty);
        }
        validators.sort_by_key(|v| v.address);
        let mut total: u64 = 0;
        for (i, v) in validators.iter().enumerate() {
            ValidatorKey::from_bytes(v.public_key.0)?;
            if v.address != v.public_key.address() {
                return Err(ValidatorSetError::AddressMismatch(v.address));
            }
            if v.voting_power == 0 {
                return Err(ValidatorSetError::ZeroPower(v.address));
            }
            if i > 0 && validators[i - 1].address == v.address {
                return Err(ValidatorSetError::Duplicate(v.address));
            }
            total = total
                .checked_add(v.voting_power)
                .ok_or(ValidatorSetError::PowerOverflow)?;
        }
        Ok(Self {
            validators,
            total_voting_power: total,
        })
    }

    /// Validators in address order.
    pub fn validators(&self) -> &[Validator] {
        &self.validators
    }

    /// Number of validators.
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Always `false` (a set is never empty); provided for clippy.
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }

    /// Sum of all voting power.
    pub const fn total_voting_power(&self) -> u64 {
        self.total_voting_power
    }

    /// Look a validator up by address.
    pub fn get(&self, address: &ValidatorAddress) -> Option<&Validator> {
        self.validators
            .binary_search_by_key(address, |v| v.address)
            .ok()
            .map(|i| &self.validators[i])
    }

    /// Validator at `index` (address order).
    pub fn get_by_index(&self, index: usize) -> Option<&Validator> {
        self.validators.get(index)
    }

    /// `true` iff `power > 2/3` of the total (BFT quorum, integer math).
    pub fn is_quorum(&self, power: u64) -> bool {
        u128::from(power) * 3 > u128::from(self.total_voting_power) * 2
    }

    /// Smallest power that is a quorum: `floor(2 * total / 3) + 1`.
    pub fn quorum_threshold(&self) -> u64 {
        // Fits in u64: 2 * total / 3 < total.
        (u128::from(self.total_voting_power) * 2 / 3 + 1) as u64
    }

    /// Phase-2 proposer (§6.6): `k = (height + round) mod total_power`, then walk the
    /// validators in address order accumulating power until the running sum exceeds `k`.
    /// A pure function; equal powers give plain round-robin.
    pub fn select_proposer(&self, height: u64, round: u32) -> &Validator {
        let total = u128::from(self.total_voting_power);
        let k = (u128::from(height) + u128::from(round)) % total;
        let mut sum: u128 = 0;
        for v in &self.validators {
            sum += u128::from(v.voting_power);
            if sum > k {
                return v;
            }
        }
        unreachable!("k < total voting power")
    }
}

// ---------------------------------------------------------------------------
// Sign-bytes (§6.4)
// ---------------------------------------------------------------------------

/// Vote type, encoded as one byte in the sign-bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum VoteType {
    /// First voting step.
    Prevote = 1,
    /// Second voting step; precommits for a block form its [`CommitCertificate`].
    Precommit = 2,
}

/// `"XDEVS/VOTE/1" ‖ chain_id:u64be ‖ type:u8 ‖ height:u64be ‖ round:u32be ‖
/// (0x00 | 0x01 ‖ block_hash)`. `None` = a nil vote.
pub fn vote_sign_bytes(
    chain_id: u64,
    vote_type: VoteType,
    height: u64,
    round: u32,
    block_hash: Option<B256>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(VOTE_DOMAIN.len() + 8 + 1 + 8 + 4 + 33);
    out.extend_from_slice(VOTE_DOMAIN);
    out.extend_from_slice(&chain_id.to_be_bytes());
    out.push(vote_type as u8);
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&round.to_be_bytes());
    match block_hash {
        None => out.push(0x00),
        Some(h) => {
            out.push(0x01);
            out.extend_from_slice(h.as_slice());
        }
    }
    out
}

/// `"XDEVS/PROPOSAL/1" ‖ chain_id:u64be ‖ height:u64be ‖ round:u32be ‖ pol_round:i64be ‖
/// block_hash`. `pol_round = None` encodes as `-1`.
pub fn proposal_sign_bytes(
    chain_id: u64,
    height: u64,
    round: u32,
    pol_round: Option<u32>,
    block_hash: B256,
) -> Vec<u8> {
    let pol: i64 = pol_round.map_or(-1, i64::from);
    let mut out = Vec::with_capacity(PROPOSAL_DOMAIN.len() + 8 + 8 + 4 + 8 + 32);
    out.extend_from_slice(PROPOSAL_DOMAIN);
    out.extend_from_slice(&chain_id.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&round.to_be_bytes());
    out.extend_from_slice(&pol.to_be_bytes());
    out.extend_from_slice(block_hash.as_slice());
    out
}

/// `"XDEVS/PARTS/1" ‖ chain_id:u64be ‖ height:u64be ‖ round:u32be ‖ parts_hash`, where
/// `parts_hash = keccak256(all Data chunks concatenated)`. Signed in the `Fin` part.
pub fn parts_sign_bytes(chain_id: u64, height: u64, round: u32, parts_hash: B256) -> Vec<u8> {
    let mut out = Vec::with_capacity(PARTS_DOMAIN.len() + 8 + 8 + 4 + 32);
    out.extend_from_slice(PARTS_DOMAIN);
    out.extend_from_slice(&chain_id.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&round.to_be_bytes());
    out.extend_from_slice(parts_hash.as_slice());
    out
}

// ---------------------------------------------------------------------------
// Commit certificates (§6.5)
// ---------------------------------------------------------------------------

/// One precommit signature inside a [`CommitCertificate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable)]
pub struct CommitSig {
    /// Signing validator.
    pub validator: ValidatorAddress,
    /// Ed25519 signature over the precommit sign-bytes.
    pub signature: [u8; SIGNATURE_LEN],
}

/// Errors verifying a [`CommitCertificate`] or decoding one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CertificateError {
    /// The certificate is for another height than expected.
    #[error("certificate height {found} != expected {expected}")]
    WrongHeight {
        /// Height in the certificate.
        found: u64,
        /// Height expected by the caller.
        expected: u64,
    },
    /// Signatures are not strictly sorted by validator (unsorted or duplicate).
    #[error("signatures not strictly sorted by validator at {0}")]
    NotSorted(ValidatorAddress),
    /// A signer is not in the validator set.
    #[error("unknown validator {0}")]
    UnknownValidator(ValidatorAddress),
    /// A signature does not verify.
    #[error("invalid signature from {0}")]
    InvalidSignature(ValidatorAddress),
    /// Signed power is not more than 2/3 of the total.
    #[error("insufficient voting power: {signed} of {total}")]
    InsufficientPower {
        /// Power that signed.
        signed: u64,
        /// Total power of the set.
        total: u64,
    },
    /// Malformed RLP.
    #[error("invalid certificate encoding: {0}")]
    Decode(String),
}

/// A commit certificate: > 2/3 of the voting power precommitted `block_hash` at
/// `(height, round)`. RLP-encoded and stored per height in the `CommitCertificates`
/// table, in the same `WriteBatch` as the block (§6.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable)]
pub struct CommitCertificate {
    /// Decided height.
    pub height: u64,
    /// Round of the decision.
    pub round: u32,
    /// Decided block hash (the Ethereum block hash).
    pub block_hash: B256,
    /// Precommit signatures, strictly sorted by validator address.
    pub signatures: Vec<CommitSig>,
}

impl CommitCertificate {
    /// A certificate with its signatures sorted by validator (duplicates are kept and
    /// will fail [`CommitCertificate::verify`]).
    pub fn new(height: u64, round: u32, block_hash: B256, mut signatures: Vec<CommitSig>) -> Self {
        signatures.sort_by_key(|s| s.validator);
        Self {
            height,
            round,
            block_hash,
            signatures,
        }
    }

    /// Canonical RLP bytes (the stored and served form).
    pub fn to_bytes(&self) -> Vec<u8> {
        alloy_rlp::encode(self)
    }

    /// Decode canonical RLP, rejecting trailing bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CertificateError> {
        alloy_rlp::decode_exact(bytes).map_err(|e| CertificateError::Decode(e.to_string()))
    }

    /// Sign-bytes every signature in this certificate signs (a precommit for the block).
    pub fn sign_bytes(&self, chain_id: u64) -> Vec<u8> {
        vote_sign_bytes(
            chain_id,
            VoteType::Precommit,
            self.height,
            self.round,
            Some(self.block_hash),
        )
    }

    /// Verify against the validator set at `self.height`: signatures strictly sorted (no
    /// duplicates), every signer known, every signature valid, signed power > 2/3.
    /// Returns the signed voting power.
    pub fn verify(&self, chain_id: u64, set: &ValidatorSet) -> Result<u64, CertificateError> {
        let msg = self.sign_bytes(chain_id);
        let mut signed: u64 = 0;
        let mut prev: Option<ValidatorAddress> = None;
        for sig in &self.signatures {
            if prev.is_some_and(|p| p >= sig.validator) {
                return Err(CertificateError::NotSorted(sig.validator));
            }
            prev = Some(sig.validator);
            let v = set
                .get(&sig.validator)
                .ok_or(CertificateError::UnknownValidator(sig.validator))?;
            if !v.public_key.verify(&msg, &sig.signature) {
                return Err(CertificateError::InvalidSignature(sig.validator));
            }
            // Cannot overflow: the set's total fits in u64 and signers are unique.
            signed += v.voting_power;
        }
        if !set.is_quorum(signed) {
            return Err(CertificateError::InsufficientPower {
                signed,
                total: set.total_voting_power(),
            });
        }
        Ok(signed)
    }

    /// [`CommitCertificate::verify`] plus a check that the certificate is for `height`.
    pub fn verify_at(
        &self,
        chain_id: u64,
        height: u64,
        set: &ValidatorSet,
    ) -> Result<u64, CertificateError> {
        if self.height != height {
            return Err(CertificateError::WrongHeight {
                found: self.height,
                expected: height,
            });
        }
        self.verify(chain_id, set)
    }
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// A signed vote, as needed to prove equivocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignedVote {
    /// Prevote or precommit.
    pub vote_type: VoteType,
    /// Height.
    pub height: u64,
    /// Round.
    pub round: u32,
    /// Voted block (`None` = nil).
    pub block_hash: Option<B256>,
    /// Voter.
    pub validator: ValidatorAddress,
    /// Ed25519 signature over [`vote_sign_bytes`].
    pub signature: [u8; SIGNATURE_LEN],
}

impl SignedVote {
    /// The bytes this vote signs.
    pub fn sign_bytes(&self, chain_id: u64) -> Vec<u8> {
        vote_sign_bytes(
            chain_id,
            self.vote_type,
            self.height,
            self.round,
            self.block_hash,
        )
    }

    /// `true` iff the signature verifies under `key`.
    pub fn verify(&self, chain_id: u64, key: &ValidatorKey) -> bool {
        key.verify(&self.sign_bytes(chain_id), &self.signature)
    }
}

/// Two votes by the same validator for the same (height, round, type) and different
/// values: double-sign evidence (plan §11.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConflictingVotes {
    /// First vote.
    pub first: SignedVote,
    /// Second, conflicting vote.
    pub second: SignedVote,
}

impl ConflictingVotes {
    /// `true` iff this is real evidence against a member of `set`: same voter, height,
    /// round and type, different values, both signatures valid.
    pub fn is_valid(&self, chain_id: u64, set: &ValidatorSet) -> bool {
        let (a, b) = (&self.first, &self.second);
        if a.validator != b.validator
            || a.height != b.height
            || a.round != b.round
            || a.vote_type != b.vote_type
            || a.block_hash == b.block_hash
        {
            return false;
        }
        let Some(v) = set.get(&a.validator) else {
            return false;
        };
        a.verify(chain_id, &v.public_key) && b.verify(chain_id, &v.public_key)
    }
}

/// Misbehaviour reported by the engine to [`crate::ConsensusApp::report_evidence`]
/// (logged in Phase 2, slashed in Phase 3).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Evidence {
    /// Two conflicting votes from one validator.
    DoubleVote(ConflictingVotes),
    /// Two different signed proposals from the proposer of one (height, round).
    DoubleProposal {
        /// Offending proposer.
        validator: ValidatorAddress,
        /// Height.
        height: u64,
        /// Round.
        round: u32,
        /// The two proposed block hashes.
        block_hashes: (B256, B256),
    },
}

impl Evidence {
    /// The offending validator.
    pub fn validator(&self) -> ValidatorAddress {
        match self {
            Self::DoubleVote(c) => c.first.validator,
            Self::DoubleProposal { validator, .. } => *validator,
        }
    }

    /// The height of the offence.
    pub fn height(&self) -> u64 {
        match self {
            Self::DoubleVote(c) => c.first.height,
            Self::DoubleProposal { height, .. } => *height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_consensus::SigningKey;

    const CHAIN: u64 = 7117;

    fn key(i: u8) -> SigningKey {
        SigningKey::from([i; 32])
    }

    fn vkey(sk: &SigningKey) -> ValidatorKey {
        ValidatorKey(sk.verification_key().to_bytes())
    }

    fn set(powers: &[u64]) -> (Vec<SigningKey>, ValidatorSet) {
        let sks: Vec<_> = (1..=powers.len() as u8).map(key).collect();
        let vals = sks
            .iter()
            .zip(powers)
            .map(|(sk, p)| Validator::new(vkey(sk), *p))
            .collect();
        (sks, ValidatorSet::new(vals).unwrap())
    }

    fn cert(sks: &[SigningKey], height: u64, round: u32, hash: B256) -> CommitCertificate {
        let msg = vote_sign_bytes(CHAIN, VoteType::Precommit, height, round, Some(hash));
        let sigs = sks
            .iter()
            .map(|sk| CommitSig {
                validator: vkey(sk).address(),
                signature: sk.sign(&msg).to_bytes(),
            })
            .collect();
        CommitCertificate::new(height, round, hash, sigs)
    }

    #[test]
    fn address_is_sha256_prefix() {
        let k = vkey(&key(1));
        let d = Sha256::digest(k.0);
        assert_eq!(k.address().0, d[..20]);
        assert_eq!(k.address().to_string().len(), 42);
    }

    #[test]
    fn dalek_and_consensus_keys_agree() {
        // A 32-byte seed gives the same public key in ed25519-dalek (libp2p / keystores)
        // and ed25519-consensus (Malachite).
        let seed = [9u8; 32];
        let dalek = ed25519_dalek::SigningKey::from_bytes(&seed);
        assert_eq!(
            dalek.verifying_key().to_bytes(),
            SigningKey::from(seed).verification_key().to_bytes()
        );
    }

    #[test]
    fn validator_set_rules() {
        let (_, s) = set(&[1, 2, 3]);
        assert_eq!(s.len(), 3);
        assert_eq!(s.total_voting_power(), 6);
        assert!(s
            .validators()
            .windows(2)
            .all(|w| w[0].address < w[1].address));
        for v in s.validators() {
            assert_eq!(s.get(&v.address), Some(v));
        }
        assert!(s.get(&ValidatorAddress([0; 20])).is_none());

        assert_eq!(ValidatorSet::new(vec![]), Err(ValidatorSetError::Empty));
        let v = Validator::new(vkey(&key(1)), 0);
        assert_eq!(
            ValidatorSet::new(vec![v]),
            Err(ValidatorSetError::ZeroPower(v.address))
        );
        let v = Validator::new(vkey(&key(1)), 1);
        assert_eq!(
            ValidatorSet::new(vec![v, v]),
            Err(ValidatorSetError::Duplicate(v.address))
        );
        let bad = Validator {
            address: ValidatorAddress([1; 20]),
            ..v
        };
        assert_eq!(
            ValidatorSet::new(vec![bad]),
            Err(ValidatorSetError::AddressMismatch(bad.address))
        );
        let big = |i| Validator::new(vkey(&key(i)), u64::MAX);
        assert_eq!(
            ValidatorSet::new(vec![big(1), big(2)]),
            Err(ValidatorSetError::PowerOverflow)
        );
    }

    #[test]
    fn rejects_invalid_point() {
        // y = 2 is not on the curve.
        let mut b = [0u8; 32];
        b[0] = 2;
        assert!(ValidatorKey::from_bytes(b).is_err());
        assert!(ValidatorKey::from_bytes(vkey(&key(3)).0).is_ok());
    }

    #[test]
    fn quorum_math() {
        let (_, s) = set(&[1, 1, 1, 1]);
        assert_eq!(s.quorum_threshold(), 3);
        assert!(!s.is_quorum(2));
        assert!(s.is_quorum(3));
        let (_, s) = set(&[1, 1, 1]);
        assert_eq!(s.quorum_threshold(), 3);
        assert!(!s.is_quorum(2));
        let (_, s) = set(&[u64::MAX / 2, u64::MAX / 2]);
        assert!(s.is_quorum(s.total_voting_power()));
        assert!(!s.is_quorum(u64::MAX / 2));
        assert_eq!(
            s.quorum_threshold(),
            (u128::from(s.total_voting_power()) * 2 / 3 + 1) as u64
        );
    }

    #[test]
    fn proposer_round_robin_and_weighted() {
        let (_, s) = set(&[1, 1, 1, 1]);
        let vs = s.validators();
        for h in 0..8u64 {
            assert_eq!(s.select_proposer(h, 0), &vs[(h % 4) as usize]);
            assert_eq!(s.select_proposer(h, 1), &vs[((h + 1) % 4) as usize]);
        }
        // Weighted: counts over one full cycle equal the powers.
        let (_, s) = set(&[1, 2, 3]);
        let mut counts = std::collections::BTreeMap::new();
        for h in 0..6 {
            *counts
                .entry(s.select_proposer(h, 0).address)
                .or_insert(0u64) += 1;
        }
        for v in s.validators() {
            assert_eq!(counts[&v.address], v.voting_power);
        }
        // No overflow at the extremes.
        let _ = s.select_proposer(u64::MAX, u32::MAX);
    }

    #[test]
    fn sign_bytes_layout() {
        let h = B256::repeat_byte(0xab);
        let v = vote_sign_bytes(CHAIN, VoteType::Precommit, 5, 2, Some(h));
        assert_eq!(&v[..12], b"XDEVS/VOTE/1");
        assert_eq!(&v[12..20], &CHAIN.to_be_bytes());
        assert_eq!(v[20], 2);
        assert_eq!(&v[21..29], &5u64.to_be_bytes());
        assert_eq!(&v[29..33], &2u32.to_be_bytes());
        assert_eq!(v[33], 1);
        assert_eq!(&v[34..], h.as_slice());
        let nil = vote_sign_bytes(CHAIN, VoteType::Prevote, 5, 2, None);
        assert_eq!(nil.len(), 34);
        assert_eq!((nil[20], nil[33]), (1, 0));

        let p = proposal_sign_bytes(CHAIN, 5, 2, None, h);
        assert_eq!(&p[..16], b"XDEVS/PROPOSAL/1");
        assert_eq!(&p[36..44], &(-1i64).to_be_bytes());
        assert_eq!(p.len(), 16 + 8 + 8 + 4 + 8 + 32);
        let p1 = proposal_sign_bytes(CHAIN, 5, 2, Some(1), h);
        assert_eq!(&p1[36..44], &1i64.to_be_bytes());

        let f = parts_sign_bytes(CHAIN, 5, 2, h);
        assert_eq!(&f[..13], b"XDEVS/PARTS/1");
        assert_eq!(f.len(), 13 + 8 + 8 + 4 + 32);
        // Domains separate the message kinds.
        assert_ne!(&v[..16], &p[..16]);
    }

    #[test]
    fn certificate_roundtrip_and_verify() {
        let (sks, s) = set(&[1, 1, 1, 1]);
        let hash = B256::repeat_byte(7);
        let c = cert(&sks[..3], 10, 1, hash);
        assert_eq!(c.verify(CHAIN, &s), Ok(3));
        assert_eq!(c.verify_at(CHAIN, 10, &s), Ok(3));
        assert!(matches!(
            c.verify_at(CHAIN, 11, &s),
            Err(CertificateError::WrongHeight { .. })
        ));
        let bytes = c.to_bytes();
        assert_eq!(CommitCertificate::from_bytes(&bytes), Ok(c.clone()));
        // ~84 B per signer plus a small header.
        assert!(bytes.len() < 3 * 90 + 50, "{}", bytes.len());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(CommitCertificate::from_bytes(&trailing).is_err());
        assert!(CommitCertificate::from_bytes(&bytes[..bytes.len() - 1]).is_err());

        // Wrong chain id: signatures fail.
        assert!(matches!(
            c.verify(CHAIN + 1, &s),
            Err(CertificateError::InvalidSignature(_))
        ));
    }

    #[test]
    fn certificate_rejections() {
        let (sks, s) = set(&[1, 1, 1, 1]);
        let hash = B256::repeat_byte(7);
        // 2 of 4 is not > 2/3.
        assert_eq!(
            cert(&sks[..2], 1, 0, hash).verify(CHAIN, &s),
            Err(CertificateError::InsufficientPower {
                signed: 2,
                total: 4
            })
        );
        // Duplicate signer.
        let mut c = cert(&sks[..3], 1, 0, hash);
        c.signatures.push(c.signatures[2]);
        assert!(matches!(
            c.verify(CHAIN, &s),
            Err(CertificateError::NotSorted(_))
        ));
        // Unsorted.
        let mut c = cert(&sks[..3], 1, 0, hash);
        c.signatures.swap(0, 1);
        assert!(matches!(
            c.verify(CHAIN, &s),
            Err(CertificateError::NotSorted(_))
        ));
        // Unknown signer.
        let outsider = key(99);
        let c = cert(&[&sks[..3], &[outsider]].concat(), 1, 0, hash);
        assert!(matches!(
            c.verify(CHAIN, &s),
            Err(CertificateError::UnknownValidator(_))
        ));
        // Tampered hash.
        let mut c = cert(&sks[..3], 1, 0, hash);
        c.block_hash = B256::repeat_byte(8);
        assert!(matches!(
            c.verify(CHAIN, &s),
            Err(CertificateError::InvalidSignature(_))
        ));
    }

    #[test]
    fn evidence_validation() {
        let (sks, s) = set(&[1, 1, 1, 1]);
        let sk = &sks[0];
        let vote = |hash: Option<B256>| {
            let mut v = SignedVote {
                vote_type: VoteType::Prevote,
                height: 3,
                round: 0,
                block_hash: hash,
                validator: vkey(sk).address(),
                signature: [0; 64],
            };
            v.signature = sk.sign(&v.sign_bytes(CHAIN)).to_bytes();
            v
        };
        let a = vote(Some(B256::repeat_byte(1)));
        let b = vote(Some(B256::repeat_byte(2)));
        let ev = ConflictingVotes {
            first: a,
            second: b,
        };
        assert!(ev.is_valid(CHAIN, &s));
        assert!(!ConflictingVotes {
            first: a,
            second: a
        }
        .is_valid(CHAIN, &s));
        let mut forged = b;
        forged.signature[0] ^= 1;
        assert!(!ConflictingVotes {
            first: a,
            second: forged
        }
        .is_valid(CHAIN, &s));
        let nil = vote(None);
        assert!(ConflictingVotes {
            first: a,
            second: nil
        }
        .is_valid(CHAIN, &s));
        let e = Evidence::DoubleVote(ev);
        assert_eq!(e.validator(), a.validator);
        assert_eq!(e.height(), 3);
    }
}

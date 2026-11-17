//! Staking-driven validator set (Phase 3, plan §11.4–11.6): epoch validator sets, the
//! consensus data of blocks and the double-sign evidence pool.
//!
//! * **Epoch sets.** Until the first epoch boundary at or after
//!   `ChainConfig::staking_validators_block` the genesis set is in force. Executing a
//!   boundary block `B` (`B % epochLength == 0`) runs `Staking.onEpoch()` and reads
//!   `consensusValidatorSet()`; the node converts it ([`set_from_staking`]) and stores it
//!   under `B` in the `ValidatorSets` table in the block's commit batch. It governs the
//!   heights `B + 1 ..= B + epochLength` on every node ([`validator_set_at`]), so
//!   Malachite switches sets exactly at `B + 1`. Mid-epoch changes of the contract's
//!   active set (jailing, tombstoning, exits) take effect at the next boundary.
//! * **Consensus data.** Each block from the activation on carries
//!   `xdevs_consensus::BlockConsensusData`: its proposer, the parent's commit certificate
//!   (whose signers are the liveness bits of `recordParticipation`) and double-sign
//!   evidence. Validators check it before executing the block (`ChainApp`).
//! * **Evidence.** Conflicting votes reported by Malachite or submitted over RPC
//!   (`xdevs_submitEvidence`) are verified against the validator set of their height and
//!   kept in an [`EvidencePool`] until a block includes them.

use anyhow::{bail, Context};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use xdevs_consensus::{ConflictingVotes, Validator, ValidatorKey, ValidatorSet};
use xdevs_execution::StakingValidator;
use xdevs_primitives::{Bytes, ChainConfig, B256, U256};
use xdevs_storage::{tables, DbRead, DbReadExt};

/// Most evidence items in one block.
pub const MAX_EVIDENCE_PER_BLOCK: usize = 8;
/// Evidence older than this many blocks is refused (must stay well below the unbonding
/// period, 1 day = 43,200 blocks at 2 s, so the stake it slashes is still bonded or
/// unbonding: `docs/specs/predeploys-system.md` §7.2).
pub const MAX_EVIDENCE_AGE_BLOCKS: u64 = 10_000;
/// Most pending evidence items a node keeps.
pub const MAX_PENDING_EVIDENCE: usize = 256;

/// Wei per whole tXDS: Staking powers (wei) are converted to voting power in whole tXDS.
const WEI_PER_POWER: u64 = 1_000_000_000_000_000_000;
/// Cap of one validator's voting power (keeps the total of ≤ 256 members in `u64`).
const MAX_POWER: u64 = u64::MAX >> 16;

/// Canonical encoding of a validator set in the `ValidatorSets` table: for each
/// validator in address order, `public key (32 B) ‖ voting power (u64 big-endian)`.
pub fn encode_set(set: &ValidatorSet) -> Vec<u8> {
    let mut out = Vec::with_capacity(set.len() * 40);
    for v in set.validators() {
        out.extend_from_slice(&v.public_key.0);
        out.extend_from_slice(&v.voting_power.to_be_bytes());
    }
    out
}

/// Inverse of [`encode_set`].
pub fn decode_set(bytes: &[u8]) -> anyhow::Result<ValidatorSet> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(40) {
        bail!("stored validator set has {} bytes", bytes.len());
    }
    let validators = bytes
        .chunks_exact(40)
        .map(|c| {
            let mut key = [0u8; 32];
            key.copy_from_slice(&c[..32]);
            let mut power = [0u8; 8];
            power.copy_from_slice(&c[32..]);
            Ok(Validator::new(
                ValidatorKey::from_bytes(key)?,
                u64::from_be_bytes(power),
            ))
        })
        .collect::<Result<Vec<_>, xdevs_consensus::ValidatorSetError>>()
        .context("stored validator set")?;
    ValidatorSet::new(validators).context("stored validator set")
}

/// The consensus validator set for the Staking contract's set: voting power = bonded
/// stake in whole tXDS (`stake / 1e18`, at least 1), as `genesis.json` does. Members
/// whose consensus key is not a valid Ed25519 public key (`register` cannot check it)
/// are left out: they could never sign. `None` if no member is usable; the caller then
/// keeps the current set (the chain must never lose its set).
pub fn set_from_staking(validators: &[StakingValidator]) -> Option<ValidatorSet> {
    let mut members = Vec::with_capacity(validators.len());
    for v in validators {
        let Ok(key) = ValidatorKey::from_bytes(v.consensus_key.0) else {
            tracing::warn!(
                target: "xdevs::staking",
                operator = %v.operator,
                key = %v.consensus_key,
                "active validator has an invalid Ed25519 consensus key; left out of the set"
            );
            continue;
        };
        let power = v.stake / U256::from(WEI_PER_POWER);
        let power = u64::try_from(power)
            .unwrap_or(MAX_POWER)
            .clamp(1, MAX_POWER);
        members.push(Validator::new(key, power));
    }
    if members.is_empty() {
        return None;
    }
    ValidatorSet::new(members).ok()
}

/// Stored validator set fixed by epoch-boundary block `boundary`, if committed.
pub fn stored_set<R: DbRead + ?Sized>(db: &R, boundary: u64) -> anyhow::Result<Option<Bytes>> {
    Ok(db.get::<tables::ValidatorSets>(&boundary)?)
}

/// The validator set in force at `height`: the genesis set until the first epoch
/// boundary, then the set stored by the last boundary below `height`. Errors when that
/// boundary block is not committed yet (the set of a future epoch is unknown).
pub fn validator_set_at<R: DbRead + ?Sized>(
    db: &R,
    cfg: &ChainConfig,
    genesis: &ValidatorSet,
    height: u64,
) -> anyhow::Result<ValidatorSet> {
    match cfg.validator_set_boundary(height) {
        None => Ok(genesis.clone()),
        Some(boundary) => match stored_set(db, boundary)? {
            Some(raw) => decode_set(&raw),
            None => bail!(
                "validator set of height {height} is not known yet: epoch block {boundary} \
                 is not committed"
            ),
        },
    }
}

/// Verified double-sign evidence waiting for inclusion, by evidence hash.
#[derive(Debug, Default)]
pub struct EvidencePool {
    pending: Mutex<BTreeMap<B256, ConflictingVotes>>,
}

impl EvidencePool {
    /// Add verified, canonical evidence. Returns `false` if already pending or the pool
    /// is full.
    pub fn add(&self, evidence: ConflictingVotes) -> bool {
        let mut pending = self.pending.lock();
        if pending.len() >= MAX_PENDING_EVIDENCE {
            return false;
        }
        pending.insert(evidence.hash(), evidence).is_none()
    }

    /// Up to `limit` pending items, oldest offence first, that `usable` accepts.
    pub fn select(
        &self,
        limit: usize,
        usable: impl FnMut(&ConflictingVotes) -> bool,
    ) -> Vec<ConflictingVotes> {
        let mut items: Vec<ConflictingVotes> = self.pending.lock().values().copied().collect();
        items.sort_by_key(|e| (e.first.height, e.hash()));
        items.into_iter().filter(usable).take(limit).collect()
    }

    /// Forget the given evidence hashes and every item whose height is at or below
    /// `expired` (too old to be included).
    pub fn prune(&self, included: &[B256], expired: u64) {
        let mut pending = self.pending.lock();
        for h in included {
            pending.remove(h);
        }
        pending.retain(|_, e| e.first.height > expired);
    }

    /// Number of pending items.
    pub fn len(&self) -> usize {
        self.pending.lock().len()
    }

    /// `true` if nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_consensus::{SignedVote, VoteType};
    use xdevs_primitives::Address;

    fn key(i: u8) -> [u8; 32] {
        ed25519_consensus::SigningKey::from([i; 32])
            .verification_key()
            .to_bytes()
    }

    fn staking(i: u8, tx_ds: u64) -> StakingValidator {
        StakingValidator {
            operator: Address::repeat_byte(i),
            consensus_key: B256::from(key(i)),
            stake: U256::from(tx_ds) * U256::from(WEI_PER_POWER),
        }
    }

    #[test]
    fn staking_set_conversion() {
        let set = set_from_staking(&[staking(1, 12_500), staking(2, 10_000)]).unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.total_voting_power(), 22_500);
        assert_eq!(
            set.get(&ValidatorKey(key(1)).address())
                .unwrap()
                .voting_power,
            12_500
        );
        // Sub-tXDS stake still counts as 1; an invalid key is dropped.
        // Not every 32-byte string is an Ed25519 point: find one that is not.
        let invalid = (1u8..=255)
            .map(|i| {
                let mut b = [0u8; 32];
                b[0] = i;
                b
            })
            .find(|b| ValidatorKey::from_bytes(*b).is_err())
            .expect("an invalid Ed25519 encoding");
        let mut bad = staking(3, 5);
        bad.consensus_key = B256::from(invalid);
        let mut tiny = staking(4, 0);
        tiny.stake = U256::from(1u64);
        let set = set_from_staking(&[staking(1, 1), bad.clone(), tiny]).unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.total_voting_power(), 2);
        assert!(set_from_staking(&[bad]).is_none());
        assert!(set_from_staking(&[]).is_none());
        // Encoding round trip.
        let set = set_from_staking(&[staking(1, 7), staking(2, 9)]).unwrap();
        assert_eq!(decode_set(&encode_set(&set)).unwrap(), set);
        assert!(decode_set(&[]).is_err());
        assert!(decode_set(&[0u8; 41]).is_err());
    }

    #[test]
    fn set_lookup_follows_epoch_boundaries() {
        let db = xdevs_storage::MemDb::new();
        let cfg = ChainConfig {
            epoch_length: 10,
            staking_validators_block: Some(5),
            ..ChainConfig::new(1)
        };
        let genesis = set_from_staking(&[staking(1, 1), staking(2, 1)]).unwrap();
        let next = set_from_staking(&[staking(1, 1), staking(2, 1), staking(3, 1)]).unwrap();
        for h in [1, 5, 10] {
            assert_eq!(validator_set_at(&db, &cfg, &genesis, h).unwrap(), genesis);
        }
        // Height 11 needs block 10's set, which is not committed yet.
        assert!(validator_set_at(&db, &cfg, &genesis, 11).is_err());
        let mut batch = xdevs_storage::WriteBatch::new();
        batch.put::<tables::ValidatorSets>(&10, &Bytes::from(encode_set(&next)));
        xdevs_storage::Database::write(&db, batch).unwrap();
        for h in [11, 15, 20] {
            assert_eq!(validator_set_at(&db, &cfg, &genesis, h).unwrap(), next);
        }
        assert!(validator_set_at(&db, &cfg, &genesis, 21).is_err());
    }

    fn evidence(height: u64, seed: u8) -> ConflictingVotes {
        let sk = ed25519_consensus::SigningKey::from([seed; 32]);
        let vote = |hash: Option<B256>| {
            let mut v = SignedVote {
                vote_type: VoteType::Precommit,
                height,
                round: 0,
                block_hash: hash,
                validator: ValidatorKey(sk.verification_key().to_bytes()).address(),
                signature: [0; 64],
            };
            v.signature = sk.sign(&v.sign_bytes(1)).to_bytes();
            v
        };
        ConflictingVotes {
            first: vote(Some(B256::repeat_byte(1))),
            second: vote(None),
        }
        .canonical()
    }

    #[test]
    fn evidence_pool() {
        let pool = EvidencePool::default();
        assert!(pool.is_empty());
        assert!(pool.add(evidence(9, 1)));
        assert!(!pool.add(evidence(9, 1)), "duplicate");
        assert!(pool.add(evidence(3, 2)));
        let picked = pool.select(8, |_| true);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].first.height, 3, "oldest offence first");
        assert_eq!(pool.select(1, |e| e.first.height == 9)[0].first.height, 9);
        pool.prune(&[evidence(9, 1).hash()], 0);
        assert_eq!(pool.len(), 1);
        pool.prune(&[], 3);
        assert!(pool.is_empty());
    }
}

//! `consensus.redb` (research doc §6.7): the persisted [`SignGuard`] state and the
//! undecided-block store. A separate redb file from the chain DB, so wiping or
//! resyncing the chain never resets the double-sign guard. redb's own exclusive file
//! lock also stops two processes from sharing one consensus dir.
//!
//! Tables:
//! * `SignGuard: "last" -> height:u64be ‖ round:u32be ‖ step:u8 ‖ keccak(sign_bytes)`.
//! * `UndecidedBlocks: height:u64be ‖ hash -> rlp(block)` ([`super::codec::encode_block`]).
//! * `UndecidedProposals: height:u64be ‖ round:u32be ‖ hash -> rlp(meta)`: who streamed
//!   the block for which round and our verdict; replayed to Malachite on `StartedRound`
//!   after a crash (additive to §6.7).
//!
//! Every write commits with `Durability::Immediate` (fsync).

use super::codec::{decode_block, encode_block};
use crate::types::ValidatorAddress;
use alloy_rlp::{RlpDecodable, RlpEncodable};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use std::path::Path;
use xdevs_primitives::{Block, B256};

const SIGN_GUARD: TableDefinition<&str, &[u8]> = TableDefinition::new("SignGuard");
const UNDECIDED_BLOCKS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("UndecidedBlocks");
const UNDECIDED_PROPOSALS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("UndecidedProposals");
const LAST_KEY: &str = "last";

/// Signing step; the order is Proposal < Prevote < Precommit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub(crate) enum SignStep {
    Proposal = 0,
    Prevote = 1,
    Precommit = 2,
}

impl SignStep {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Proposal),
            1 => Some(Self::Prevote),
            2 => Some(Self::Precommit),
            _ => None,
        }
    }
}

/// The last thing this validator signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SignRecord {
    pub height: u64,
    pub round: u32,
    pub step: SignStep,
    /// keccak256 of the sign-bytes.
    pub payload: B256,
}

impl SignRecord {
    fn hrs(&self) -> (u64, u32, SignStep) {
        (self.height, self.round, self.step)
    }

    fn to_bytes(self) -> [u8; 45] {
        let mut out = [0u8; 45];
        out[..8].copy_from_slice(&self.height.to_be_bytes());
        out[8..12].copy_from_slice(&self.round.to_be_bytes());
        out[12] = self.step as u8;
        out[13..].copy_from_slice(self.payload.as_slice());
        out
    }

    fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != 45 {
            return None;
        }
        Some(Self {
            height: u64::from_be_bytes(b[..8].try_into().ok()?),
            round: u32::from_be_bytes(b[8..12].try_into().ok()?),
            step: SignStep::from_u8(b[12])?,
            payload: B256::from_slice(&b[13..]),
        })
    }
}

/// Why the SignGuard refused to sign.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SignGuardError {
    /// `(h, r, step)` is below the last signed one.
    #[error("sign guard: refusing to sign {attempted:?}, below last signed {last:?}")]
    Regression {
        last: SignRecord,
        attempted: SignRecord,
    },
    /// Same `(h, r, step)` as the last signature but a different payload: double sign.
    #[error("sign guard: DOUBLE-SIGN attempt at {attempted:?}, already signed {last:?}")]
    Conflict {
        last: SignRecord,
        attempted: SignRecord,
    },
    /// The guard state could not be read or persisted (fail closed).
    #[error("sign guard storage: {0}")]
    Storage(String),
}

/// Proposal metadata stored next to an undecided block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub(crate) struct ProposalMeta {
    /// POL round encoded as `valid_round + 1` (`0` = nil).
    pub valid_round: u64,
    /// The validator that streamed the block for this round.
    pub proposer: ValidatorAddress,
    /// [`ProposalMeta::VALID`], [`ProposalMeta::INVALID`] or [`ProposalMeta::PENDING`].
    pub validity: u8,
}

impl ProposalMeta {
    pub const INVALID: u8 = 0;
    pub const VALID: u8 = 1;
    /// Received for a future height; validated when that height starts.
    pub const PENDING: u8 = 2;
}

/// `consensus.redb`.
#[derive(Debug)]
pub(crate) struct ConsensusStore {
    db: Database,
}

fn err(e: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("consensus.redb: {e}")
}

fn block_key(height: u64, hash: B256) -> [u8; 40] {
    let mut k = [0u8; 40];
    k[..8].copy_from_slice(&height.to_be_bytes());
    k[8..].copy_from_slice(hash.as_slice());
    k
}

fn proposal_key(height: u64, round: u32, hash: B256) -> [u8; 44] {
    let mut k = [0u8; 44];
    k[..8].copy_from_slice(&height.to_be_bytes());
    k[8..12].copy_from_slice(&round.to_be_bytes());
    k[12..].copy_from_slice(hash.as_slice());
    k
}

impl ConsensusStore {
    /// Open (or create) the store at `path`. Fails if another handle holds it.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let db = Database::create(path).map_err(err)?;
        // Create the tables up front so reads never miss them.
        let mut tx = db.begin_write().map_err(err)?;
        tx.set_durability(Durability::Immediate).map_err(err)?;
        tx.open_table(SIGN_GUARD).map_err(err)?;
        tx.open_table(UNDECIDED_BLOCKS).map_err(err)?;
        tx.open_table(UNDECIDED_PROPOSALS).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Self { db })
    }

    // -- SignGuard ----------------------------------------------------------

    /// The last record persisted by the guard.
    pub fn last_signed(&self) -> anyhow::Result<Option<SignRecord>> {
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(SIGN_GUARD).map_err(err)?;
        let Some(v) = t.get(LAST_KEY).map_err(err)? else {
            return Ok(None);
        };
        SignRecord::from_bytes(v.value())
            .map(Some)
            .ok_or_else(|| err("corrupt SignGuard record"))
    }

    /// The §6.7 rule, applied before any signature leaves: refuse below `last`, allow
    /// an identical re-sign, refuse a different payload at the same `(h, r, step)`,
    /// otherwise persist `attempted` durably and allow.
    pub fn sign_guard(&self, attempted: SignRecord) -> Result<(), SignGuardError> {
        let storage = |e: anyhow::Error| SignGuardError::Storage(e.to_string());
        // One write transaction: check and record are atomic w.r.t. concurrent signers.
        let mut tx = self.db.begin_write().map_err(|e| storage(err(e)))?;
        tx.set_durability(Durability::Immediate)
            .map_err(|e| storage(err(e)))?;
        {
            let mut t = tx.open_table(SIGN_GUARD).map_err(|e| storage(err(e)))?;
            let last = match t.get(LAST_KEY).map_err(|e| storage(err(e)))? {
                Some(v) => Some(
                    SignRecord::from_bytes(v.value())
                        .ok_or_else(|| storage(err("corrupt SignGuard record")))?,
                ),
                None => None,
            };
            if let Some(last) = last {
                match attempted.hrs().cmp(&last.hrs()) {
                    std::cmp::Ordering::Less => {
                        return Err(SignGuardError::Regression { last, attempted })
                    }
                    std::cmp::Ordering::Equal if attempted.payload == last.payload => {
                        // Deterministic re-sign (WAL replay / rebroadcast): nothing to write.
                        return Ok(());
                    }
                    std::cmp::Ordering::Equal => {
                        return Err(SignGuardError::Conflict { last, attempted })
                    }
                    std::cmp::Ordering::Greater => {}
                }
            }
            t.insert(LAST_KEY, &attempted.to_bytes()[..])
                .map_err(|e| storage(err(e)))?;
        }
        tx.commit().map_err(|e| storage(err(e)))
    }

    // -- Undecided blocks ---------------------------------------------------

    /// Store a proposed or synced block under `(height, hash)`.
    pub fn put_block(&self, height: u64, hash: B256, block: &Block) -> anyhow::Result<()> {
        let mut tx = self.db.begin_write().map_err(err)?;
        tx.set_durability(Durability::Immediate).map_err(err)?;
        {
            let mut t = tx.open_table(UNDECIDED_BLOCKS).map_err(err)?;
            let key = block_key(height, hash);
            if t.get(&key[..]).map_err(err)?.is_none() {
                t.insert(&key[..], &encode_block(block)[..]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }

    /// The block stored under `(height, hash)`.
    pub fn get_block(&self, height: u64, hash: B256) -> anyhow::Result<Option<Block>> {
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(UNDECIDED_BLOCKS).map_err(err)?;
        let Some(v) = t.get(&block_key(height, hash)[..]).map_err(err)? else {
            return Ok(None);
        };
        decode_block(v.value()).map(Some).map_err(err)
    }

    /// Record (or update) the metadata of a proposal for `(height, round, hash)`.
    pub fn put_proposal(
        &self,
        height: u64,
        round: u32,
        hash: B256,
        meta: ProposalMeta,
    ) -> anyhow::Result<()> {
        let mut tx = self.db.begin_write().map_err(err)?;
        tx.set_durability(Durability::Immediate).map_err(err)?;
        {
            let mut t = tx.open_table(UNDECIDED_PROPOSALS).map_err(err)?;
            t.insert(
                &proposal_key(height, round, hash)[..],
                &alloy_rlp::encode(meta)[..],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    /// All proposals stored for `height`: `(round, hash, meta)` in key order.
    pub fn proposals_at_height(
        &self,
        height: u64,
    ) -> anyhow::Result<Vec<(u32, B256, ProposalMeta)>> {
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(UNDECIDED_PROPOSALS).map_err(err)?;
        let start = height.to_be_bytes();
        let mut out = Vec::new();
        for entry in t.range(&start[..]..).map_err(err)? {
            let (k, v) = entry.map_err(err)?;
            let k = k.value();
            if k.len() != 44 || k[..8] != start {
                break;
            }
            let round = u32::from_be_bytes(k[8..12].try_into().expect("4 bytes"));
            let meta: ProposalMeta = alloy_rlp::decode_exact(v.value()).map_err(err)?;
            out.push((round, B256::from_slice(&k[12..]), meta));
        }
        Ok(out)
    }

    /// `true` if a proposal is stored for `height` (streams for future heights are
    /// kept until their height starts; see `Host::height_params`).
    pub fn has_proposal_at(&self, height: u64) -> anyhow::Result<bool> {
        let tx = self.db.begin_read().map_err(err)?;
        let t = tx.open_table(UNDECIDED_PROPOSALS).map_err(err)?;
        let start = height.to_be_bytes();
        let next = t.range(&start[..]..).map_err(err)?.next();
        Ok(match next.transpose().map_err(err)? {
            Some((k, _)) => k.value().starts_with(&start),
            None => false,
        })
    }

    /// Drop every block and proposal at heights `<= height` (at `Finalized`).
    pub fn prune(&self, height: u64) -> anyhow::Result<()> {
        let upper = height.checked_add(1).map(u64::to_be_bytes);
        let mut tx = self.db.begin_write().map_err(err)?;
        tx.set_durability(Durability::Immediate).map_err(err)?;
        for table in [UNDECIDED_BLOCKS, UNDECIDED_PROPOSALS] {
            let mut t = tx.open_table(table).map_err(err)?;
            match &upper {
                Some(upper) => t.retain_in::<&[u8], _>(..&upper[..], |_, _| false),
                None => t.retain(|_, _| false),
            }
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::Header;

    fn rec(height: u64, round: u32, step: SignStep, p: u8) -> SignRecord {
        SignRecord {
            height,
            round,
            step,
            payload: B256::repeat_byte(p),
        }
    }

    #[test]
    fn sign_guard_rules_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consensus").join("consensus.redb");
        {
            let s = ConsensusStore::open(&path).unwrap();
            assert_eq!(s.last_signed().unwrap(), None);
            s.sign_guard(rec(5, 0, SignStep::Proposal, 1)).unwrap();
            s.sign_guard(rec(5, 0, SignStep::Prevote, 2)).unwrap();
            // Equal (h, r, s) and payload: deterministic re-sign allowed.
            s.sign_guard(rec(5, 0, SignStep::Prevote, 2)).unwrap();
            // Equal (h, r, s), different payload: double sign refused.
            assert!(matches!(
                s.sign_guard(rec(5, 0, SignStep::Prevote, 3)),
                Err(SignGuardError::Conflict { .. })
            ));
            // Lower step / round / height refused.
            assert!(matches!(
                s.sign_guard(rec(5, 0, SignStep::Proposal, 1)),
                Err(SignGuardError::Regression { .. })
            ));
            assert!(matches!(
                s.sign_guard(rec(4, 9, SignStep::Precommit, 1)),
                Err(SignGuardError::Regression { .. })
            ));
            s.sign_guard(rec(5, 1, SignStep::Proposal, 4)).unwrap();
            assert!(matches!(
                s.sign_guard(rec(5, 0, SignStep::Precommit, 9)),
                Err(SignGuardError::Regression { .. })
            ));
            s.sign_guard(rec(6, 0, SignStep::Precommit, 5)).unwrap();
            // A second open of the same file fails (exclusive lock).
            assert!(ConsensusStore::open(&path).is_err());
        }
        // Restart: the guard survives.
        let s = ConsensusStore::open(&path).unwrap();
        assert_eq!(
            s.last_signed().unwrap(),
            Some(rec(6, 0, SignStep::Precommit, 5))
        );
        s.sign_guard(rec(6, 0, SignStep::Precommit, 5)).unwrap();
        assert!(s.sign_guard(rec(6, 0, SignStep::Precommit, 6)).is_err());
        assert!(s.sign_guard(rec(6, 0, SignStep::Prevote, 5)).is_err());
        s.sign_guard(rec(7, 0, SignStep::Prevote, 1)).unwrap();
    }

    #[test]
    fn undecided_blocks_and_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let s = ConsensusStore::open(&dir.path().join("c.redb")).unwrap();
        let block = |n: u64| {
            Block::new(
                Header {
                    number: n,
                    ..Default::default()
                },
                vec![],
            )
        };
        let meta = ProposalMeta {
            valid_round: 0,
            proposer: ValidatorAddress([1; 20]),
            validity: ProposalMeta::VALID,
        };
        for h in 1..=3u64 {
            let b = block(h);
            s.put_block(h, b.hash(), &b).unwrap();
            s.put_proposal(h, 0, b.hash(), meta).unwrap();
            s.put_proposal(h, 2, b.hash(), meta).unwrap();
        }
        let b2 = block(2);
        assert_eq!(s.get_block(2, b2.hash()).unwrap(), Some(b2.clone()));
        assert_eq!(s.get_block(2, B256::ZERO).unwrap(), None);
        let at2 = s.proposals_at_height(2).unwrap();
        assert_eq!(at2.len(), 2);
        assert_eq!((at2[0].0, at2[1].0), (0, 2));
        assert_eq!(at2[0].1, b2.hash());
        assert!(s.has_proposal_at(3).unwrap());
        assert!(!s.has_proposal_at(4).unwrap());
        s.prune(2).unwrap();
        assert!(!s.has_proposal_at(2).unwrap());
        assert!(!s.has_proposal_at(0).unwrap());
        assert!(s.has_proposal_at(3).unwrap());
        assert_eq!(s.get_block(2, b2.hash()).unwrap(), None);
        assert!(s.proposals_at_height(1).unwrap().is_empty());
        assert!(s.proposals_at_height(2).unwrap().is_empty());
        assert_eq!(s.proposals_at_height(3).unwrap().len(), 2);
        s.prune(u64::MAX).unwrap();
        assert!(s.proposals_at_height(3).unwrap().is_empty());
        assert!(!s.has_proposal_at(3).unwrap());
    }
}

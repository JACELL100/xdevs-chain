//! The xdevs-side consensus traits (research doc §6.2, refines plan §7.3). **Frozen**:
//! change only additively and report it in `docs/specs/phase1-interfaces.md`.
//!
//! No Malachite types appear here, so the engine is swappable.

use crate::types::{CommitCertificate, Evidence, ValidatorSet};
use std::{sync::Arc, time::Instant};
use xdevs_primitives::{Block, B256};

/// The application's verdict on a full proposed block (ADR-0003 execute-before-vote).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Validity {
    /// The block re-executed correctly on the committed head: prevote for it.
    Valid,
    /// The block is invalid: prevote nil.
    Invalid,
}

impl Validity {
    /// `Valid` iff `ok`.
    pub const fn from_bool(ok: bool) -> Self {
        if ok {
            Self::Valid
        } else {
            Self::Invalid
        }
    }

    /// `true` for [`Validity::Valid`].
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Where a block handed to [`ConsensusApp::validate`] comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockOrigin {
    /// A live proposal for the current height (the timeliness check applies).
    Consensus,
    /// An already-decided block received through block sync (ValueSync), or a
    /// re-proposal of a value that already has a polka (`pol_round` set): more than 2/3
    /// already prevoted it, so the subjective timeliness (clock) check must not apply.
    /// Every other rule does.
    Sync,
}

/// What the consensus engine needs from the application (the node's `ChainApp`).
#[async_trait::async_trait]
pub trait ConsensusApp: Send + Sync + 'static {
    /// Last committed (height, block hash); the engine starts at height + 1.
    async fn head(&self) -> anyhow::Result<(u64, B256)>;

    /// Validator set active at `height` (Phase 2: the genesis set for every height).
    async fn validator_set(&self, height: u64) -> anyhow::Result<ValidatorSet>;

    /// Build AND execute a block on the committed head; must finish before `deadline`.
    /// The app caches the execution outcome under the block hash.
    async fn propose(&self, height: u64, round: u32, deadline: Instant) -> anyhow::Result<Block>;

    /// Re-execute a proposal on the committed head (ADR-0003). `Err` = local failure
    /// (no verdict).
    async fn validate(
        &self,
        height: u64,
        block: &Block,
        origin: BlockOrigin,
    ) -> anyhow::Result<Validity>;

    /// Persist a decided block + certificate atomically. Idempotent: called again for the
    /// same height with the same hash it only replaces the certificate if the new one has
    /// more signatures.
    async fn commit(&self, block: Block, cert: CommitCertificate) -> anyhow::Result<()>;

    /// Serve block sync: the decided block at `height` and its certificate.
    async fn decided(&self, height: u64) -> anyhow::Result<Option<(Block, CommitCertificate)>>;

    /// Lowest height [`ConsensusApp::decided`] can serve (1 while we keep all blocks).
    async fn earliest_height(&self) -> anyhow::Result<u64>;

    /// Misbehaviour seen by the engine (logged in Phase 2, slashed in Phase 3).
    async fn report_evidence(&self, _e: Evidence) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A BFT engine driving a [`ConsensusApp`] until `shutdown` turns `true`.
#[async_trait::async_trait]
pub trait ConsensusEngine: Send {
    /// Run consensus. Returns `Ok(())` after a requested shutdown, `Err` on a fatal error
    /// (the node must then stop: the chain halts rather than forks).
    async fn run(
        self,
        app: Arc<dyn ConsensusApp>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validity_helpers() {
        assert_eq!(Validity::from_bool(true), Validity::Valid);
        assert_eq!(Validity::from_bool(false), Validity::Invalid);
        assert!(Validity::Valid.is_valid());
        assert!(!Validity::Invalid.is_valid());
    }

    /// The traits must stay object safe (`Arc<dyn ConsensusApp>`).
    #[test]
    fn app_is_object_safe() {
        fn _takes(_: Arc<dyn ConsensusApp>) {}
    }
}

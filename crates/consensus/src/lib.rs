//! BFT consensus for xDEVS Chain (plan §11, ADR-0002, `docs/research/phase2-consensus.md` §6).
//!
//! * [`app`] — the xdevs-side traits: [`ConsensusApp`] (implemented by the node's
//!   `ChainApp`) and [`ConsensusEngine`] (implemented by [`MalachiteEngine`]). No Malachite
//!   types appear in this module.
//! * [`types`] — validator sets, commit certificates, evidence and the canonical
//!   sign-bytes (§6.4–§6.6).
//! * [`config`] — `[consensus]` / `[p2p]` configuration and the engine config (§6.10).
//! * [`MalachiteEngine`] — the engine adapter. **All** Malachite types stay in the
//!   private `malachite` module; nothing outside this crate depends on Malachite.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod app;
pub mod config;
mod engine;
mod malachite;
pub mod types;

pub use app::{BlockOrigin, ConsensusApp, ConsensusEngine, Validity};
pub use config::{ConsensusTimeouts, EngineConfig, P2pConfig};
pub use engine::{EngineKeys, MalachiteEngine};
pub use types::{
    parts_sign_bytes, proposal_sign_bytes, vote_sign_bytes, CertificateError, CommitCertificate,
    CommitSig, ConflictingVotes, Evidence, SignedVote, Validator, ValidatorAddress, ValidatorKey,
    ValidatorSet, ValidatorSetError, VoteType,
};

//! The Malachite `Context` for xDEVS Chain and its types (research doc §6.4).
//!
//! * `Height` is a `u64` newtype; `Address` is [`ValidatorAddress`].
//! * `Value = BlockValue { hash }`, `ValueId = B256` (the Ethereum block hash).
//! * The validator set wraps the xdevs [`ValidatorSet`] so proposer selection is the
//!   §6.6 rule ([`ValidatorSet::select_proposer`]).
//! * Vote extensions are unused in Phase 2 (`Extension = ()`, policy `Disabled`).

use super::{Ed25519, LinearTimeouts, PublicKey};
use crate::types::{ValidatorAddress, ValidatorSet};
use arc_malachitebft_app_channel::app::types::core::{
    self as mc, NilOrVal, Round, SignedExtension, VoteType as MVoteType,
};
use std::fmt;
use xdevs_primitives::B256;

// ---------------------------------------------------------------------------
// Height, address, value
// ---------------------------------------------------------------------------

/// Block height (Malachite needs a local type to implement its `Height` trait).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Height(pub u64);

impl fmt::Display for Height {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl mc::Height for Height {
    const ZERO: Self = Self(0);
    const INITIAL: Self = Self(1);

    fn increment_by(&self, n: u64) -> Self {
        Self(self.0.saturating_add(n))
    }

    fn decrement_by(&self, n: u64) -> Option<Self> {
        self.0.checked_sub(n).map(Self)
    }

    fn as_u64(&self) -> u64 {
        self.0
    }
}

impl mc::Address for ValidatorAddress {}

/// The consensus value: an Ethereum block, identified by its hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BlockValue {
    pub hash: B256,
}

impl BlockValue {
    pub const fn new(hash: B256) -> Self {
        Self { hash }
    }
}

impl mc::Value for BlockValue {
    type Id = B256;

    fn id(&self) -> B256 {
        self.hash
    }
}

// ---------------------------------------------------------------------------
// Proposal and vote
// ---------------------------------------------------------------------------

/// A signed-over proposal for `(height, round)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Proposal {
    pub height: Height,
    pub round: Round,
    pub value: BlockValue,
    pub pol_round: Round,
    pub validator_address: ValidatorAddress,
}

impl mc::Proposal<XdevsContext> for Proposal {
    fn height(&self) -> Height {
        self.height
    }

    fn round(&self) -> Round {
        self.round
    }

    fn value(&self) -> &BlockValue {
        &self.value
    }

    fn take_value(self) -> BlockValue {
        self.value
    }

    fn pol_round(&self) -> Round {
        self.pol_round
    }

    fn validator_address(&self) -> &ValidatorAddress {
        &self.validator_address
    }
}

/// A prevote or precommit. Vote extensions are not supported (Phase 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Vote {
    pub vote_type: MVoteType,
    pub height: Height,
    pub round: Round,
    pub value: NilOrVal<B256>,
    pub validator_address: ValidatorAddress,
}

impl mc::Vote<XdevsContext> for Vote {
    fn height(&self) -> Height {
        self.height
    }

    fn round(&self) -> Round {
        self.round
    }

    fn value(&self) -> &NilOrVal<B256> {
        &self.value
    }

    fn take_value(self) -> NilOrVal<B256> {
        self.value
    }

    fn vote_type(&self) -> MVoteType {
        self.vote_type
    }

    fn validator_address(&self) -> &ValidatorAddress {
        &self.validator_address
    }

    fn extension(&self) -> Option<&SignedExtension<XdevsContext>> {
        None
    }

    fn take_extension(&mut self) -> Option<SignedExtension<XdevsContext>> {
        None
    }

    fn extend(self, _extension: SignedExtension<XdevsContext>) -> Self {
        // Extensions are disabled (VoteExtensionPolicy::Disabled); never attached.
        self
    }
}

// ---------------------------------------------------------------------------
// Proposal parts (§6.4)
// ---------------------------------------------------------------------------

/// One part of a streamed proposal: `Init`, then `Data` chunks of the RLP block, then
/// `Fin` with the streamer's signature over [`crate::types::parts_sign_bytes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProposalPart {
    Init {
        height: Height,
        round: Round,
        pol_round: Round,
        /// The validator that streams (and signs `Fin`).
        proposer: ValidatorAddress,
    },
    Data(Vec<u8>),
    Fin {
        signature: [u8; 64],
    },
}

impl ProposalPart {
    /// Bytes this part costs in reassembly buffers.
    pub fn size_hint(&self) -> usize {
        match self {
            Self::Data(chunk) => chunk.len(),
            Self::Init { .. } | Self::Fin { .. } => 64,
        }
    }
}

impl mc::ProposalPart<XdevsContext> for ProposalPart {
    fn is_first(&self) -> bool {
        matches!(self, Self::Init { .. })
    }

    fn is_last(&self) -> bool {
        matches!(self, Self::Fin { .. })
    }
}

// ---------------------------------------------------------------------------
// Validators
// ---------------------------------------------------------------------------

/// A validator in Malachite form (its public key as Malachite's `PublicKey`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MValidator {
    pub address: ValidatorAddress,
    pub public_key: PublicKey,
    pub voting_power: u64,
}

impl mc::Validator<XdevsContext> for MValidator {
    fn address(&self) -> &ValidatorAddress {
        &self.address
    }

    fn public_key(&self) -> &PublicKey {
        &self.public_key
    }

    fn voting_power(&self) -> u64 {
        self.voting_power
    }
}

/// The xdevs [`ValidatorSet`] plus its Malachite view (same order: by address).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MValidatorSet {
    inner: ValidatorSet,
    validators: Vec<MValidator>,
}

impl MValidatorSet {
    /// Convert; keys were already checked by [`ValidatorSet::new`].
    pub fn new(inner: ValidatorSet) -> anyhow::Result<Self> {
        let validators = inner
            .validators()
            .iter()
            .map(|v| {
                Ok(MValidator {
                    address: v.address,
                    public_key: PublicKey::from_bytes(v.public_key.0)
                        .map_err(|e| anyhow::anyhow!("validator {}: {e}", v.address))?,
                    voting_power: v.voting_power,
                })
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self { inner, validators })
    }

    /// The xdevs set.
    pub fn inner(&self) -> &ValidatorSet {
        &self.inner
    }

    fn index_of(&self, address: &ValidatorAddress) -> Option<usize> {
        self.validators
            .binary_search_by_key(address, |v| v.address)
            .ok()
    }
}

impl mc::ValidatorSet<XdevsContext> for MValidatorSet {
    fn count(&self) -> usize {
        self.validators.len()
    }

    fn total_voting_power(&self) -> u64 {
        self.inner.total_voting_power()
    }

    fn get_by_address(&self, address: &ValidatorAddress) -> Option<&MValidator> {
        self.index_of(address).map(|i| &self.validators[i])
    }

    fn get_by_index(&self, index: usize) -> Option<&MValidator> {
        self.validators.get(index)
    }
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// The Malachite context of an xDEVS chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct XdevsContext {
    pub chain_id: u64,
}

impl XdevsContext {
    pub const fn new(chain_id: u64) -> Self {
        Self { chain_id }
    }
}

/// Round as `u32` (Malachite never asks to sign or select for `Round::Nil`).
pub(crate) fn round_u32(round: Round) -> u32 {
    round.as_u32().unwrap_or(0)
}

impl mc::Context for XdevsContext {
    type Address = ValidatorAddress;
    type Height = Height;
    type ProposalPart = ProposalPart;
    type Proposal = Proposal;
    type Validator = MValidator;
    type ValidatorSet = MValidatorSet;
    type Timeouts = LinearTimeouts;
    type Value = BlockValue;
    type Vote = Vote;
    type Extension = ();
    type SigningScheme = Ed25519;

    fn select_proposer<'a>(
        &self,
        validator_set: &'a MValidatorSet,
        height: Height,
        round: Round,
    ) -> &'a MValidator {
        let chosen = validator_set
            .inner
            .select_proposer(height.0, round_u32(round))
            .address;
        let i = validator_set
            .index_of(&chosen)
            .expect("both views hold the same validators");
        &validator_set.validators[i]
    }

    fn new_proposal(
        &self,
        height: Height,
        round: Round,
        value: BlockValue,
        pol_round: Round,
        address: ValidatorAddress,
    ) -> Proposal {
        Proposal {
            height,
            round,
            value,
            pol_round,
            validator_address: address,
        }
    }

    fn new_prevote(
        &self,
        height: Height,
        round: Round,
        value_id: NilOrVal<B256>,
        address: ValidatorAddress,
    ) -> Vote {
        Vote {
            vote_type: MVoteType::Prevote,
            height,
            round,
            value: value_id,
            validator_address: address,
        }
    }

    fn new_precommit(
        &self,
        height: Height,
        round: Round,
        value_id: NilOrVal<B256>,
        address: ValidatorAddress,
    ) -> Vote {
        Vote {
            vote_type: MVoteType::Precommit,
            height,
            round,
            value: value_id,
            validator_address: address,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::types::{Validator, ValidatorKey};
    use mc::{Context as _, ValidatorSet as _};

    pub(crate) fn test_set(n: u8) -> (Vec<super::super::PrivateKey>, MValidatorSet) {
        let keys: Vec<_> = (1..=n)
            .map(|i| super::super::private_key(&[i; 32]))
            .collect();
        let vals = keys
            .iter()
            .map(|k| Validator::new(ValidatorKey(*k.public_key().as_bytes()), 1))
            .collect();
        (
            keys,
            MValidatorSet::new(ValidatorSet::new(vals).unwrap()).unwrap(),
        )
    }

    #[test]
    fn proposer_matches_xdevs_rule() {
        let (_, set) = test_set(4);
        let ctx = XdevsContext::new(1);
        for h in 1..10u64 {
            for r in 0..3u32 {
                let p = ctx.select_proposer(&set, Height(h), Round::new(r));
                assert_eq!(p.address, set.inner().select_proposer(h, r).address);
                assert_eq!(set.get_by_address(&p.address), Some(p));
            }
        }
        assert_eq!(set.count(), 4);
        assert_eq!(set.total_voting_power(), 4);
    }
}

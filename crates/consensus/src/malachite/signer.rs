//! Ed25519 `Signer` / `Verifier` over the xdevs sign-bytes (research doc §6.4), with the
//! persisted [`SignGuard`](super::store::ConsensusStore::sign_guard) checked before any
//! vote or proposal signature is returned (§6.7). `sign_validator_proof` is exempt.

use super::context::{round_u32, Proposal, Vote, XdevsContext};
use super::store::{ConsensusStore, SignRecord, SignStep};
use super::{PrivateKey, PublicKey, Signature};
use crate::types::{proposal_sign_bytes, vote_sign_bytes, VoteType};
use arc_malachitebft_app_channel::app::types::core::{
    NilOrVal, SignedMessage, ValidatorProof, VoteExtensionScope, VoteType as MVoteType,
};
use arc_malachitebft_app_channel::{Signer, Verifier};
use arc_malachitebft_signing::{Error as SignError, VerificationResult};
use std::sync::Arc;
use xdevs_primitives::keccak256;

/// Vote sign-bytes of a Malachite vote.
pub(crate) fn vote_bytes(chain_id: u64, v: &Vote) -> Vec<u8> {
    let vt = match v.vote_type {
        MVoteType::Prevote => VoteType::Prevote,
        MVoteType::Precommit => VoteType::Precommit,
    };
    let hash = match v.value {
        NilOrVal::Nil => None,
        NilOrVal::Val(h) => Some(h),
    };
    vote_sign_bytes(chain_id, vt, v.height.0, round_u32(v.round), hash)
}

/// Proposal sign-bytes of a Malachite proposal.
pub(crate) fn proposal_bytes(chain_id: u64, p: &Proposal) -> Vec<u8> {
    proposal_sign_bytes(
        chain_id,
        p.height.0,
        round_u32(p.round),
        p.pol_round.as_u32(),
        p.value.hash,
    )
}

/// The validator's signer: Ed25519 key + SignGuard.
pub(crate) struct XdevsSigner {
    chain_id: u64,
    key: PrivateKey,
    store: Arc<ConsensusStore>,
}

impl XdevsSigner {
    pub fn new(chain_id: u64, key: PrivateKey, store: Arc<ConsensusStore>) -> Self {
        Self {
            chain_id,
            key,
            store,
        }
    }

    /// Check and record with the guard, then sign.
    fn guarded_sign(
        &self,
        height: u64,
        round: u32,
        step: SignStep,
        bytes: &[u8],
    ) -> Result<Signature, SignError> {
        let record = SignRecord {
            height,
            round,
            step,
            payload: keccak256(bytes),
        };
        if let Err(e) = self.store.sign_guard(record) {
            tracing::error!(target: "xdevs::consensus", "{e}");
            return Err(SignError::from_source(e));
        }
        Ok(self.key.sign(bytes))
    }
}

#[async_trait::async_trait]
impl Signer<XdevsContext> for XdevsSigner {
    async fn sign_vote(&self, vote: Vote) -> Result<SignedMessage<XdevsContext, Vote>, SignError> {
        let step = match vote.vote_type {
            MVoteType::Prevote => SignStep::Prevote,
            MVoteType::Precommit => SignStep::Precommit,
        };
        let bytes = vote_bytes(self.chain_id, &vote);
        let sig = self.guarded_sign(vote.height.0, round_u32(vote.round), step, &bytes)?;
        Ok(SignedMessage::new(vote, sig))
    }

    async fn sign_proposal(
        &self,
        proposal: Proposal,
    ) -> Result<SignedMessage<XdevsContext, Proposal>, SignError> {
        let bytes = proposal_bytes(self.chain_id, &proposal);
        let sig = self.guarded_sign(
            proposal.height.0,
            round_u32(proposal.round),
            SignStep::Proposal,
            &bytes,
        )?;
        Ok(SignedMessage::new(proposal, sig))
    }

    async fn sign_vote_extension(
        &self,
        _scope: VoteExtensionScope<XdevsContext>,
        _extension: (),
    ) -> Result<SignedMessage<XdevsContext, ()>, SignError> {
        Err(SignError::from_source(
            "vote extensions are disabled in Phase 2",
        ))
    }

    async fn sign_validator_proof(
        &self,
        public_key: Vec<u8>,
        peer_id: Vec<u8>,
    ) -> Result<ValidatorProof<XdevsContext>, SignError> {
        // Exempt from the guard: it binds the consensus key to the libp2p identity.
        let sig = self
            .key
            .sign(&ValidatorProof::<XdevsContext>::signing_bytes(
                &public_key,
                &peer_id,
            ));
        Ok(ValidatorProof::new(public_key, peer_id, sig))
    }
}

/// Signature verification for votes, proposals and validator proofs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct XdevsVerifier {
    chain_id: u64,
}

impl XdevsVerifier {
    pub fn new(chain_id: u64) -> Self {
        Self { chain_id }
    }
}

#[async_trait::async_trait]
impl Verifier<XdevsContext> for XdevsVerifier {
    async fn verify_signed_vote(
        &self,
        vote: &Vote,
        signature: &Signature,
        public_key: &PublicKey,
    ) -> Result<VerificationResult, SignError> {
        let ok = public_key
            .verify(&vote_bytes(self.chain_id, vote), signature)
            .is_ok();
        Ok(VerificationResult::from_bool(ok))
    }

    async fn verify_signed_proposal(
        &self,
        proposal: &Proposal,
        signature: &Signature,
        public_key: &PublicKey,
    ) -> Result<VerificationResult, SignError> {
        let ok = public_key
            .verify(&proposal_bytes(self.chain_id, proposal), signature)
            .is_ok();
        Ok(VerificationResult::from_bool(ok))
    }

    async fn verify_signed_vote_extension(
        &self,
        _scope: &VoteExtensionScope<XdevsContext>,
        _extension: &(),
        _signature: &Signature,
        _public_key: &PublicKey,
    ) -> Result<VerificationResult, SignError> {
        Ok(VerificationResult::Invalid)
    }

    async fn verify_validator_proof(
        &self,
        proof: &ValidatorProof<XdevsContext>,
    ) -> Result<VerificationResult, SignError> {
        let Ok(bytes) = <[u8; 32]>::try_from(&proof.public_key[..]) else {
            return Ok(VerificationResult::Invalid);
        };
        let Ok(pk) = PublicKey::from_bytes(bytes) else {
            return Ok(VerificationResult::Invalid);
        };
        Ok(VerificationResult::from_bool(
            pk.verify(&proof.preimage(), &proof.signature).is_ok(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::malachite::context::{BlockValue, Height};
    use crate::malachite::private_key;
    use crate::types::{ValidatorAddress, ValidatorKey};
    use arc_malachitebft_app_channel::app::types::core::{Context as _, Round};
    use xdevs_primitives::B256;

    const CHAIN: u64 = 7117;

    #[tokio::test]
    async fn guarded_signer_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consensus.redb");
        let key = private_key(&[3; 32]);
        let pk = key.public_key();
        let addr = ValidatorKey(*pk.as_bytes()).address();
        let ctx = XdevsContext::new(CHAIN);
        let verifier = XdevsVerifier::new(CHAIN);
        let a = NilOrVal::Val(B256::repeat_byte(1));
        let b = NilOrVal::Val(B256::repeat_byte(2));
        {
            let store = Arc::new(ConsensusStore::open(&path).unwrap());
            let signer = XdevsSigner::new(CHAIN, key.clone(), store);

            let p = ctx.new_proposal(
                Height(3),
                Round::new(0),
                BlockValue::new(B256::repeat_byte(1)),
                Round::Nil,
                addr,
            );
            let sp = signer.sign_proposal(p.clone()).await.unwrap();
            assert!(verifier
                .verify_signed_proposal(&p, &sp.signature, &pk)
                .await
                .unwrap()
                .is_valid());
            // The signature is over the xdevs proposal sign-bytes.
            assert!(ValidatorKey(*pk.as_bytes()).verify(
                &proposal_sign_bytes(CHAIN, 3, 0, None, B256::repeat_byte(1)),
                &sp.signature.to_bytes()
            ));

            let v = ctx.new_prevote(Height(3), Round::new(0), a, addr);
            let s1 = signer.sign_vote(v.clone()).await.unwrap();
            // Re-sign (same payload) is allowed and deterministic.
            let s2 = signer.sign_vote(v.clone()).await.unwrap();
            assert_eq!(s1.signature, s2.signature);
            assert!(verifier
                .verify_signed_vote(&v, &s1.signature, &pk)
                .await
                .unwrap()
                .is_valid());
            // Conflicting prevote at the same (h, r): refused.
            let conflicting = ctx.new_prevote(Height(3), Round::new(0), b, addr);
            assert!(signer.sign_vote(conflicting).await.is_err());
            // Lower: a proposal for the same round after prevoting.
            let lower = ctx.new_proposal(
                Height(3),
                Round::new(0),
                BlockValue::new(B256::repeat_byte(1)),
                Round::Nil,
                addr,
            );
            assert!(signer.sign_proposal(lower).await.is_err());
            signer
                .sign_vote(ctx.new_precommit(Height(3), Round::new(0), a, addr))
                .await
                .unwrap();
        }
        // After a restart: the identical precommit may be re-signed, a different one not.
        let store = Arc::new(ConsensusStore::open(&path).unwrap());
        let signer = XdevsSigner::new(CHAIN, key, store);
        signer
            .sign_vote(ctx.new_precommit(Height(3), Round::new(0), a, addr))
            .await
            .unwrap();
        assert!(signer
            .sign_vote(ctx.new_precommit(Height(3), Round::new(0), NilOrVal::Nil, addr))
            .await
            .is_err());
        assert!(signer
            .sign_vote(ctx.new_prevote(Height(3), Round::new(0), a, addr))
            .await
            .is_err());
        signer
            .sign_vote(ctx.new_prevote(Height(4), Round::new(0), NilOrVal::Nil, addr))
            .await
            .unwrap();

        // Validator proofs are exempt and verify.
        let proof = signer
            .sign_validator_proof(pk.as_bytes().to_vec(), vec![1, 2, 3])
            .await
            .unwrap();
        assert!(verifier
            .verify_validator_proof(&proof)
            .await
            .unwrap()
            .is_valid());
        let mut forged = proof;
        forged.peer_id = vec![9];
        assert!(!verifier
            .verify_validator_proof(&forged)
            .await
            .unwrap()
            .is_valid());
        let _ = ValidatorAddress::default();
    }
}

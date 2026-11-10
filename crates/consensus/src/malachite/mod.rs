//! Malachite adapter (WP1, research doc §6.3–§6.8). **All** Malachite types live in this
//! module and never appear in the crate's public API.
//!
//! Day-0 stub: it imports the pinned `arc-malachitebft-*` 0.8.2 crates (so the whole
//! dependency graph, including Malachite's libp2p 0.56 network, is compiled and checked
//! on every platform) and holds the pure config mappings. WP1 adds here:
//! `context` (Malachite `Context` + types), `codec` (`XdevsCodec` for network / WAL /
//! sync), `signer` (Ed25519 `Signer`/`Verifier` + `SignGuard`), `streaming` (proposal
//! parts), `store` (`UndecidedBlocks`), and `engine` (the `AppMsg` loop).

// Re-exports used by the WP1 modules as they land.
#[allow(unused_imports)]
pub(crate) use arc_malachitebft_app_channel::{
    app::types::core::{HeightParams, LinearTimeouts, Round, Timeout, VoteExtensionPolicy},
    start_engine, AppMsg, Channels, NetworkMsg,
};
#[allow(unused_imports)]
pub(crate) use arc_malachitebft_signing_ed25519::{Ed25519, PrivateKey, PublicKey, Signature};

use crate::config::ConsensusTimeouts;
use std::time::Duration;

/// `[consensus]` → Malachite `LinearTimeouts` (§7). `rebroadcast` follows Malachite's
/// default rule `propose + prevote + precommit`.
#[allow(dead_code)] // used by the WP1 engine loop (HeightParams)
pub(crate) fn linear_timeouts(t: &ConsensusTimeouts) -> LinearTimeouts {
    let ms = Duration::from_millis;
    let (propose, prevote, precommit) = (
        ms(t.timeout_propose_ms),
        ms(t.timeout_prevote_ms),
        ms(t.timeout_precommit_ms),
    );
    LinearTimeouts {
        propose,
        propose_delta: ms(t.timeout_propose_delta_ms),
        prevote,
        prevote_delta: ms(t.timeout_prevote_delta_ms),
        precommit,
        precommit_delta: ms(t.timeout_precommit_delta_ms),
        rebroadcast: propose + prevote + precommit,
        max_timeout: ms(t.max_timeout_ms),
    }
}

/// Malachite private key from a 32-byte Ed25519 seed.
pub(crate) fn private_key(seed: &[u8; 32]) -> PrivateKey {
    PrivateKey::from(*seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ValidatorKey, VoteType};

    #[test]
    fn timeouts_map_and_grow_linearly() {
        let lt = linear_timeouts(&ConsensusTimeouts::default());
        assert_eq!(
            lt.duration_for(Timeout::propose(Round::new(0))),
            Duration::from_millis(1000)
        );
        assert_eq!(
            lt.duration_for(Timeout::propose(Round::new(2))),
            Duration::from_millis(2000)
        );
        assert_eq!(
            lt.duration_for(Timeout::prevote(Round::new(1))),
            Duration::from_millis(750)
        );
        assert_eq!(
            lt.duration_for(Timeout::precommit(Round::new(4))),
            Duration::from_millis(1500)
        );
        // Capped by max_timeout.
        assert_eq!(
            lt.duration_for(Timeout::propose(Round::new(1000))),
            Duration::from_secs(10)
        );
        assert_eq!(lt.rebroadcast, Duration::from_millis(2000));
    }

    #[test]
    fn malachite_signatures_verify_as_xdevs_signatures() {
        let sk = private_key(&[5u8; 32]);
        let key = ValidatorKey(*sk.public_key().as_bytes());
        let msg = crate::types::vote_sign_bytes(1, VoteType::Prevote, 1, 0, None);
        let sig = sk.sign(&msg);
        assert!(key.verify(&msg, &sig.to_bytes()));
        assert!(!key.verify(b"other", &sig.to_bytes()));
    }
}

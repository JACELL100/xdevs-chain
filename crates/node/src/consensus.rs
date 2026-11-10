//! Validator / full-node wiring helpers (research doc §6.9–§6.11): the consensus
//! validator set from genesis, key files, the engine config and the tx-gossip glue.

use crate::config::{NodeConfig, ValidatorSection};
use anyhow::{anyhow, Context};
use std::{path::Path, sync::Arc};
use xdevs_consensus::{EngineConfig, EngineKeys, Validator, ValidatorKey, ValidatorSet};
use xdevs_crypto::{decode_and_recover, ConsensusKeystore, Ed25519Seed, NodeKey};
use xdevs_genesis::Genesis;
use xdevs_mempool::{Pool, PoolError, TxOrigin, TxPool};
use xdevs_network::{TxValidator, Verdict};
use zeroize::Zeroizing;

/// Environment variable holding the consensus keystore password (when
/// `[validator] password_file` is not set).
pub const KEY_PASSWORD_ENV: &str = "XDEVS_KEY_PASSWORD";

/// The static consensus validator set of `genesis` (`None` = no validators: dev chain).
pub fn genesis_validator_set(genesis: &Genesis) -> anyhow::Result<Option<ValidatorSet>> {
    if genesis.validators.is_empty() {
        return Ok(None);
    }
    let validators = genesis
        .validator_set()?
        .iter()
        .map(|v| {
            ValidatorKey::from_bytes(v.consensus_key)
                .map(|key| Validator::new(key, v.power))
                .map_err(|e| anyhow!("genesis validator {:?}: {e}", v.name))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(
        ValidatorSet::new(validators).map_err(|e| anyhow!("genesis validator set: {e}"))?,
    ))
}

/// Read the consensus key: a keystore v3 JSON (`ConsensusKeystore`, password from
/// `password_file`, else `$XDEVS_KEY_PASSWORD`, else empty) or the 32-byte seed as hex.
pub fn load_consensus_key(section: &ValidatorSection) -> anyhow::Result<Ed25519Seed> {
    let path = section
        .key_file
        .as_ref()
        .context("[validator] enabled = true but key_file is not set")?;
    let text = Zeroizing::new(
        std::fs::read_to_string(path)
            .with_context(|| format!("read consensus key {}", path.display()))?,
    );
    if text.trim_start().starts_with('{') {
        let keystore = ConsensusKeystore::from_json_str(&text)
            .with_context(|| format!("consensus key {}", path.display()))?;
        let password = Zeroizing::new(match &section.password_file {
            Some(p) => std::fs::read_to_string(p)
                .with_context(|| format!("read password file {}", p.display()))?
                .trim_end_matches(['\r', '\n'])
                .to_string(),
            None => std::env::var(KEY_PASSWORD_ENV).unwrap_or_default(),
        });
        keystore
            .decrypt(password.as_bytes())
            .with_context(|| format!("decrypt consensus key {}", path.display()))
    } else {
        Ed25519Seed::from_hex(text.trim())
            .with_context(|| format!("consensus key {}", path.display()))
    }
}

/// The libp2p identity seed: `[p2p] node_key_file` (created if missing), else an
/// ephemeral identity.
pub fn load_node_key(path: Option<&Path>) -> anyhow::Result<Ed25519Seed> {
    Ok(match path {
        Some(p) => NodeKey::load_or_generate(p)
            .with_context(|| format!("node key {}", p.display()))?
            .seed()
            .clone(),
        None => Ed25519Seed::generate(),
    })
}

/// Engine config and keys for a validator (`consensus_key = Some`) or a full node.
pub fn engine_setup(
    config: &NodeConfig,
    genesis: &Genesis,
    consensus_key: Option<&Ed25519Seed>,
    node_key: &Ed25519Seed,
) -> (EngineConfig, EngineKeys) {
    let mut engine = EngineConfig::new(genesis.config.chain_id, &config.chain.data_dir);
    engine.timeouts = config.consensus.clone();
    engine.p2p = config.p2p.clone();
    // Name the node after its genesis validator entry, if any.
    let own = consensus_key.map(|k| k.public_key());
    if let Some(name) = genesis
        .validators
        .iter()
        .find(|v| Some(v.consensus_key) == own)
        .and_then(|v| v.name.clone())
    {
        engine.moniker = name;
    }
    let keys = match consensus_key {
        Some(k) => EngineKeys::validator(*k.as_bytes(), *node_key.as_bytes()),
        None => EngineKeys::full_node(*node_key.as_bytes()),
    };
    (engine, keys)
}

/// Gossip verdict of a pool admission result (research doc §6.11): `Accept` when added
/// or already known; `Reject` (penalise, never forward) for txs no node would ever
/// accept; `Ignore` for txs that are only unacceptable *here and now* (nonce, funds,
/// fee, pool limits).
pub fn gossip_verdict(result: &Result<(), PoolError>) -> Verdict {
    match result {
        Ok(()) | Err(PoolError::AlreadyKnown) => Verdict::Accept,
        Err(
            PoolError::Oversized { .. }
            | PoolError::UnsupportedType(_)
            | PoolError::EmptyAuthorizationList
            | PoolError::InvalidSignature(_)
            | PoolError::WrongChainId { .. }
            | PoolError::IntrinsicGasTooLow { .. }
            | PoolError::GasLimitExceedsBlock
            | PoolError::GasLimitTooHigh { .. }
            | PoolError::TipAboveFeeCap { .. }
            | PoolError::InitCodeTooLarge { .. },
        ) => Verdict::Reject,
        Err(_) => Verdict::Ignore,
    }
}

/// The tx-gossip validator: decode, recover, admit to the pool (inbound gossip →
/// validated pool admission). Runs on the network's validation thread, after the
/// per-peer rate limit; the pool rejects unfunded senders and bad nonces from a state
/// read before taking its write lock.
pub fn gossip_validator(pool: Arc<Pool>, chain_id: u64) -> TxValidator {
    Arc::new(move |raw: &[u8]| {
        let tx = match decode_and_recover(raw, chain_id) {
            Ok(tx) => tx,
            Err(e) => {
                tracing::debug!(target: "xdevs::gossip", error = %e, "rejecting undecodable tx");
                return Verdict::Reject;
            }
        };
        let hash = tx.hash();
        let result = pool.add(tx, TxOrigin::External).map(|_| ());
        let verdict = gossip_verdict(&result);
        if let Err(e) = &result {
            tracing::debug!(target: "xdevs::gossip", %hash, error = %e, ?verdict, "gossiped tx not added");
        }
        verdict
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_crypto::ScryptParams;

    #[test]
    fn consensus_key_from_hex_and_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let seed = Ed25519Seed::from_bytes(&[9u8; 32]);

        let hex_path = dir.path().join("key.hex");
        std::fs::write(
            &hex_path,
            format!("0x{}\n", xdevs_primitives::hex::encode([9u8; 32])),
        )
        .unwrap();
        let section = ValidatorSection {
            enabled: true,
            key_file: Some(hex_path),
            ..Default::default()
        };
        assert_eq!(load_consensus_key(&section).unwrap(), seed);

        let ks_path = dir.path().join("key.json");
        let ks = ConsensusKeystore::encrypt(&seed, "pw", ScryptParams::LIGHT).unwrap();
        std::fs::write(&ks_path, ks.to_json_string()).unwrap();
        let pw_path = dir.path().join("pw");
        std::fs::write(&pw_path, "pw\n").unwrap();
        let mut section = ValidatorSection {
            enabled: true,
            key_file: Some(ks_path),
            password_file: Some(pw_path.clone()),
            ..Default::default()
        };
        assert_eq!(load_consensus_key(&section).unwrap(), seed);
        std::fs::write(&pw_path, "wrong").unwrap();
        assert!(load_consensus_key(&section).is_err());
        section.key_file = None;
        assert!(load_consensus_key(&section).is_err());
    }

    #[test]
    fn gossip_verdicts() {
        assert_eq!(gossip_verdict(&Ok(())), Verdict::Accept);
        assert_eq!(
            gossip_verdict(&Err(PoolError::AlreadyKnown)),
            Verdict::Accept
        );
        assert_eq!(
            gossip_verdict(&Err(PoolError::UnsupportedType(3))),
            Verdict::Reject
        );
        assert_eq!(gossip_verdict(&Err(PoolError::PoolFull)), Verdict::Ignore);
    }
}

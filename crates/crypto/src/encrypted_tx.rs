//! Encrypted-mempool envelopes (Phase 5 prototype, `docs/research/encrypted-mempool.md`).
//!
//! An **envelope** is an ordinary signed transaction from any account to
//! [`ENCRYPTED_INBOX_ADDRESS`] whose calldata is an [`EncryptedPayload`]:
//!
//! ```text
//! 0x01 ‖ keyId (32) ‖ innerGasLimit (8, BE) ‖ U (33) ‖ c (32) ‖ z (32) ‖ AEAD body
//! ```
//!
//! The body is the threshold encryption ([`crate::threshold`]) of a complete signed inner
//! transaction (EIP-2718 bytes) under the committee key, with the [`envelope_label`] —
//! chain id, key id, the envelope's sender and the committed inner gas limit — as
//! associated data and proof-of-knowledge context. Binding the sender means a payload
//! copied into somebody else's envelope fails the proof and is never decrypted early.
//!
//! What stays visible: the envelope sender, nonce, fees and calldata size, and the inner
//! gas limit commitment. Everything else (target, value, calldata, the inner sender) is
//! hidden until the committee releases decryption shares after the envelope's block is
//! committed.

use crate::threshold::{self, Ciphertext, PublicPoint, ThresholdError};
use xdevs_primitives::{
    constants::ENCRYPTED_INBOX_ADDRESS, keccak256, Address, Bytes, ChainConfig, B256,
};

/// Payload format version.
pub const PAYLOAD_VERSION: u8 = 0x01;
/// Fixed header: version, key id, inner gas limit.
pub const PAYLOAD_HEADER_LEN: usize = 1 + 32 + 8;
/// Smallest valid payload (empty plaintext).
pub const MIN_PAYLOAD_LEN: usize = PAYLOAD_HEADER_LEN + threshold::CIPHERTEXT_OVERHEAD;

const LABEL_DOMAIN: &[u8] = b"xdevs/encrypted-tx/v1";

/// Key id of a committee public key: `keccak256(compressed point)`.
pub fn key_id(public_key: &PublicPoint) -> B256 {
    keccak256(public_key.as_bytes())
}

/// The context a payload is bound to.
pub fn envelope_label(
    chain_id: u64,
    key_id: B256,
    sender: Address,
    inner_gas_limit: u64,
) -> Vec<u8> {
    let mut l = Vec::with_capacity(LABEL_DOMAIN.len() + 8 + 32 + 20 + 8);
    l.extend_from_slice(LABEL_DOMAIN);
    l.extend_from_slice(&chain_id.to_be_bytes());
    l.extend_from_slice(key_id.as_slice());
    l.extend_from_slice(sender.as_slice());
    l.extend_from_slice(&inner_gas_limit.to_be_bytes());
    l
}

/// Decoded envelope calldata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedPayload {
    /// Committee key the payload is encrypted to.
    pub key_id: B256,
    /// Upper bound on the inner transaction's gas limit (visible; reserves room in the
    /// next block).
    pub inner_gas_limit: u64,
    /// The threshold ciphertext of the inner transaction.
    pub ciphertext: Ciphertext,
}

impl EncryptedPayload {
    /// Calldata encoding.
    pub fn encode(&self) -> Bytes {
        let ct = self.ciphertext.to_bytes();
        let mut out = Vec::with_capacity(PAYLOAD_HEADER_LEN + ct.len());
        out.push(PAYLOAD_VERSION);
        out.extend_from_slice(self.key_id.as_slice());
        out.extend_from_slice(&self.inner_gas_limit.to_be_bytes());
        out.extend_from_slice(&ct);
        out.into()
    }

    /// Decode calldata (structure only; [`MempoolKey::check`] verifies it).
    pub fn decode(data: &[u8]) -> Result<Self, ThresholdError> {
        if data.len() < MIN_PAYLOAD_LEN {
            return Err(ThresholdError::Malformed(format!(
                "payload of {} bytes, at least {MIN_PAYLOAD_LEN}",
                data.len()
            )));
        }
        if data[0] != PAYLOAD_VERSION {
            return Err(ThresholdError::Malformed(format!(
                "payload version {:#04x}",
                data[0]
            )));
        }
        let key_id = B256::from_slice(&data[1..33]);
        let mut gas = [0u8; 8];
        gas.copy_from_slice(&data[33..41]);
        Ok(Self {
            key_id,
            inner_gas_limit: u64::from_be_bytes(gas),
            ciphertext: Ciphertext::from_bytes(&data[PAYLOAD_HEADER_LEN..])?,
        })
    }

    /// The inner gas limit of envelope calldata without decoding the rest (`None` if
    /// the calldata is too short or of another version). Block builders use it to keep a
    /// block's envelopes within [`ChainConfig::encrypted_gas_budget`].
    pub fn peek_inner_gas(data: &[u8]) -> Option<u64> {
        if data.len() < MIN_PAYLOAD_LEN || data[0] != PAYLOAD_VERSION {
            return None;
        }
        let mut gas = [0u8; 8];
        gas.copy_from_slice(&data[33..41]);
        Some(u64::from_be_bytes(gas))
    }
}

/// The committee key of a chain, validated from [`ChainConfig::encrypted_mempool`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MempoolKey {
    /// `t`.
    pub threshold: u32,
    /// Committee public key.
    pub public_key: PublicPoint,
    /// `keccak256(public_key)`.
    pub key_id: B256,
    /// `Y_i`, share index `i` at position `i - 1`.
    pub verification_keys: Vec<PublicPoint>,
}

impl MempoolKey {
    /// Parse and validate the chain's committee key (`Ok(None)` if none is configured).
    pub fn from_config(cfg: &ChainConfig) -> Result<Option<Self>, ThresholdError> {
        let Some(p) = &cfg.encrypted_mempool else {
            return Ok(None);
        };
        let n = p.verification_keys.len();
        if p.threshold == 0 || p.threshold as usize > n || n > threshold::MAX_COMMITTEE as usize {
            return Err(ThresholdError::InvalidParameters(format!(
                "encryptedMempool threshold {} of {n} verification keys",
                p.threshold
            )));
        }
        let public_key = PublicPoint::from_slice(&p.public_key)?;
        let verification_keys = p
            .verification_keys
            .iter()
            .map(|k| PublicPoint::from_slice(k))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Self {
            threshold: p.threshold,
            key_id: key_id(&public_key),
            public_key,
            verification_keys,
        }))
    }

    /// Committee size `n`.
    pub fn committee_size(&self) -> u32 {
        self.verification_keys.len() as u32
    }

    /// Verification key of share `index` (1-based).
    pub fn verification_key(&self, index: u32) -> Option<&PublicPoint> {
        (index as usize)
            .checked_sub(1)
            .and_then(|i| self.verification_keys.get(i))
    }

    /// Check a payload carried by an envelope from `sender`: current key id, inner gas
    /// within `max_inner_gas`, and the ciphertext's proof of knowledge for the label.
    /// Only payloads that pass are ever decrypted.
    pub fn check(
        &self,
        chain_id: u64,
        sender: Address,
        payload: &EncryptedPayload,
        max_inner_gas: u64,
    ) -> Result<(), ThresholdError> {
        if payload.key_id != self.key_id {
            return Err(ThresholdError::Malformed(format!(
                "payload for key {}, current key is {}",
                payload.key_id, self.key_id
            )));
        }
        if payload.inner_gas_limit == 0 || payload.inner_gas_limit > max_inner_gas {
            return Err(ThresholdError::Malformed(format!(
                "inner gas limit {} outside 1..={max_inner_gas}",
                payload.inner_gas_limit
            )));
        }
        payload.ciphertext.verify(&envelope_label(
            chain_id,
            self.key_id,
            sender,
            payload.inner_gas_limit,
        ))
    }

    /// Encrypt a signed inner transaction (`inner_raw`, EIP-2718 bytes) for an envelope
    /// that `sender` will sign. `inner_gas_limit` must be at least the inner tx's gas
    /// limit, or the decrypted tx is skipped.
    pub fn seal(
        &self,
        chain_id: u64,
        sender: Address,
        inner_raw: &[u8],
        inner_gas_limit: u64,
    ) -> EncryptedPayload {
        let label = envelope_label(chain_id, self.key_id, sender, inner_gas_limit);
        EncryptedPayload {
            key_id: self.key_id,
            inner_gas_limit,
            ciphertext: threshold::encrypt_random(&self.public_key, &label, inner_raw),
        }
    }

    /// Open a payload from `sender` with verified shares (see [`threshold::combine`]).
    pub fn open(
        &self,
        chain_id: u64,
        sender: Address,
        payload: &EncryptedPayload,
        shares: &[threshold::DecryptionShare],
    ) -> Result<Vec<u8>, ThresholdError> {
        let label = envelope_label(chain_id, self.key_id, sender, payload.inner_gas_limit);
        threshold::combine(self.threshold, shares, &payload.ciphertext, &label)
    }
}

/// `true` if `to` is the encrypted-mempool inbox.
pub fn is_envelope_target(to: Option<Address>) -> bool {
    to == Some(ENCRYPTED_INBOX_ADDRESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threshold::deal_random;
    use xdevs_primitives::EncryptedMempoolParams;

    fn config(t: u32, n: u32) -> (ChainConfig, threshold::Dealt) {
        let d = deal_random(t, n).unwrap();
        let mut cfg = ChainConfig::new(7117);
        cfg.encrypted_mempool_block = Some(1);
        cfg.encrypted_mempool = Some(EncryptedMempoolParams {
            threshold: t,
            public_key: Bytes::copy_from_slice(d.public_key.as_bytes()),
            verification_keys: d
                .verification_keys
                .iter()
                .map(|k| Bytes::copy_from_slice(k.as_bytes()))
                .collect(),
        });
        (cfg, d)
    }

    #[test]
    fn seal_check_open() {
        let (cfg, d) = config(2, 3);
        let key = MempoolKey::from_config(&cfg).unwrap().unwrap();
        assert_eq!(key.committee_size(), 3);
        assert!(key.verification_key(0).is_none() && key.verification_key(4).is_none());
        let sender = Address::repeat_byte(0xaa);
        let inner = b"\x02inner-tx-bytes".to_vec();
        let payload = key.seal(cfg.chain_id, sender, &inner, 100_000);
        let data = payload.encode();
        assert_eq!(EncryptedPayload::peek_inner_gas(&data), Some(100_000));
        let back = EncryptedPayload::decode(&data).unwrap();
        assert_eq!(back, payload);
        key.check(cfg.chain_id, sender, &back, cfg.encrypted_gas_budget())
            .unwrap();

        // Copied into another sender's envelope: refused (never decrypted early).
        let thief = Address::repeat_byte(0xbb);
        assert!(key.check(cfg.chain_id, thief, &back, u64::MAX).is_err());
        // Another chain, another committed gas, over budget: refused.
        assert!(key.check(1, sender, &back, u64::MAX).is_err());
        let mut more_gas = back.clone();
        more_gas.inner_gas_limit += 1;
        assert!(key
            .check(cfg.chain_id, sender, &more_gas, u64::MAX)
            .is_err());
        assert!(key.check(cfg.chain_id, sender, &back, 99_999).is_err());

        let shares: Vec<_> = d.shares[1..]
            .iter()
            .map(|s| s.decryption_share_random(&back.ciphertext).unwrap())
            .collect();
        for (s, i) in shares.iter().zip(1..) {
            s.verify(&key.verification_keys[i], &back.ciphertext)
                .unwrap();
        }
        assert_eq!(
            key.open(cfg.chain_id, sender, &back, &shares).unwrap(),
            inner
        );
        assert!(key.open(cfg.chain_id, thief, &back, &shares).is_err());
    }

    #[test]
    fn malformed_payloads_and_keys() {
        let (cfg, _) = config(1, 1);
        let key = MempoolKey::from_config(&cfg).unwrap().unwrap();
        assert!(EncryptedPayload::decode(&[]).is_err());
        assert!(EncryptedPayload::decode(&[1u8; MIN_PAYLOAD_LEN - 1]).is_err());
        let mut data = key
            .seal(cfg.chain_id, Address::ZERO, b"x", 21_000)
            .encode()
            .to_vec();
        data[0] = 2;
        assert!(EncryptedPayload::decode(&data).is_err());
        assert_eq!(EncryptedPayload::peek_inner_gas(&data), None);
        // Wrong key id.
        data[0] = PAYLOAD_VERSION;
        data[1] ^= 1;
        let p = EncryptedPayload::decode(&data).unwrap();
        assert!(key
            .check(cfg.chain_id, Address::ZERO, &p, u64::MAX)
            .is_err());

        assert_eq!(MempoolKey::from_config(&ChainConfig::new(1)).unwrap(), None);
        let mut bad = cfg.clone();
        bad.encrypted_mempool.as_mut().unwrap().threshold = 2;
        assert!(MempoolKey::from_config(&bad).is_err());
        let mut bad = cfg.clone();
        bad.encrypted_mempool.as_mut().unwrap().public_key = Bytes::from(vec![0u8; 33]);
        assert!(MempoolKey::from_config(&bad).is_err());
        assert!(is_envelope_target(Some(ENCRYPTED_INBOX_ADDRESS)));
        assert!(!is_envelope_target(None));
    }
}

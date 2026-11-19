//! xDEVS Chain precompiles (plan §10.3, `docs/specs/precompiles.md`).
//!
//! `ED25519VERIFY` at `0x0A00…0001`: Ed25519 signature verification for light clients and
//! bridges (Phase 7). It exists from [`ChainConfig::ed25519_verify_block`] on; before that
//! (and on chains that never set it) the address is an ordinary empty account.
//!
//! * Input: `public key (32) ‖ signature R ‖ S (64) ‖ message (any length)`.
//! * Output: 32 bytes `0x…01` if the signature is valid; empty otherwise (short input, a key
//!   that is not a curve point, a bad signature). It never reverts, like `P256VERIFY`
//!   (EIP-7951), so callers test `returndatasize`.
//! * Rule: ZIP-215 (`ed25519-consensus`), exactly what xDEVS consensus accepts for votes, so
//!   a contract verifying xDEVS commit certificates agrees with the validators bit for bit.
//! * Gas: [`ED25519_VERIFY_BASE_GAS`] + [`ED25519_VERIFY_WORD_GAS`] per 32-byte word of the
//!   message (SHA-512 over it), charged also for an invalid signature; out of gas halts.

use revm::{
    handler::EthPrecompiles,
    precompile::{
        call_eth_precompile, EthPrecompileOutput, EthPrecompileResult, Precompile, PrecompileHalt,
        PrecompileId, PrecompileResult, Precompiles,
    },
};
use std::{borrow::Cow, sync::OnceLock};
use xdevs_primitives::{address, Address, Bytes, ChainConfig, B256};

use crate::env::SPEC;

/// Address of `ED25519VERIFY` (plan §10.3, xDEVS Chain precompile range `0x0A00…`).
pub const ED25519_VERIFY_ADDRESS: Address = address!("0x0a00000000000000000000000000000000000001");
/// Base gas of `ED25519VERIFY` (between `ecrecover`, 3,000, and `P256VERIFY`, 6,900).
pub const ED25519_VERIFY_BASE_GAS: u64 = 3_000;
/// Gas per 32-byte word of the message (the SHA-512 pass; `SHA256` charges 12).
pub const ED25519_VERIFY_WORD_GAS: u64 = 12;

/// The `ED25519VERIFY` precompile.
pub const ED25519_VERIFY: Precompile = Precompile::new(
    PrecompileId::Custom(Cow::Borrowed("ED25519VERIFY")),
    ED25519_VERIFY_ADDRESS,
    ed25519_verify_precompile,
);

fn ed25519_verify_precompile(input: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
    Ok(call_eth_precompile(
        ed25519_verify,
        input,
        gas_limit,
        reservoir,
    ))
}

/// Gas of an `ED25519VERIFY` call with `input_len` input bytes.
pub fn ed25519_verify_gas(input_len: usize) -> u64 {
    let words = input_len.saturating_sub(96).div_ceil(32) as u64;
    ED25519_VERIFY_BASE_GAS.saturating_add(ED25519_VERIFY_WORD_GAS.saturating_mul(words))
}

/// `ED25519VERIFY` on raw input (see the module docs).
pub fn ed25519_verify(input: &[u8], gas_limit: u64) -> EthPrecompileResult {
    let cost = ed25519_verify_gas(input.len());
    if cost > gas_limit {
        return Err(PrecompileHalt::OutOfGas);
    }
    let out = if verify(input) {
        B256::with_last_byte(1).into()
    } else {
        Bytes::new()
    };
    Ok(EthPrecompileOutput::new(cost, out))
}

fn verify(input: &[u8]) -> bool {
    if input.len() < 96 {
        return false;
    }
    let (key, rest) = input.split_at(32);
    let (sig, msg) = rest.split_at(64);
    let Ok(key) = ed25519_consensus::VerificationKey::try_from(key) else {
        return false;
    };
    let Ok(sig) = <[u8; 64]>::try_from(sig) else {
        return false;
    };
    key.verify(&ed25519_consensus::Signature::from(sig), msg)
        .is_ok()
}

/// The Osaka precompiles plus `ED25519VERIFY`.
pub fn with_ed25519() -> &'static Precompiles {
    static SET: OnceLock<Precompiles> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set = Precompiles::osaka().clone();
        set.extend([ED25519_VERIFY]);
        set
    })
}

/// The precompile provider of block `number` under `cfg`.
pub(crate) fn provider(cfg: &ChainConfig, number: u64) -> EthPrecompiles {
    if cfg.ed25519_verify_active(number) {
        EthPrecompiles {
            precompiles: with_ed25519(),
            spec: SPEC,
        }
    } else {
        EthPrecompiles::new(SPEC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_consensus::SigningKey;

    fn input(sk: &SigningKey, msg: &[u8]) -> Vec<u8> {
        let mut v = sk.verification_key().to_bytes().to_vec();
        v.extend_from_slice(&sk.sign(msg).to_bytes());
        v.extend_from_slice(msg);
        v
    }

    #[test]
    fn verifies_valid_signatures() {
        let sk = SigningKey::from([7u8; 32]);
        for len in [0usize, 1, 31, 32, 66, 1000] {
            let msg = vec![0xab; len];
            let out = ed25519_verify(&input(&sk, &msg), 1_000_000).unwrap();
            assert_eq!(out.bytes, Bytes::from(B256::with_last_byte(1)));
            assert_eq!(out.gas_used, ed25519_verify_gas(96 + len));
        }
    }

    #[test]
    fn rejects_without_reverting() {
        let sk = SigningKey::from([7u8; 32]);
        let good = input(&sk, b"vote");
        // tampered message, signature, key; short input
        for i in [0usize, 40, 100] {
            let mut bad = good.clone();
            bad[i] ^= 1;
            let out = ed25519_verify(&bad, 1_000_000).unwrap();
            assert!(out.bytes.is_empty(), "byte {i}");
            assert_eq!(out.gas_used, ed25519_verify_gas(bad.len()));
        }
        assert!(ed25519_verify(&good[..95], 1_000_000)
            .unwrap()
            .bytes
            .is_empty());
        assert!(ed25519_verify(&[], 1_000_000).unwrap().bytes.is_empty());
    }

    #[test]
    fn rfc8032_test_1() {
        // RFC 8032 §7.1 TEST 1 (empty message).
        let key = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let sig = hex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        let out = ed25519_verify(&[key, sig].concat(), 3_000).unwrap();
        assert_eq!(out.bytes, Bytes::from(B256::with_last_byte(1)));
    }

    #[test]
    fn gas_schedule_and_out_of_gas() {
        assert_eq!(ed25519_verify_gas(96), 3_000);
        assert_eq!(ed25519_verify_gas(97), 3_012);
        assert_eq!(ed25519_verify_gas(96 + 66), 3_036);
        let sk = SigningKey::from([1u8; 32]);
        assert_eq!(
            ed25519_verify(&input(&sk, b"x"), 3_011),
            Err(PrecompileHalt::OutOfGas)
        );
    }

    #[test]
    fn activation_gate() {
        let mut cfg = ChainConfig::new(1);
        let off = provider(&cfg, 10);
        assert!(off.precompiles.get(&ED25519_VERIFY_ADDRESS).is_none());
        cfg.ed25519_verify_block = Some(10);
        assert!(provider(&cfg, 9)
            .precompiles
            .get(&ED25519_VERIFY_ADDRESS)
            .is_none());
        let on = provider(&cfg, 10);
        assert!(on.precompiles.get(&ED25519_VERIFY_ADDRESS).is_some());
        // Every Osaka precompile is still there.
        for a in Precompiles::osaka().addresses() {
            assert!(on.precompiles.contains(a));
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}

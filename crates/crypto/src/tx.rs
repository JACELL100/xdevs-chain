//! Transaction sender recovery and signing.

use alloy_consensus::{SignableTransaction, Signed, TxEnvelope};
use alloy_primitives::Signature;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use xdevs_primitives::{
    reexports::alloy_eips::eip7702::{Authorization, SignedAuthorization},
    Decodable2718, RecoveredTx, Transaction,
};

/// Why an incoming transaction was refused before reaching the pool.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TxRecoveryError {
    /// The bytes are not a valid EIP-2718 transaction.
    #[error("rlp: {0}")]
    Decode(String),
    /// Pre-EIP-155 legacy transaction (no chain id): replayable, always rejected.
    #[error("only replay-protected (EIP-155) transactions allowed over RPC")]
    Unprotected,
    /// Transaction signed for another chain.
    #[error("invalid chain id for signer: have {got} want {expected}")]
    WrongChainId {
        /// This chain's id.
        expected: u64,
        /// Chain id in the transaction.
        got: u64,
    },
    /// Signature invalid (bad `r`/`s`, high-s, unrecoverable).
    #[error("invalid sender: {0}")]
    InvalidSignature(String),
}

/// Check replay protection and the chain id, then recover the sender (k256, EIP-2 low-s).
pub fn recover_tx(tx: TxEnvelope, chain_id: u64) -> Result<RecoveredTx, TxRecoveryError> {
    match tx.chain_id() {
        None => return Err(TxRecoveryError::Unprotected),
        Some(got) if got != chain_id => {
            return Err(TxRecoveryError::WrongChainId {
                expected: chain_id,
                got,
            })
        }
        Some(_) => {}
    }
    RecoveredTx::recover(tx).map_err(|e| TxRecoveryError::InvalidSignature(e.to_string()))
}

/// Decode raw `eth_sendRawTransaction` bytes (EIP-2718, no trailing bytes allowed) and
/// [`recover_tx`] them.
pub fn decode_and_recover(raw: &[u8], chain_id: u64) -> Result<RecoveredTx, TxRecoveryError> {
    let tx =
        TxEnvelope::decode_2718_exact(raw).map_err(|e| TxRecoveryError::Decode(e.to_string()))?;
    recover_tx(tx, chain_id)
}

/// Sign any transaction type with `signer` (signature over `tx.signature_hash()`).
pub fn sign_tx<T: SignableTransaction<Signature>>(
    signer: &PrivateKeySigner,
    tx: T,
) -> Result<Signed<T>, alloy_signer::Error> {
    let sig = signer.sign_hash_sync(&tx.signature_hash())?;
    Ok(tx.into_signed(sig))
}

/// Sign an EIP-7702 authorization tuple `(chain_id, address, nonce)` (signature over
/// `keccak256(0x05 ‖ rlp([chain_id, address, nonce]))`, RFC 6979 low-s).
pub fn sign_authorization(
    signer: &PrivateKeySigner,
    auth: Authorization,
) -> Result<SignedAuthorization, alloy_signer::Error> {
    let sig = signer.sign_hash_sync(&auth.signature_hash())?;
    Ok(auth.into_signed(sig))
}

/// Sign `tx` and wrap it into a [`TxEnvelope`] ready for `encoded_2718()`.
pub fn sign_tx_envelope<T>(
    signer: &PrivateKeySigner,
    tx: T,
) -> Result<TxEnvelope, alloy_signer::Error>
where
    T: SignableTransaction<Signature>,
    TxEnvelope: From<Signed<T>>,
{
    Ok(sign_tx(signer, tx)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::signer_from_hex;
    use alloy_primitives::{address, hex, Bytes, TxKind, U256};
    use xdevs_primitives::{Encodable2718, TxEip1559, TxLegacy};

    /// EIP-155 example (chain id 1).
    const EIP155_RAW: &str = "f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

    fn key() -> PrivateKeySigner {
        signer_from_hex("4646464646464646464646464646464646464646464646464646464646464646").unwrap()
    }

    #[test]
    fn eip155_vector_recovers() {
        let raw = hex::decode(EIP155_RAW).unwrap();
        let rtx = decode_and_recover(&raw, 1).unwrap();
        assert_eq!(
            rtx.sender,
            address!("0x9d8A62f656a8d1615C1294fd71e9CFb3E4855A4F")
        );
        assert_eq!(rtx.sender, key().address());
    }

    #[test]
    fn wrong_chain_rejected() {
        let raw = hex::decode(EIP155_RAW).unwrap();
        assert_eq!(
            decode_and_recover(&raw, 7117).unwrap_err(),
            TxRecoveryError::WrongChainId {
                expected: 7117,
                got: 1
            }
        );
    }

    #[test]
    fn trailing_bytes_rejected() {
        let mut raw = hex::decode(EIP155_RAW).unwrap();
        raw.push(0);
        assert!(matches!(
            decode_and_recover(&raw, 1),
            Err(TxRecoveryError::Decode(_))
        ));
        assert!(matches!(
            decode_and_recover(&[], 1),
            Err(TxRecoveryError::Decode(_))
        ));
    }

    #[test]
    fn unprotected_legacy_rejected() {
        let tx = TxLegacy {
            chain_id: None,
            nonce: 0,
            gas_price: 1_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(address!("0x3535353535353535353535353535353535353535")),
            value: U256::from(1),
            input: Bytes::new(),
        };
        let env = sign_tx_envelope(&key(), tx).unwrap();
        // Still a valid signature...
        assert!(RecoveredTx::recover(env.clone()).is_ok());
        // ...but refused.
        assert_eq!(
            recover_tx(env, 1).unwrap_err(),
            TxRecoveryError::Unprotected
        );
    }

    #[test]
    fn sign_eip155_reproduces_vector() {
        let tx = TxLegacy {
            chain_id: Some(1),
            nonce: 9,
            gas_price: 20_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(address!("0x3535353535353535353535353535353535353535")),
            value: U256::from(1_000_000_000_000_000_000u128),
            input: Bytes::new(),
        };
        let env = sign_tx_envelope(&key(), tx).unwrap();
        // RFC 6979 deterministic nonces => byte-identical to the spec example.
        assert_eq!(hex::encode(env.encoded_2718()), EIP155_RAW);
    }

    #[test]
    fn eip7702_sign_encode_and_recover() {
        use xdevs_primitives::{TxEip7702, U256};
        let signer = key();
        let other = crate::generate_signer();
        let auth = sign_authorization(
            &other,
            Authorization {
                chain_id: U256::from(7117),
                address: address!("0x0000000000000000000000000000000000001005"),
                nonce: 4,
            },
        )
        .unwrap();
        assert_eq!(auth.recover_authority().unwrap(), other.address());
        let tx = TxEip7702 {
            chain_id: 7117,
            nonce: 1,
            gas_limit: 100_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: 1,
            to: address!("0x00000000000000000000000000000000000000aa"),
            value: U256::ZERO,
            access_list: Default::default(),
            authorization_list: vec![auth.clone()],
            input: Bytes::new(),
        };
        let raw = sign_tx_envelope(&signer, tx).unwrap().encoded_2718();
        assert_eq!(raw[0], 4);
        let rtx = decode_and_recover(&raw, 7117).unwrap();
        assert_eq!(rtx.sender, signer.address());
        assert_eq!(rtx.authorization_list().unwrap(), &[auth][..]);
    }

    #[test]
    fn eip1559_sign_and_recover_roundtrip() {
        let signer = key();
        let tx = TxEip1559 {
            chain_id: 7117,
            nonce: 3,
            gas_limit: 21_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(address!("0x00000000000000000000000000000000000000aa")),
            value: U256::from(5),
            access_list: Default::default(),
            input: Bytes::new(),
        };
        let env = sign_tx_envelope(&signer, tx).unwrap();
        let raw = env.encoded_2718();
        assert_eq!(raw[0], 2);
        let rtx = decode_and_recover(&raw, 7117).unwrap();
        assert_eq!(rtx.sender, signer.address());
        assert_eq!(rtx.hash(), xdevs_primitives::keccak256(&raw));
        assert_eq!(rtx.nonce(), 3);
    }
}

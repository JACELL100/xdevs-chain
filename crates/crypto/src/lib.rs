//! Cryptography for xDEVS Chain (plan §9).
//!
//! * [`tx`]: sender recovery for incoming transactions (EIP-155 replay protection is
//!   mandatory for legacy txs) and transaction signing.
//! * [`keys`]: secp256k1 account key generation and parsing (`k256` via
//!   `alloy-signer-local`).
//! * [`keystore`]: Web3 Secret Storage v3 keystores (scrypt / pbkdf2 + AES-128-CTR +
//!   keccak MAC), compatible with geth, Foundry (`cast wallet`) and MetaMask exports.
//!
//! Primitives are never implemented by hand: every algorithm comes from an audited
//! RustCrypto / alloy crate. Secret material is zeroized on drop.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod keys;
pub mod keystore;
pub mod tx;

pub use alloy_signer_local::PrivateKeySigner;
pub use keys::{generate_signer, signer_from_hex, KeyError};
pub use keystore::{
    decrypt_key, decrypt_signer, encrypt_key, encrypt_signer, Keystore, KeystoreError, ScryptParams,
};
pub use tx::{
    decode_and_recover, recover_tx, sign_authorization, sign_tx, sign_tx_envelope, TxRecoveryError,
};
pub use xdevs_primitives::keccak256;

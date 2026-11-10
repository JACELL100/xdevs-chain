//! Core types for xDEVS Chain.
//!
//! Ethereum compatibility is the first rule: headers, transactions, receipts and
//! logs are the [`alloy`](https://github.com/alloy-rs) types re-exported as-is, so
//! hashes, RLP encodings and trie roots match Ethereum byte-for-byte. xDEVS-specific
//! data (chain configuration, flat-state accounts, recovered transactions) lives in
//! small wrapper types defined here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod account;
mod block;
mod config;
pub mod constants;
pub mod serde_helpers;
mod tx;

pub use account::{Account, AccountInfo};
pub use block::{
    compute_logs_bloom, compute_receipts_root, compute_transactions_root, set_compat_header_fields,
    Block, SealedHeader,
};
pub use config::{BaseFeeParams, ChainConfig};
pub use tx::{RecoveredTx, TxLocation};

// ---------------------------------------------------------------------------
// Re-exports: the canonical Ethereum types every other crate uses.
// ---------------------------------------------------------------------------

pub use alloy_consensus::Header;
pub use alloy_consensus::{
    transaction::SignerRecoverable, Receipt, ReceiptEnvelope, ReceiptWithBloom, Signed,
    Transaction, TxEip1559, TxEip2930, TxEip7702, TxEnvelope, TxLegacy, TxReceipt, TxType,
    EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH,
};
pub use alloy_eips::eip2718::{Decodable2718, Encodable2718, Typed2718};
pub use alloy_primitives::{
    address, b256, hex, keccak256, logs_bloom, Address, BlockHash, BlockNumber, Bloom, Bytes,
    ChainId, Log, LogData, Sealable, Sealed, Signature, StorageKey, StorageValue, TxHash, TxKind,
    B256, B64, KECCAK256_EMPTY, U256,
};

/// Re-export of the underlying alloy crates for advanced uses (e.g. `alloy_rlp` derives).
pub mod reexports {
    pub use alloy_consensus;
    pub use alloy_eips;
    pub use alloy_primitives;
    pub use alloy_rlp;
    pub use alloy_trie;
}

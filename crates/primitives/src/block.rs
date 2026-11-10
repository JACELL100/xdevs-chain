//! Blocks and Ethereum root helpers (plan §8.1).

use crate::constants::EMPTY_REQUESTS_HASH;
use alloy_consensus::{proofs, Header, ReceiptEnvelope, TxEnvelope, EMPTY_ROOT_HASH};
use alloy_primitives::{Bloom, Sealable, Sealed, TxHash, B256};
use serde::{Deserialize, Serialize};

/// A header together with its (cached) hash.
pub type SealedHeader = Sealed<Header>;

/// An xDEVS block: an Ethereum-compatible header plus its transactions.
///
/// Consensus data (commit certificates, round, proposer) is *not* part of this type and
/// not part of the header hash; it is stored alongside from Phase 2 on (plan §8.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Ethereum header; its `hash_slow()` is the block hash.
    pub header: Header,
    /// Transactions in execution order.
    pub body: Vec<TxEnvelope>,
}

impl Block {
    /// Create a block from its parts.
    pub const fn new(header: Header, body: Vec<TxEnvelope>) -> Self {
        Self { header, body }
    }

    /// Block hash = `keccak256(rlp(header))`. Recomputed on every call; cache it
    /// (e.g. with [`Block::seal_header`]) on hot paths.
    pub fn hash(&self) -> B256 {
        self.header.hash_slow()
    }

    /// Block number.
    pub const fn number(&self) -> u64 {
        self.header.number
    }

    /// Clone the header and seal it with its hash.
    pub fn seal_header(&self) -> SealedHeader {
        self.header.clone().seal_slow()
    }

    /// Hashes of the transactions in block order.
    pub fn tx_hashes(&self) -> impl Iterator<Item = TxHash> + '_ {
        self.body.iter().map(|tx| *tx.tx_hash())
    }

    /// Transactions root of the body (to compare against `header.transactions_root`).
    pub fn compute_transactions_root(&self) -> B256 {
        compute_transactions_root(&self.body)
    }
}

/// Ethereum transactions root: ordered trie of `rlp(index) => tx.encode_2718()`.
pub fn compute_transactions_root(txs: &[TxEnvelope]) -> B256 {
    if txs.is_empty() {
        return EMPTY_ROOT_HASH;
    }
    proofs::calculate_transaction_root(txs)
}

/// Ethereum receipts root: ordered trie of `rlp(index) => receipt.encode_2718()`.
pub fn compute_receipts_root(receipts: &[ReceiptEnvelope]) -> B256 {
    if receipts.is_empty() {
        return EMPTY_ROOT_HASH;
    }
    proofs::calculate_receipt_root(receipts)
}

/// Block logs bloom: union of all receipt blooms.
pub fn compute_logs_bloom(receipts: &[ReceiptEnvelope]) -> Bloom {
    let mut bloom = Bloom::ZERO;
    for r in receipts {
        bloom.accrue_bloom(r.logs_bloom());
    }
    bloom
}

/// Set the post-merge "compatibility" header fields that xDEVS keeps constant
/// (plan §8.1): `difficulty = 0`, `nonce = 0`, empty withdrawals root, zero parent
/// beacon root, empty requests hash and zero blob gas. Every block producer (genesis,
/// dev producer, Phase-2 proposer) must call this so headers hash identically everywhere.
pub fn set_compat_header_fields(header: &mut Header) {
    header.difficulty = Default::default();
    header.nonce = Default::default();
    header.ommers_hash = alloy_consensus::EMPTY_OMMER_ROOT_HASH;
    header.withdrawals_root = Some(EMPTY_ROOT_HASH);
    header.blob_gas_used = Some(0);
    header.excess_blob_gas = Some(0);
    header.parent_beacon_block_root = Some(B256::ZERO);
    header.requests_hash = Some(EMPTY_REQUESTS_HASH);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_roots() {
        assert_eq!(compute_transactions_root(&[]), EMPTY_ROOT_HASH);
        assert_eq!(compute_receipts_root(&[]), EMPTY_ROOT_HASH);
        assert_eq!(compute_logs_bloom(&[]), Bloom::ZERO);
    }

    #[test]
    fn block_hash_is_header_hash() {
        let mut header = Header {
            number: 5,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        set_compat_header_fields(&mut header);
        let block = Block::new(header.clone(), vec![]);
        assert_eq!(block.hash(), header.hash_slow());
        assert_eq!(block.seal_header().hash(), block.hash());
        assert_eq!(block.compute_transactions_root(), EMPTY_ROOT_HASH);
    }

    #[test]
    fn requests_hash_constant_is_sha256_of_empty() {
        // sha256("") — EIP-7685 requests hash of a block without requests.
        assert_eq!(
            EMPTY_REQUESTS_HASH,
            alloy_primitives::b256!(
                "0xe3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            )
        );
    }
}

//! Blocks and Ethereum root helpers (plan §8.1).

use crate::constants::{CONSENSUS_DATA_DOMAIN, EMPTY_REQUESTS_HASH};
use alloy_consensus::{proofs, Header, ReceiptEnvelope, TxEnvelope, EMPTY_ROOT_HASH};
use alloy_primitives::{keccak256, Bloom, Bytes, Sealable, Sealed, TxHash, B256};
use serde::{Deserialize, Serialize};

/// A header together with its (cached) hash.
pub type SealedHeader = Sealed<Header>;

/// An xDEVS block: an Ethereum-compatible header, its transactions and (from the
/// staking activation on, Phase 3) its consensus data.
///
/// The commit certificate *of this block* is not part of this type and not part of the
/// header hash; it is stored alongside (plan §8.1). `consensus` carries what execution
/// needs from consensus (the previous block's commit certificate, the proposer and
/// double-sign evidence) as opaque bytes (`xdevs_consensus::BlockConsensusData`); the
/// header commits to it through `requests_hash` ([`consensus_data_hash`]). Empty before
/// the activation, so older blocks hash and encode exactly as before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Ethereum header; its `hash_slow()` is the block hash.
    pub header: Header,
    /// Transactions in execution order.
    pub body: Vec<TxEnvelope>,
    /// Consensus data (RLP), empty before the staking activation.
    #[serde(default, skip_serializing_if = "is_empty")]
    pub consensus: Bytes,
}

impl Block {
    /// Create a block from its parts (no consensus data).
    pub const fn new(header: Header, body: Vec<TxEnvelope>) -> Self {
        Self {
            header,
            body,
            consensus: Bytes::new(),
        }
    }

    /// Create a block with consensus data (see [`Block::consensus`]).
    pub const fn with_consensus(header: Header, body: Vec<TxEnvelope>, consensus: Bytes) -> Self {
        Self {
            header,
            body,
            consensus,
        }
    }

    /// `true` if `header.requests_hash` commits to [`Block::consensus`]. A block without
    /// consensus data also matches a header without `requests_hash` (the protocol value
    /// of that field is checked with the other compatibility fields).
    pub fn consensus_matches_header(&self) -> bool {
        match self.header.requests_hash {
            None => self.consensus.is_empty(),
            Some(hash) => hash == consensus_data_hash(&self.consensus),
        }
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

fn is_empty(b: &Bytes) -> bool {
    b.is_empty()
}

/// Header `requests_hash` committing to a block's consensus data: the EIP-7685 empty
/// value `sha256("")` when there is none (every block before the staking activation),
/// else `keccak256(0x78 ‖ data)`. Binding the data to the header makes the block hash
/// (what consensus votes on) cover it.
pub fn consensus_data_hash(data: &[u8]) -> B256 {
    if data.is_empty() {
        return EMPTY_REQUESTS_HASH;
    }
    let mut buf = Vec::with_capacity(1 + data.len());
    buf.push(CONSENSUS_DATA_DOMAIN);
    buf.extend_from_slice(data);
    keccak256(buf)
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
    fn consensus_data_commitment() {
        let mut header = Header::default();
        set_compat_header_fields(&mut header);
        let plain = Block::new(header.clone(), vec![]);
        assert!(plain.consensus.is_empty() && plain.consensus_matches_header());
        assert_eq!(consensus_data_hash(&[]), EMPTY_REQUESTS_HASH);

        let data = Bytes::from_static(b"consensus");
        let mut with = Block::with_consensus(header.clone(), vec![], data.clone());
        assert!(!with.consensus_matches_header());
        with.header.requests_hash = Some(consensus_data_hash(&data));
        assert!(with.consensus_matches_header());
        assert_ne!(with.hash(), plain.hash());
        with.consensus = Bytes::from_static(b"other");
        assert!(!with.consensus_matches_header());
        with.header.requests_hash = None;
        assert!(!with.consensus_matches_header());
        with.consensus = Bytes::new();
        assert!(with.consensus_matches_header());
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

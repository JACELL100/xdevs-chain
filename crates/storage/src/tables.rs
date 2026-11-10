//! Table definitions (plan §14.1).
//!
//! | Table            | Key                    | Value                                  |
//! |------------------|------------------------|----------------------------------------|
//! | `Headers`        | block number (u64 BE)  | header (RLP)                           |
//! | `HeaderNumbers`  | block hash             | block number                           |
//! | `Bodies`         | block number           | RLP list of txs (network encoding)     |
//! | `Transactions`   | tx hash                | [`TxLocation`] (block number, index)   |
//! | `Receipts`       | (number, index)        | receipt (EIP-2718 encoding)            |
//! | `PlainAccounts`  | address                | [`AccountInfo`] (RLP)                  |
//! | `PlainStorage`   | (address, slot)        | value (32 bytes BE, never zero)        |
//! | `Bytecodes`      | code hash              | raw bytecode                           |
//! | `Meta`           | utf-8 string           | raw bytes (see [`crate::meta_keys`])   |
//! | `AccountHistory` | (address, block)       | account *before* `block` (or absent)   |
//! | `StorageHistory` | (address, slot, block) | slot value *before* `block` (0 = unset)|
//! | `StateChangeSets`| block number           | [`StateChangeSet`] (keys for pruning)  |
//! | `CommitCertificates` | block number       | commit certificate (RLP, Phase 2)      |
//! | `TrieNodes`      | tag ‖ [hashed addr] ‖ nibble path | trie node (RLP), see [`TrieNodes`] |
//!
//! `Transactions` stores only the location; the transaction itself lives in `Bodies`
//! (saves disk vs. storing it twice). The history tables hold reverse diffs for the
//! last N blocks (§14.4) so state can be read at recent past blocks; the
//! `StateChangeSets` index lets old entries be pruned block by block.
//! `CommitCertificates` (Phase 2) holds the commit certificate of each block, written in
//! the same batch as the block; `TrieNodes` (Phase 2) caches Merkle-Patricia trie nodes
//! for incremental state roots. Both are additive: [`crate::SCHEMA_VERSION`] is unchanged
//! and backends create missing tables on open. `LogIndex` arrives in a later phase.

use crate::codec::{Decode, Encode};
use xdevs_primitives::{
    AccountInfo, Address, Bytes, Header, ReceiptEnvelope, TxEnvelope, TxLocation, B256, U256,
};

/// Identifier of a physical table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TableId {
    /// `number -> header`.
    Headers,
    /// `hash -> number`.
    HeaderNumbers,
    /// `number -> transactions`.
    Bodies,
    /// `tx hash -> location`.
    Transactions,
    /// `(number, index) -> receipt`.
    Receipts,
    /// `address -> account`.
    PlainAccounts,
    /// `(address, slot) -> value`.
    PlainStorage,
    /// `code hash -> bytecode`.
    Bytecodes,
    /// Misc metadata.
    Meta,
    /// `(address, block) -> account before block`.
    AccountHistory,
    /// `(address, slot, block) -> value before block`.
    StorageHistory,
    /// `block -> keys changed by block`.
    StateChangeSets,
    /// `block number -> commit certificate` (Phase 2).
    CommitCertificates,
    /// `trie node key -> RLP trie node` (Phase 2, incremental state root).
    TrieNodes,
}

impl TableId {
    /// Every table, in a fixed order (backends create them all on open).
    pub const ALL: [TableId; 14] = [
        TableId::Headers,
        TableId::HeaderNumbers,
        TableId::Bodies,
        TableId::Transactions,
        TableId::Receipts,
        TableId::PlainAccounts,
        TableId::PlainStorage,
        TableId::Bytecodes,
        TableId::Meta,
        TableId::AccountHistory,
        TableId::StorageHistory,
        TableId::StateChangeSets,
        TableId::CommitCertificates,
        TableId::TrieNodes,
    ];

    /// Stable on-disk table name.
    pub const fn name(self) -> &'static str {
        match self {
            TableId::Headers => "Headers",
            TableId::HeaderNumbers => "HeaderNumbers",
            TableId::Bodies => "Bodies",
            TableId::Transactions => "Transactions",
            TableId::Receipts => "Receipts",
            TableId::PlainAccounts => "PlainAccounts",
            TableId::PlainStorage => "PlainStorage",
            TableId::Bytecodes => "Bytecodes",
            TableId::Meta => "Meta",
            TableId::AccountHistory => "AccountHistory",
            TableId::StorageHistory => "StorageHistory",
            TableId::StateChangeSets => "StateChangeSets",
            TableId::CommitCertificates => "CommitCertificates",
            TableId::TrieNodes => "TrieNodes",
        }
    }
}

/// A typed table: physical id + key/value codecs.
pub trait Table: Send + Sync + 'static {
    /// Physical table.
    const ID: TableId;
    /// Key type (encoding preserves ordering).
    type Key: Encode + Decode;
    /// Value type.
    type Value: Encode + Decode;
}

macro_rules! table {
    ($(#[$doc:meta])* $name:ident, $key:ty, $value:ty) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default)]
        pub struct $name;
        impl Table for $name {
            const ID: TableId = TableId::$name;
            type Key = $key;
            type Value = $value;
        }
    };
}

table!(/// `block number -> header (RLP)`.
    Headers, u64, Header);
table!(/// `block hash -> block number`.
    HeaderNumbers, B256, u64);
table!(/// `block number -> transactions`.
    Bodies, u64, Vec<TxEnvelope>);
table!(/// `tx hash -> (block number, index)`.
    Transactions, B256, TxLocation);
table!(/// `(block number, tx index) -> receipt (EIP-2718)`.
    Receipts, (u64, u64), ReceiptEnvelope);
table!(/// `address -> flat account`.
    PlainAccounts, Address, AccountInfo);
table!(/// `(address, slot) -> value`. Zero values are deleted, never stored.
    PlainStorage, (Address, B256), U256);
table!(/// `code hash -> bytecode`.
    Bytecodes, B256, Bytes);
table!(/// `string key -> raw bytes`, see [`crate::meta_keys`].
    Meta, String, Vec<u8>);
table!(/// `(address, block) -> account as it was before that block` (`None` = absent).
    AccountHistory, (Address, u64), Option<AccountInfo>);
table!(/// `(address, slot, block) -> value before that block` (zero = unset).
    StorageHistory, (Address, B256, u64), U256);
table!(/// `block -> keys whose history entries that block wrote` (pruning index).
    StateChangeSets, u64, StateChangeSet);
table!(/// `block number -> commit certificate` (Phase 2). The value is the RLP of
    /// `xdevs_consensus::CommitCertificate` (research doc §6.5), stored opaquely so storage
    /// does not depend on consensus. Written in the same `WriteBatch` as the block; may be
    /// replaced by a certificate with more signatures at `Finalized`.
    CommitCertificates, u64, Bytes);
table!(/// Merkle-Patricia trie node cache for incremental state roots (Phase 2,
    /// ADR-0004). Key = [`account_trie_key`] / [`storage_trie_key`]: a tag byte
    /// (`0x00` account trie, `0x01` storage trie), for storage tries the 32-byte
    /// `keccak256(address)`, then the node's nibble path from the root, **one nibble per
    /// byte** (`0x00..=0x0f`). Value = the RLP-encoded trie node. The tag and the
    /// fixed-length owner make keys prefix-free per trie, so one account's whole storage
    /// trie can be removed with a prefix delete, and walks visit nodes in path order.
    TrieNodes, Vec<u8>, Bytes);

/// [`TrieNodes`] tag of account-trie nodes.
pub const TRIE_NODES_ACCOUNT_TAG: u8 = 0x00;
/// [`TrieNodes`] tag of storage-trie nodes.
pub const TRIE_NODES_STORAGE_TAG: u8 = 0x01;

/// [`TrieNodes`] key of the account-trie node at nibble path `path` (one nibble per byte).
pub fn account_trie_key(path: &[u8]) -> Vec<u8> {
    debug_assert!(path.iter().all(|n| *n < 16), "nibble path");
    let mut k = Vec::with_capacity(1 + path.len());
    k.push(TRIE_NODES_ACCOUNT_TAG);
    k.extend_from_slice(path);
    k
}

/// [`TrieNodes`] key of the node at nibble path `path` in the storage trie of the account
/// whose hashed address is `hashed_address` (`keccak256(address)`).
pub fn storage_trie_key(hashed_address: B256, path: &[u8]) -> Vec<u8> {
    debug_assert!(path.iter().all(|n| *n < 16), "nibble path");
    let mut k = storage_trie_prefix(hashed_address);
    k.extend_from_slice(path);
    k
}

/// Prefix shared by every [`TrieNodes`] key of one account's storage trie.
pub fn storage_trie_prefix(hashed_address: B256) -> Vec<u8> {
    let mut k = Vec::with_capacity(33 + 64);
    k.push(TRIE_NODES_STORAGE_TAG);
    k.extend_from_slice(hashed_address.as_slice());
    k
}

/// The state keys changed by one block: the keys of its [`AccountHistory`] and
/// [`StorageHistory`] entries, kept so they can be pruned without a table scan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateChangeSet {
    /// Changed accounts.
    pub accounts: Vec<Address>,
    /// Changed storage slots.
    pub slots: Vec<(Address, B256)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{meta_keys, Database, DbReadExt, MemDb, RedbDatabase, WriteBatch, SCHEMA_VERSION};

    #[test]
    fn table_names_are_unique() {
        let names: std::collections::BTreeSet<_> = TableId::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names.len(), TableId::ALL.len());
        assert!(names.contains("CommitCertificates") && names.contains("TrieNodes"));
    }

    #[test]
    fn trie_node_keys() {
        let h = B256::repeat_byte(0xaa);
        assert_eq!(account_trie_key(&[]), vec![0x00]);
        assert_eq!(account_trie_key(&[1, 0xf]), vec![0x00, 1, 0xf]);
        let k = storage_trie_key(h, &[3]);
        assert_eq!(k.len(), 1 + 32 + 1);
        assert!(k.starts_with(&storage_trie_prefix(h)));
        assert!(
            !storage_trie_key(B256::repeat_byte(0xab), &[]).starts_with(&storage_trie_prefix(h))
        );
    }

    fn roundtrip(db: &dyn Database) {
        let h = B256::repeat_byte(1);
        let mut b = WriteBatch::new();
        b.put::<CommitCertificates>(&7, &Bytes::from_static(b"cert7"))
            .put::<TrieNodes>(&account_trie_key(&[]), &Bytes::from_static(b"root"))
            .put::<TrieNodes>(&storage_trie_key(h, &[]), &Bytes::from_static(b"s0"))
            .put::<TrieNodes>(&storage_trie_key(h, &[5]), &Bytes::from_static(b"s5"));
        db.write(b).unwrap();
        assert_eq!(
            db.get::<CommitCertificates>(&7).unwrap(),
            Some(Bytes::from_static(b"cert7"))
        );
        assert_eq!(db.get::<CommitCertificates>(&8).unwrap(), None);
        let storage = db
            .collect_prefix::<TrieNodes>(&storage_trie_prefix(h))
            .unwrap();
        assert_eq!(storage.len(), 2);
        assert_eq!(storage[1].0, storage_trie_key(h, &[5]));
        let mut b = WriteBatch::new();
        b.delete_prefix(TableId::TrieNodes, storage_trie_prefix(h));
        db.write(b).unwrap();
        assert!(db
            .collect_prefix::<TrieNodes>(&storage_trie_prefix(h))
            .unwrap()
            .is_empty());
        assert!(db
            .get::<TrieNodes>(&account_trie_key(&[]))
            .unwrap()
            .is_some());
    }

    #[test]
    fn new_tables_roundtrip() {
        roundtrip(&MemDb::new());
        let dir = tempfile::tempdir().unwrap();
        roundtrip(&RedbDatabase::open_dir(dir.path()).unwrap());
    }

    /// A Phase-1 database (without the Phase-2 tables) opens unchanged: the new tables
    /// are created on open and the schema version stays the same.
    #[test]
    fn phase1_database_gains_new_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.redb");
        {
            let db = redb::Database::create(&path).unwrap();
            let txn = db.begin_write().unwrap();
            {
                let def = |t: TableId| redb::TableDefinition::<&[u8], &[u8]>::new(t.name());
                for t in &TableId::ALL[..12] {
                    txn.open_table(def(*t)).unwrap();
                }
                let mut meta = txn.open_table(def(TableId::Meta)).unwrap();
                meta.insert(
                    meta_keys::SCHEMA_VERSION.as_bytes(),
                    SCHEMA_VERSION.to_be_bytes().as_slice(),
                )
                .unwrap();
            }
            txn.commit().unwrap();
        }
        let db = RedbDatabase::open(&path).unwrap();
        assert_eq!(db.get::<CommitCertificates>(&1).unwrap(), None);
        roundtrip(&db);
        assert_eq!(SCHEMA_VERSION, 1);
    }
}

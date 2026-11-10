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
//!
//! `Transactions` stores only the location; the transaction itself lives in `Bodies`
//! (saves disk vs. storing it twice). The history tables hold reverse diffs for the
//! last N blocks (§14.4) so state can be read at recent past blocks; the
//! `StateChangeSets` index lets old entries be pruned block by block. `Commits`,
//! `TrieNodes` and `LogIndex` arrive in later phases.

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
}

impl TableId {
    /// Every table, in a fixed order (backends create them all on open).
    pub const ALL: [TableId; 12] = [
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

/// The state keys changed by one block: the keys of its [`AccountHistory`] and
/// [`StorageHistory`] entries, kept so they can be pruned without a table scan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateChangeSet {
    /// Changed accounts.
    pub accounts: Vec<Address>,
    /// Changed storage slots.
    pub slots: Vec<(Address, B256)>,
}

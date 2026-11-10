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
//!
//! `Transactions` stores only the location; the transaction itself lives in `Bodies`
//! (saves disk vs. storing it twice). `Commits`, `TrieNodes`, history tables and
//! `LogIndex` arrive in later phases.

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
}

impl TableId {
    /// Every table, in a fixed order (backends create them all on open).
    pub const ALL: [TableId; 9] = [
        TableId::Headers,
        TableId::HeaderNumbers,
        TableId::Bodies,
        TableId::Transactions,
        TableId::Receipts,
        TableId::PlainAccounts,
        TableId::PlainStorage,
        TableId::Bytecodes,
        TableId::Meta,
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

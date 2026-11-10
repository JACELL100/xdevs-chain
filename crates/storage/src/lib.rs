//! Persistent storage for xDEVS Chain (plan §7.3, §14).
//!
//! * [`Database`] — object-safe key-value interface over byte keys/values, grouped
//!   in fixed [`TableId`]s, with **atomic** [`WriteBatch`] commits (§14.2) and
//!   consistent read snapshots.
//! * [`tables`] — typed table definitions (§14.1) with their key/value codecs;
//!   use them through [`DbReadExt`] and [`WriteBatch::put`].
//! * [`MemDb`] — complete in-memory implementation for tests.
//! * [`RedbDatabase`] — the on-disk `redb` backend used by the node.
//!
//! Keys are encoded so that byte order equals logical order (big-endian integers),
//! which makes range walks (`walk`) iterate blocks/slots in ascending order.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod batch;
mod codec;
mod error;
mod mem;
mod redb_db;
pub mod tables;

pub use batch::{WriteBatch, WriteOp};
pub use codec::{Decode, Encode};
pub use error::DbError;
pub use mem::MemDb;
pub use redb_db::{RedbDatabase, RedbSnapshot, DB_FILE_NAME};
pub use tables::{Table, TableId};

/// Read access to a consistent view of the database.
pub trait DbRead {
    /// Raw get: value stored under `key` in `table`.
    fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError>;

    /// Visit every entry of `table` with `key >= start`, in ascending key order, until
    /// `visit` returns `false` or the table ends.
    fn walk_raw(
        &self,
        table: TableId,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), DbError>;
}

/// Persistent key-value storage with atomic write batches (plan §7.3).
///
/// Reads through the `DbRead` supertrait observe the latest committed batch; use
/// [`Database::snapshot`] when several reads must see the same state.
pub trait Database: DbRead + Send + Sync + 'static {
    /// A read-only view frozen at the latest committed batch. It is `Send + Sync`
    /// so it can be held across `.await` points or moved to another thread.
    fn snapshot(&self) -> Result<Box<dyn DbRead + Send + Sync + '_>, DbError>;

    /// Apply all operations of `batch` atomically, in order: after a crash either all
    /// of them or none of them are visible.
    fn write(&self, batch: WriteBatch) -> Result<(), DbError>;
}

/// Decoded `(key, value)` pairs of table `T`, in key order.
pub type Entries<T> = Vec<(<T as Table>::Key, <T as Table>::Value)>;

/// Typed read helpers, available on every [`DbRead`] (including `dyn DbRead`).
pub trait DbReadExt: DbRead {
    /// Typed get.
    fn get<T: Table>(&self, key: &T::Key) -> Result<Option<T::Value>, DbError> {
        match self.get_raw(T::ID, &key.encode())? {
            Some(bytes) => T::Value::decode(&bytes).map(Some),
            None => Ok(None),
        }
    }

    /// Typed walk from `start` (inclusive) in ascending key order; stops when `visit`
    /// returns `Ok(false)` or an error.
    fn walk<T: Table>(
        &self,
        start: &T::Key,
        mut visit: impl FnMut(T::Key, T::Value) -> Result<bool, DbError>,
    ) -> Result<(), DbError> {
        let mut err = None;
        self.walk_raw(T::ID, &start.encode(), &mut |k, v| {
            let res = T::Key::decode(k).and_then(|k| Ok((k, T::Value::decode(v)?)));
            match res.and_then(|(k, v)| visit(k, v)) {
                Ok(cont) => cont,
                Err(e) => {
                    err = Some(e);
                    false
                }
            }
        })?;
        err.map_or(Ok(()), Err)
    }

    /// All entries whose encoded key starts with `prefix` (e.g. all storage slots of
    /// one address in [`tables::PlainStorage`]).
    fn collect_prefix<T: Table>(&self, prefix: &[u8]) -> Result<Entries<T>, DbError> {
        let mut out = Vec::new();
        let mut err = None;
        self.walk_raw(T::ID, prefix, &mut |k, v| {
            if !k.starts_with(prefix) {
                return false;
            }
            match T::Key::decode(k).and_then(|k| Ok((k, T::Value::decode(v)?))) {
                Ok(kv) => {
                    out.push(kv);
                    true
                }
                Err(e) => {
                    err = Some(e);
                    false
                }
            }
        })?;
        err.map_or(Ok(out), Err)
    }
}

impl<D: DbRead + ?Sized> DbReadExt for D {}

/// Well-known keys of the [`tables::Meta`] table.
pub mod meta_keys {
    /// Latest committed block number (`u64`, big-endian).
    pub const LATEST_BLOCK: &str = "latest_block";
    /// Chain config as JSON (`ChainConfig`).
    pub const CHAIN_CONFIG: &str = "chain_config";
    /// Genesis block hash (32 bytes) — verified on every start.
    pub const GENESIS_HASH: &str = "genesis_hash";
    /// On-disk schema version (`u64`, big-endian), see [`super::SCHEMA_VERSION`].
    pub const SCHEMA_VERSION: &str = "schema_version";
}

/// Current on-disk schema version. Bump when a table encoding changes.
pub const SCHEMA_VERSION: u64 = 1;

macro_rules! forward_read {
    ($($ty:ty),*) => {$(
        impl<T: DbRead + ?Sized> DbRead for $ty {
            fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
                (**self).get_raw(table, key)
            }
            fn walk_raw(
                &self,
                table: TableId,
                start: &[u8],
                visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
            ) -> Result<(), DbError> {
                (**self).walk_raw(table, start, visit)
            }
        }
    )*};
}
forward_read!(&T, Box<T>, std::sync::Arc<T>);

//! Atomic write batches (plan §14.2).

use crate::{codec::Encode, tables::Table, TableId};

/// One write operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteOp {
    /// Insert or overwrite `key`.
    Put {
        /// Target table.
        table: TableId,
        /// Encoded key.
        key: Vec<u8>,
        /// Encoded value.
        value: Vec<u8>,
    },
    /// Remove `key` (no-op if absent).
    Delete {
        /// Target table.
        table: TableId,
        /// Encoded key.
        key: Vec<u8>,
    },
    /// Remove every key starting with `prefix` (e.g. wipe an account's storage).
    DeletePrefix {
        /// Target table.
        table: TableId,
        /// Encoded key prefix.
        prefix: Vec<u8>,
    },
}

/// An ordered list of writes committed atomically by [`crate::Database::write`].
///
/// A committed block's header, body, receipts, tx index, state changes and meta
/// updates must all go into **one** batch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<WriteOp>,
}

impl WriteBatch {
    /// Empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Typed put.
    pub fn put<T: Table>(&mut self, key: &T::Key, value: &T::Value) -> &mut Self {
        self.put_raw(T::ID, key.encode(), value.encode())
    }

    /// Typed delete.
    pub fn delete<T: Table>(&mut self, key: &T::Key) -> &mut Self {
        self.delete_raw(T::ID, key.encode())
    }

    /// Raw put.
    pub fn put_raw(&mut self, table: TableId, key: Vec<u8>, value: Vec<u8>) -> &mut Self {
        self.ops.push(WriteOp::Put { table, key, value });
        self
    }

    /// Raw delete.
    pub fn delete_raw(&mut self, table: TableId, key: Vec<u8>) -> &mut Self {
        self.ops.push(WriteOp::Delete { table, key });
        self
    }

    /// Delete every key of `table` starting with `prefix`.
    pub fn delete_prefix(&mut self, table: TableId, prefix: Vec<u8>) -> &mut Self {
        self.ops.push(WriteOp::DeletePrefix { table, prefix });
        self
    }

    /// Append all operations of `other` after ours.
    pub fn extend(&mut self, other: WriteBatch) -> &mut Self {
        self.ops.extend(other.ops);
        self
    }

    /// Operations in application order.
    pub fn ops(&self) -> &[WriteOp] {
        &self.ops
    }

    /// Consume into operations.
    pub fn into_ops(self) -> Vec<WriteOp> {
        self.ops
    }

    /// Number of operations.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// `true` if there are no operations.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

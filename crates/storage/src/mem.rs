//! In-memory [`Database`] for tests and ephemeral dev nodes.

use crate::{Database, DbError, DbRead, TableId, WriteBatch, WriteOp};
use parking_lot::RwLock;
use std::{collections::BTreeMap, ops::Bound, sync::Arc};

type Tables = BTreeMap<TableId, BTreeMap<Vec<u8>, Vec<u8>>>;

/// A complete, ordered, in-memory database. Snapshots are cheap (`Arc` clone);
/// writes copy-on-write if a snapshot is still alive. Batches are atomic.
#[derive(Debug, Default, Clone)]
pub struct MemDb {
    inner: Arc<RwLock<Arc<Tables>>>,
}

impl MemDb {
    /// Empty database.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Frozen view of a [`MemDb`].
#[derive(Debug, Clone)]
pub struct MemSnapshot(Arc<Tables>);

fn get(tables: &Tables, table: TableId, key: &[u8]) -> Option<Vec<u8>> {
    tables.get(&table).and_then(|t| t.get(key)).cloned()
}

fn walk(
    tables: &Tables,
    table: TableId,
    start: &[u8],
    visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
) {
    if let Some(t) = tables.get(&table) {
        for (k, v) in t.range::<[u8], _>((Bound::Included(start), Bound::Unbounded)) {
            if !visit(k, v) {
                break;
            }
        }
    }
}

impl DbRead for MemSnapshot {
    fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(get(&self.0, table, key))
    }

    fn walk_raw(
        &self,
        table: TableId,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), DbError> {
        walk(&self.0, table, start, visit);
        Ok(())
    }
}

impl DbRead for MemDb {
    fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(get(&self.inner.read(), table, key))
    }

    fn walk_raw(
        &self,
        table: TableId,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), DbError> {
        // Walk a snapshot so `visit` never runs while the lock is held.
        let snap = self.inner.read().clone();
        walk(&snap, table, start, visit);
        Ok(())
    }
}

impl Database for MemDb {
    fn snapshot(&self) -> Result<Box<dyn DbRead + Send + Sync + '_>, DbError> {
        Ok(Box::new(MemSnapshot(self.inner.read().clone())))
    }

    fn write(&self, batch: WriteBatch) -> Result<(), DbError> {
        let mut guard = self.inner.write();
        // Build the new state on the side, then swap: readers see all-or-nothing.
        let mut next: Tables = (**guard).clone();
        for op in batch.into_ops() {
            match op {
                WriteOp::Put { table, key, value } => {
                    next.entry(table).or_default().insert(key, value);
                }
                WriteOp::Delete { table, key } => {
                    if let Some(t) = next.get_mut(&table) {
                        t.remove(&key);
                    }
                }
                WriteOp::DeletePrefix { table, prefix } => {
                    if let Some(t) = next.get_mut(&table) {
                        t.retain(|k, _| !k.starts_with(&prefix));
                    }
                }
            }
        }
        *guard = Arc::new(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tables, DbReadExt};
    use xdevs_primitives::{AccountInfo, Address, B256, U256};

    #[test]
    fn batch_put_get_delete() {
        let db = MemDb::new();
        let a = Address::repeat_byte(1);
        let mut b = WriteBatch::new();
        b.put::<tables::PlainAccounts>(&a, &AccountInfo::new(1, U256::from(5u64)));
        b.put::<tables::HeaderNumbers>(&B256::repeat_byte(9), &7);
        db.write(b).unwrap();
        assert_eq!(
            db.get::<tables::PlainAccounts>(&a).unwrap().unwrap().nonce,
            1
        );
        assert_eq!(
            db.get::<tables::HeaderNumbers>(&B256::repeat_byte(9))
                .unwrap(),
            Some(7)
        );

        let mut b = WriteBatch::new();
        b.delete::<tables::PlainAccounts>(&a);
        db.write(b).unwrap();
        assert_eq!(db.get::<tables::PlainAccounts>(&a).unwrap(), None);
    }

    #[test]
    fn snapshot_is_isolated() {
        let db = MemDb::new();
        let mut b = WriteBatch::new();
        b.put::<tables::Meta>(&"k".to_string(), &vec![1]);
        db.write(b).unwrap();
        let snap = db.snapshot().unwrap();
        let mut b = WriteBatch::new();
        b.put::<tables::Meta>(&"k".to_string(), &vec![2]);
        db.write(b).unwrap();
        assert_eq!(
            snap.get::<tables::Meta>(&"k".to_string()).unwrap(),
            Some(vec![1])
        );
        assert_eq!(
            db.get::<tables::Meta>(&"k".to_string()).unwrap(),
            Some(vec![2])
        );
    }

    #[test]
    fn prefix_walk_and_delete() {
        let db = MemDb::new();
        let (a1, a2) = (Address::repeat_byte(1), Address::repeat_byte(2));
        let mut b = WriteBatch::new();
        for i in 1..=3u8 {
            b.put::<tables::PlainStorage>(&(a1, B256::repeat_byte(i)), &U256::from(i));
            b.put::<tables::PlainStorage>(&(a2, B256::repeat_byte(i)), &U256::from(i));
        }
        db.write(b).unwrap();
        let slots = db
            .collect_prefix::<tables::PlainStorage>(a1.as_slice())
            .unwrap();
        assert_eq!(slots.len(), 3);
        assert!(slots.windows(2).all(|w| w[0].0 < w[1].0));

        let mut b = WriteBatch::new();
        b.delete_prefix(TableId::PlainStorage, a1.to_vec());
        db.write(b).unwrap();
        assert!(db
            .collect_prefix::<tables::PlainStorage>(a1.as_slice())
            .unwrap()
            .is_empty());
        assert_eq!(
            db.collect_prefix::<tables::PlainStorage>(a2.as_slice())
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn typed_walk_in_order() {
        let db = MemDb::new();
        let mut b = WriteBatch::new();
        for n in [300u64, 2, 256, 1] {
            b.put::<tables::HeaderNumbers>(&B256::with_last_byte(n as u8), &n);
            b.put::<tables::Meta>(&format!("{n:05}"), &n.to_be_bytes().to_vec());
        }
        db.write(b).unwrap();
        let mut seen = vec![];
        db.walk::<tables::Meta>(&"00002".to_string(), |k, _| {
            seen.push(k);
            Ok(true)
        })
        .unwrap();
        assert_eq!(seen, vec!["00002", "00256", "00300"]);
    }
}

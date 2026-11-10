//! On-disk backend built on `redb` (plan ADR-0005).
//!
//! Layout: one redb table per [`TableId`] (named [`TableId::name`]) with `&[u8]`
//! keys and values (redb orders `&[u8]` keys lexicographically, so the big-endian key
//! encodings walk in logical order). [`Database::write`] maps to a single redb write
//! transaction with immediate durability, so a whole block commit (§14.2) is atomic
//! and crash-safe; [`Database::snapshot`] maps to a redb read transaction (MVCC: it
//! never blocks writers and never sees later commits).

use crate::{meta_keys, Database, DbError, DbRead, TableId, WriteBatch, WriteOp, SCHEMA_VERSION};
use redb::{ReadOnlyTable, ReadableDatabase, ReadableTable, TableDefinition};
use std::{
    fmt::Display,
    ops::Bound,
    path::{Path, PathBuf},
};

/// File name of the database inside a data directory (see [`RedbDatabase::open_dir`]).
pub const DB_FILE_NAME: &str = "xdevs.redb";

type RawTable = TableDefinition<'static, &'static [u8], &'static [u8]>;
type ReadTable = ReadOnlyTable<&'static [u8], &'static [u8]>;
type WriteTable<'txn> = redb::Table<'txn, &'static [u8], &'static [u8]>;

const fn def(table: TableId) -> RawTable {
    TableDefinition::new(table.name())
}

fn backend(e: impl Display) -> DbError {
    DbError::Backend(e.to_string())
}

/// Smallest byte string greater than every string starting with `prefix`
/// (`None` = no upper bound, i.e. `prefix` is empty or all `0xff`).
fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < u8::MAX {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

/// `redb`-backed [`Database`].
pub struct RedbDatabase {
    db: redb::Database,
    path: PathBuf,
}

impl std::fmt::Debug for RedbDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbDatabase")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl RedbDatabase {
    /// Open (or create) the database file at `path`, creating all tables and
    /// checking/storing [`crate::SCHEMA_VERSION`] in `Meta`.
    ///
    /// If `path` is an existing directory, this is [`RedbDatabase::open_dir`].
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if path.is_dir() {
            return Self::open_dir(path);
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(backend)?;
        }
        let db = redb::Database::create(path).map_err(backend)?;
        let this = Self {
            db,
            path: path.to_path_buf(),
        };
        this.init_schema()?;
        Ok(this)
    }

    /// Open (or create) the database `<dir>/xdevs.redb`, creating `dir` if needed.
    pub fn open_dir(dir: &Path) -> Result<Self, DbError> {
        std::fs::create_dir_all(dir).map_err(backend)?;
        Self::open(&dir.join(DB_FILE_NAME))
    }

    /// Path of the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create missing tables and verify (or, on a fresh database, store) the schema version.
    fn init_schema(&self) -> Result<(), DbError> {
        let txn = self.db.begin_write().map_err(backend)?;
        {
            for table in TableId::ALL {
                txn.open_table(def(table)).map_err(backend)?;
            }
            let mut meta = txn.open_table(def(TableId::Meta)).map_err(backend)?;
            let key = meta_keys::SCHEMA_VERSION.as_bytes();
            let found = meta
                .get(key)
                .map_err(backend)?
                .map(|v| decode_version(v.value()))
                .transpose()?;
            match found {
                Some(found) if found != SCHEMA_VERSION => {
                    return Err(DbError::SchemaMismatch {
                        found,
                        expected: SCHEMA_VERSION,
                    });
                }
                Some(_) => {}
                None => {
                    meta.insert(key, SCHEMA_VERSION.to_be_bytes().as_slice())
                        .map_err(backend)?;
                }
            }
        }
        txn.commit().map_err(backend)
    }

    /// Apply `batch` in one write transaction. With `fail_after = Some(n)` the
    /// transaction is aborted with an error after `n` operations (atomicity tests).
    fn write_inner(&self, batch: WriteBatch, fail_after: Option<usize>) -> Result<(), DbError> {
        let txn = self.db.begin_write().map_err(backend)?;
        {
            // Open each table at most once per transaction.
            let mut open: Vec<(TableId, WriteTable<'_>)> = Vec::new();
            for (i, op) in batch.into_ops().into_iter().enumerate() {
                if fail_after == Some(i) {
                    return Err(DbError::Backend(format!("injected failure after {i} ops")));
                }
                let id = match &op {
                    WriteOp::Put { table, .. }
                    | WriteOp::Delete { table, .. }
                    | WriteOp::DeletePrefix { table, .. } => *table,
                };
                let pos = match open.iter().position(|(t, _)| *t == id) {
                    Some(pos) => pos,
                    None => {
                        open.push((id, txn.open_table(def(id)).map_err(backend)?));
                        open.len() - 1
                    }
                };
                let table = &mut open[pos].1;
                match op {
                    WriteOp::Put { key, value, .. } => {
                        table
                            .insert(key.as_slice(), value.as_slice())
                            .map_err(backend)?;
                    }
                    WriteOp::Delete { key, .. } => {
                        table.remove(key.as_slice()).map_err(backend)?;
                    }
                    WriteOp::DeletePrefix { prefix, .. } => {
                        let upper = prefix_end(&prefix);
                        let range = (
                            Bound::Included(prefix.as_slice()),
                            upper.as_deref().map_or(Bound::Unbounded, Bound::Excluded),
                        );
                        table
                            .retain_in::<&[u8], _>(range, |_, _| false)
                            .map_err(backend)?;
                    }
                }
            }
        }
        // Dropping `txn` without commit (early return above) aborts it: nothing is written.
        txn.commit().map_err(backend)
    }

    #[cfg(test)]
    pub(crate) fn write_failing_after(&self, batch: WriteBatch, n: usize) -> Result<(), DbError> {
        self.write_inner(batch, Some(n))
    }
}

fn decode_version(bytes: &[u8]) -> Result<u64, DbError> {
    let arr: [u8; 8] = bytes
        .try_into()
        .map_err(|_| DbError::Decode("schema version: expected 8 bytes".into()))?;
    Ok(u64::from_be_bytes(arr))
}

/// A frozen read view: one redb read transaction with every table opened.
pub struct RedbSnapshot {
    tables: Vec<(TableId, ReadTable)>,
}

impl std::fmt::Debug for RedbSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbSnapshot").finish_non_exhaustive()
    }
}

impl RedbSnapshot {
    fn new(db: &redb::Database) -> Result<Self, DbError> {
        let txn = db.begin_read().map_err(backend)?;
        let tables = TableId::ALL
            .iter()
            .map(|&t| Ok((t, txn.open_table(def(t)).map_err(backend)?)))
            .collect::<Result<_, DbError>>()?;
        Ok(Self { tables })
    }

    fn table(&self, id: TableId) -> &ReadTable {
        &self
            .tables
            .iter()
            .find(|(t, _)| *t == id)
            .expect("every TableId is opened")
            .1
    }
}

impl DbRead for RedbSnapshot {
    fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self
            .table(table)
            .get(key)
            .map_err(backend)?
            .map(|v| v.value().to_vec()))
    }

    fn walk_raw(
        &self,
        table: TableId,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), DbError> {
        for entry in self.table(table).range(start..).map_err(backend)? {
            let (k, v) = entry.map_err(backend)?;
            if !visit(k.value(), v.value()) {
                break;
            }
        }
        Ok(())
    }
}

impl DbRead for RedbDatabase {
    fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(def(table)).map_err(backend)?;
        Ok(t.get(key).map_err(backend)?.map(|v| v.value().to_vec()))
    }

    fn walk_raw(
        &self,
        table: TableId,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), DbError> {
        // A read transaction is a consistent snapshot and never blocks writers.
        let txn = self.db.begin_read().map_err(backend)?;
        let t = txn.open_table(def(table)).map_err(backend)?;
        for entry in t.range(start..).map_err(backend)? {
            let (k, v) = entry.map_err(backend)?;
            if !visit(k.value(), v.value()) {
                break;
            }
        }
        Ok(())
    }
}

impl Database for RedbDatabase {
    fn snapshot(&self) -> Result<Box<dyn DbRead + Send + Sync + '_>, DbError> {
        Ok(Box::new(RedbSnapshot::new(&self.db)?))
    }

    fn write(&self, batch: WriteBatch) -> Result<(), DbError> {
        self.write_inner(batch, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tables, DbReadExt, MemDb};
    use xdevs_primitives::{AccountInfo, Address, Bytes, Header, B256, U256};

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn prefix_end_cases() {
        assert_eq!(prefix_end(&[1, 2]), Some(vec![1, 3]));
        assert_eq!(prefix_end(&[1, 0xff]), Some(vec![2]));
        assert_eq!(prefix_end(&[0xff, 0xff]), None);
        assert_eq!(prefix_end(&[]), None);
    }

    /// Behaviour every backend must share; run on both `MemDb` and `RedbDatabase`.
    fn conformance(db: &dyn Database) {
        let a1 = Address::repeat_byte(1);
        let a2 = Address::repeat_byte(2);
        let aff = Address::repeat_byte(0xff);
        let mut b = WriteBatch::new();
        b.put::<tables::Headers>(
            &5,
            &Header {
                number: 5,
                ..Default::default()
            },
        );
        b.put::<tables::HeaderNumbers>(&B256::repeat_byte(5), &5);
        b.put::<tables::PlainAccounts>(&a1, &AccountInfo::new(1, U256::from(10u64)));
        b.put::<tables::Bytecodes>(&B256::repeat_byte(7), &Bytes::from_static(&[0x60]));
        for i in 1..=3u8 {
            for a in [a1, a2, aff] {
                b.put::<tables::PlainStorage>(&(a, B256::repeat_byte(i)), &U256::from(i));
            }
        }
        // Later ops in the same batch win.
        b.put::<tables::Meta>(&"k".to_string(), &vec![1]);
        b.put::<tables::Meta>(&"k".to_string(), &vec![2]);
        b.put::<tables::Meta>(&"gone".to_string(), &vec![9]);
        b.delete::<tables::Meta>(&"gone".to_string());
        db.write(b).unwrap();

        assert_eq!(db.get::<tables::Headers>(&5).unwrap().unwrap().number, 5);
        assert_eq!(db.get::<tables::Headers>(&6).unwrap(), None);
        assert_eq!(
            db.get::<tables::HeaderNumbers>(&B256::repeat_byte(5))
                .unwrap(),
            Some(5)
        );
        assert_eq!(
            db.get::<tables::PlainAccounts>(&a1).unwrap(),
            Some(AccountInfo::new(1, U256::from(10u64)))
        );
        assert_eq!(
            db.get::<tables::Bytecodes>(&B256::repeat_byte(7)).unwrap(),
            Some(Bytes::from_static(&[0x60]))
        );
        assert_eq!(
            db.get::<tables::Meta>(&"k".to_string()).unwrap(),
            Some(vec![2])
        );
        assert_eq!(db.get::<tables::Meta>(&"gone".to_string()).unwrap(), None);

        // Ordered walk with early stop.
        let mut seen = vec![];
        db.walk::<tables::PlainStorage>(&(a2, B256::ZERO), |k, _| {
            seen.push(k);
            Ok(seen.len() < 4)
        })
        .unwrap();
        assert_eq!(seen.len(), 4);
        assert!(seen[..3].iter().all(|(a, _)| *a == a2));
        assert_eq!(seen[3].0, aff);
        assert!(seen.windows(2).all(|w| w[0] < w[1]));

        // Prefix delete, including the all-0xff prefix (no upper bound).
        let mut b = WriteBatch::new();
        b.delete_prefix(TableId::PlainStorage, a1.to_vec());
        b.delete_prefix(TableId::PlainStorage, aff.to_vec());
        db.write(b).unwrap();
        let count = |a: Address| {
            db.collect_prefix::<tables::PlainStorage>(a.as_slice())
                .unwrap()
                .len()
        };
        assert_eq!((count(a1), count(a2), count(aff)), (0, 3, 0));

        // Snapshot isolation.
        let snap = db.snapshot().unwrap();
        let mut b = WriteBatch::new();
        b.put::<tables::Meta>(&"k".to_string(), &vec![3]);
        b.delete::<tables::PlainAccounts>(&a1);
        db.write(b).unwrap();
        assert_eq!(
            snap.get::<tables::Meta>(&"k".to_string()).unwrap(),
            Some(vec![2])
        );
        assert!(snap.get::<tables::PlainAccounts>(&a1).unwrap().is_some());
        assert_eq!(
            snap.collect_prefix::<tables::PlainStorage>(a2.as_slice())
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            db.get::<tables::Meta>(&"k".to_string()).unwrap(),
            Some(vec![3])
        );
        assert_eq!(db.get::<tables::PlainAccounts>(&a1).unwrap(), None);
    }

    #[test]
    fn conformance_memdb() {
        conformance(&MemDb::new());
    }

    #[test]
    fn conformance_redb() {
        let dir = tmp();
        conformance(&RedbDatabase::open_dir(dir.path()).unwrap());
    }

    #[test]
    fn schema_version_written_and_checked() {
        let dir = tmp();
        let path = dir.path().join("db.redb");
        {
            let db = RedbDatabase::open(&path).unwrap();
            assert_eq!(
                db.get::<tables::Meta>(&meta_keys::SCHEMA_VERSION.to_string())
                    .unwrap(),
                Some(SCHEMA_VERSION.to_be_bytes().to_vec())
            );
            let mut b = WriteBatch::new();
            b.put::<tables::Meta>(
                &meta_keys::SCHEMA_VERSION.to_string(),
                &(SCHEMA_VERSION + 1).to_be_bytes().to_vec(),
            );
            db.write(b).unwrap();
        }
        match RedbDatabase::open(&path) {
            Err(DbError::SchemaMismatch { found, expected }) => {
                assert_eq!((found, expected), (SCHEMA_VERSION + 1, SCHEMA_VERSION));
            }
            other => panic!("expected schema mismatch, got {other:?}"),
        }
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tmp();
        let data_dir = dir.path().join("nested").join("data");
        let a = Address::repeat_byte(0xab);
        {
            let db = RedbDatabase::open_dir(&data_dir).unwrap();
            let mut b = WriteBatch::new();
            b.put::<tables::PlainAccounts>(&a, &AccountInfo::new(3, U256::from(99u64)));
            b.put::<tables::Meta>(
                &meta_keys::LATEST_BLOCK.to_string(),
                &7u64.to_be_bytes().to_vec(),
            );
            db.write(b).unwrap();
        }
        assert!(data_dir.join(DB_FILE_NAME).is_file());
        // `open` on the directory resolves to the same file.
        let db = RedbDatabase::open(&data_dir).unwrap();
        assert_eq!(
            db.get::<tables::PlainAccounts>(&a).unwrap(),
            Some(AccountInfo::new(3, U256::from(99u64)))
        );
        assert_eq!(
            db.get::<tables::Meta>(&meta_keys::LATEST_BLOCK.to_string())
                .unwrap(),
            Some(7u64.to_be_bytes().to_vec())
        );
    }

    #[test]
    fn failed_batch_writes_nothing() {
        let dir = tmp();
        let path = dir.path().join("db.redb");
        let a = Address::repeat_byte(1);
        {
            let db = RedbDatabase::open(&path).unwrap();
            let mut base = WriteBatch::new();
            base.put::<tables::Meta>(&"k".to_string(), &vec![1]);
            base.put::<tables::PlainStorage>(&(a, B256::ZERO), &U256::from(1u64));
            db.write(base).unwrap();

            // A multi-table "block" batch that fails after 4 of its 6 ops.
            let mut b = WriteBatch::new();
            b.put::<tables::Headers>(&1, &Header::default());
            b.put::<tables::HeaderNumbers>(&B256::repeat_byte(1), &1);
            b.put::<tables::Meta>(&"k".to_string(), &vec![2]);
            b.delete_prefix(TableId::PlainStorage, a.to_vec());
            b.put::<tables::PlainAccounts>(&a, &AccountInfo::default());
            b.put::<tables::Meta>(&"latest".to_string(), &vec![1]);
            assert!(db.write_failing_after(b, 4).is_err());

            let check = |db: &RedbDatabase| {
                assert_eq!(db.get::<tables::Headers>(&1).unwrap(), None);
                assert_eq!(
                    db.get::<tables::HeaderNumbers>(&B256::repeat_byte(1))
                        .unwrap(),
                    None
                );
                assert_eq!(
                    db.get::<tables::Meta>(&"k".to_string()).unwrap(),
                    Some(vec![1])
                );
                assert_eq!(
                    db.get::<tables::PlainStorage>(&(a, B256::ZERO)).unwrap(),
                    Some(U256::from(1u64))
                );
            };
            check(&db);
            // The database stays usable after an aborted batch.
            let mut b = WriteBatch::new();
            b.put::<tables::Meta>(&"after".to_string(), &vec![1]);
            db.write(b).unwrap();
            check(&db);
        }
        let db = RedbDatabase::open(&path).unwrap();
        assert_eq!(db.get::<tables::Headers>(&1).unwrap(), None);
        assert_eq!(
            db.get::<tables::Meta>(&"after".to_string()).unwrap(),
            Some(vec![1])
        );
    }

    #[test]
    fn database_and_snapshot_are_thread_safe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RedbDatabase>();
        assert_send_sync::<RedbSnapshot>();
        let dir = tmp();
        let db = std::sync::Arc::new(RedbDatabase::open_dir(dir.path()).unwrap());
        let handles: Vec<_> = (0..4u64)
            .map(|i| {
                let db = db.clone();
                std::thread::spawn(move || {
                    let mut b = WriteBatch::new();
                    b.put::<tables::Headers>(
                        &i,
                        &Header {
                            number: i,
                            ..Default::default()
                        },
                    );
                    db.write(b).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut n = 0;
        db.walk::<tables::Headers>(&0, |k, h| {
            assert_eq!(k, h.number);
            n += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(n, 4);
    }
}

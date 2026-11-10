//! State backed by the flat storage tables.
//!
//! Phase-1 implementation: reads go straight to `PlainAccounts` / `PlainStorage` /
//! `Bytecodes`; the state root is recomputed from scratch with one ordered pass over
//! the flat tables ([`crate::root::db_state_root`], plan §14.3). Phase 2 can make it
//! incremental without changing the interface.

use crate::{InMemoryState, StateDiff, StateError, StateProvider};
use xdevs_primitives::{AccountInfo, Address, Bytes, B256, KECCAK256_EMPTY, U256};
use xdevs_storage::{tables, DbRead, DbReadExt, TableId, WriteBatch};

/// [`StateProvider`] over the latest state stored in a database (or a snapshot of it).
#[derive(Debug, Clone)]
pub struct DbStateProvider<D> {
    db: D,
}

impl<D: DbRead> DbStateProvider<D> {
    /// Wrap a database handle or snapshot (`&MemDb`, `Box<dyn DbRead>`, ...).
    pub const fn new(db: D) -> Self {
        Self { db }
    }

    /// The wrapped handle.
    pub const fn inner(&self) -> &D {
        &self.db
    }

    /// Load the complete flat state into memory (accounts, storage, code).
    pub fn load_all(&self) -> Result<InMemoryState, StateError> {
        let mut diff = StateDiff::new();
        self.db
            .walk::<tables::PlainAccounts>(&Address::ZERO, |a, info| {
                diff.accounts.insert(a, Some(info));
                Ok(true)
            })?;
        self.db
            .walk::<tables::PlainStorage>(&(Address::ZERO, B256::ZERO), |(a, slot), v| {
                diff.storage.entry(a).or_default().slots.insert(slot, v);
                Ok(true)
            })?;
        self.db.walk::<tables::Bytecodes>(&B256::ZERO, |h, code| {
            diff.codes.insert(h, code);
            Ok(true)
        })?;
        Ok(InMemoryState::from_diff(&diff))
    }
}

impl<D: DbRead> StateProvider for DbStateProvider<D> {
    fn basic_account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        Ok(self.db.get::<tables::PlainAccounts>(&address)?)
    }

    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        Ok(self
            .db
            .get::<tables::PlainStorage>(&(address, slot))?
            .unwrap_or_default())
    }

    fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytes>, StateError> {
        if code_hash == KECCAK256_EMPTY {
            return Ok(Some(Bytes::new()));
        }
        Ok(self.db.get::<tables::Bytecodes>(&code_hash)?)
    }

    fn block_hash(&self, number: u64) -> Result<Option<B256>, StateError> {
        Ok(self
            .db
            .get::<tables::Headers>(&number)?
            .map(|h| h.hash_slow()))
    }

    fn state_root_with(&self, diff: &StateDiff) -> Result<B256, StateError> {
        Ok(crate::root::db_state_root(&self.db, diff)?)
    }
}

/// Append the writes for `diff` to `batch` (same semantics as [`InMemoryState::apply`]).
pub fn write_state_diff(batch: &mut WriteBatch, diff: &StateDiff) {
    for (address, change) in &diff.accounts {
        match change {
            Some(info) => {
                batch.put::<tables::PlainAccounts>(address, info);
            }
            None => {
                batch.delete::<tables::PlainAccounts>(address);
                batch.delete_prefix(TableId::PlainStorage, address.to_vec());
            }
        }
    }
    for (address, sd) in &diff.storage {
        if matches!(diff.accounts.get(address), Some(None)) {
            continue;
        }
        if sd.wiped {
            batch.delete_prefix(TableId::PlainStorage, address.to_vec());
        }
        for (slot, value) in &sd.slots {
            if value.is_zero() {
                batch.delete::<tables::PlainStorage>(&(*address, *slot));
            } else {
                batch.put::<tables::PlainStorage>(&(*address, *slot), value);
            }
        }
    }
    for (hash, code) in &diff.codes {
        batch.put::<tables::Bytecodes>(hash, code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::tests::Rng;
    use xdevs_storage::{Database, MemDb, RedbDatabase};

    #[test]
    fn db_state_matches_in_memory_state() {
        let db = MemDb::new();
        let mut mem = InMemoryState::new();
        let a = Address::repeat_byte(1);
        let c = Address::repeat_byte(2);

        let mut d = StateDiff::new();
        d.set_account(a, AccountInfo::new(0, U256::from(1_000u64)));
        let ch = d.insert_code(Bytes::from_static(&[0x00]));
        d.set_account(
            c,
            AccountInfo {
                nonce: 1,
                balance: U256::ZERO,
                code_hash: ch,
            },
        );
        d.set_storage(c, B256::with_last_byte(1), U256::from(1u64));
        d.set_storage(c, B256::with_last_byte(2), U256::from(2u64));

        let mut batch = WriteBatch::new();
        write_state_diff(&mut batch, &d);
        db.write(batch).unwrap();
        mem.apply(&d);

        let p = DbStateProvider::new(&db);
        assert_eq!(p.basic_account(a).unwrap(), mem.basic_account(a).unwrap());
        assert_eq!(
            p.storage(c, B256::with_last_byte(2)).unwrap(),
            U256::from(2u64)
        );
        assert_eq!(p.code(c).unwrap(), Bytes::from_static(&[0x00]));
        assert_eq!(
            p.state_root_with(&StateDiff::new()).unwrap(),
            mem.state_root()
        );

        // Destroy + zero-write, through both paths.
        let mut d2 = StateDiff::new();
        d2.destroy_account(c);
        d2.set_account(a, AccountInfo::new(1, U256::from(10u64)));
        assert_eq!(
            p.state_root_with(&d2).unwrap(),
            mem.state_root_with(&d2).unwrap()
        );
        let mut batch = WriteBatch::new();
        write_state_diff(&mut batch, &d2);
        db.write(batch).unwrap();
        mem.apply(&d2);
        assert_eq!(p.storage(c, B256::with_last_byte(1)).unwrap(), U256::ZERO);
        assert_eq!(p.load_all().unwrap(), {
            // block hashes are not part of the flat tables
            mem.clone()
        });
    }

    /// A random diff over a small address pool (so accounts get updated, destroyed,
    /// recreated and wiped across steps).
    fn random_diff(rng: &mut Rng) -> StateDiff {
        let mut d = StateDiff::new();
        for _ in 0..rng.below(8) {
            let a = Address::with_last_byte(rng.below(12) as u8);
            match rng.below(6) {
                0 => {
                    d.destroy_account(a);
                }
                1 => {
                    let ch = d.insert_code(Bytes::from(rng.b256().to_vec()));
                    d.set_account(
                        a,
                        AccountInfo {
                            nonce: 1,
                            balance: rng.u256(),
                            code_hash: ch,
                        },
                    );
                }
                2 => {
                    d.set_account(a, AccountInfo::new(rng.below(100), rng.u256()));
                    let sd = d.storage.entry(a).or_default();
                    sd.wiped = true;
                    sd.slots
                        .insert(B256::with_last_byte(rng.below(8) as u8), rng.u256());
                }
                3 | 4 => {
                    d.set_account(a, AccountInfo::new(rng.below(100), rng.u256()));
                    for _ in 0..rng.below(4) {
                        d.set_storage(a, B256::with_last_byte(rng.below(8) as u8), rng.u256());
                    }
                }
                _ => {
                    // Storage-only change of a (possibly non-existent) account.
                    d.set_storage(a, B256::with_last_byte(rng.below(8) as u8), rng.u256());
                }
            }
        }
        d
    }

    /// Apply many random diffs through `write_state_diff` and through `InMemoryState`;
    /// predicted roots, actual roots and full contents must agree at every step.
    fn random_diffs_agree(db: &dyn Database, seed: u64) {
        let mut rng = Rng(seed);
        let mut mem = InMemoryState::new();
        for _ in 0..60 {
            let d = random_diff(&mut rng);
            let p = DbStateProvider::new(db);
            let predicted = p.state_root_with(&d).unwrap();
            assert_eq!(predicted, mem.state_root_with(&d).unwrap());

            let mut batch = WriteBatch::new();
            write_state_diff(&mut batch, &d);
            db.write(batch).unwrap();
            mem.apply(&d);

            let p = DbStateProvider::new(db);
            assert_eq!(p.state_root_with(&StateDiff::new()).unwrap(), predicted);
            assert_eq!(mem.state_root(), predicted);
            assert_eq!(p.load_all().unwrap(), mem);
            // A snapshot gives the same answers.
            let snap = DbStateProvider::new(db.snapshot().unwrap());
            assert_eq!(snap.state_root_with(&StateDiff::new()).unwrap(), predicted);
        }
    }

    #[test]
    fn random_diffs_agree_memdb() {
        random_diffs_agree(&MemDb::new(), 0x1234_5678_9abc_def1);
    }

    #[test]
    fn random_diffs_agree_redb_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let root = {
            let db = RedbDatabase::open_dir(dir.path()).unwrap();
            random_diffs_agree(&db, 0x0fed_cba9_8765_4321);
            DbStateProvider::new(&db)
                .state_root_with(&StateDiff::new())
                .unwrap()
        };
        let db = RedbDatabase::open_dir(dir.path()).unwrap();
        let p = DbStateProvider::new(&db);
        assert_eq!(p.state_root_with(&StateDiff::new()).unwrap(), root);
        assert_eq!(p.load_all().unwrap().state_root(), root);
    }

    #[test]
    fn storage_without_account_is_not_in_root() {
        let db = MemDb::new();
        let a = Address::repeat_byte(7);
        let mut d = StateDiff::new();
        d.set_storage(a, B256::with_last_byte(1), U256::from(1u64));
        let p = DbStateProvider::new(&db);
        assert_eq!(
            p.state_root_with(&d).unwrap(),
            xdevs_primitives::EMPTY_ROOT_HASH
        );
    }
}

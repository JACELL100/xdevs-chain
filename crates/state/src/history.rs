//! State history: reverse diffs for the last N blocks (plan §14.1, §14.4).
//!
//! When block `b` is committed, [`write_state_history`] records the value every changed
//! account and slot had **before** `b` under the key `(.., b)` in `AccountHistory` /
//! `StorageHistory`, plus a [`StateChangeSet`] index for pruning. The state after an
//! older block `n` is then: for each key, the pre-value stored by the first block
//! `> n` that changed it, or the current flat value if no later block changed it
//! ([`HistoricalStateProvider`]). A lookup is one ordered seek, not a replay.
//!
//! `Meta[STATE_HISTORY_START] = s` means the diffs of blocks `s..=head` are all
//! present, so states `s - 1 ..= head` can be served ([`oldest_available_state`]).

use crate::{StateDiff, StateError, StateProvider};
use std::collections::BTreeSet;
use xdevs_primitives::{AccountInfo, Address, Bytes, B256, KECCAK256_EMPTY, U256};
use xdevs_storage::{meta_keys, tables, DbError, DbRead, DbReadExt, StateChangeSet, WriteBatch};

/// Default number of past blocks whose state stays readable (plan §14.4 `full` mode).
pub const DEFAULT_STATE_HISTORY_BLOCKS: u64 = 10_000;

/// Max blocks of old history deleted per commit, so a shrunk window (or a long
/// archive-to-full switch) is pruned gradually instead of in one huge batch.
pub const MAX_PRUNE_BLOCKS_PER_COMMIT: u64 = 64;

fn decode_u64(bytes: &[u8]) -> Result<u64, DbError> {
    let arr: [u8; 8] = bytes
        .try_into()
        .map_err(|_| DbError::Decode(format!("expected 8 bytes, got {}", bytes.len())))?;
    Ok(u64::from_be_bytes(arr))
}

/// `Meta[STATE_HISTORY_START]`: first block whose reverse diff is stored
/// (`None` = the database keeps no history yet).
pub fn history_start<R: DbRead + ?Sized>(db: &R) -> Result<Option<u64>, DbError> {
    db.get::<tables::Meta>(&meta_keys::STATE_HISTORY_START.to_string())?
        .map(|b| decode_u64(&b))
        .transpose()
}

/// Set `Meta[STATE_HISTORY_START]` in `batch` (genesis initialisation uses `1`: there
/// is no state before block 0, so every state from genesis on is readable).
pub fn set_history_start(batch: &mut WriteBatch, start: u64) {
    batch.put::<tables::Meta>(
        &meta_keys::STATE_HISTORY_START.to_string(),
        &start.to_be_bytes().to_vec(),
    );
}

/// Oldest block whose post-state can be read, given the current `head`.
pub fn oldest_available_state<R: DbRead + ?Sized>(db: &R, head: u64) -> Result<u64, DbError> {
    Ok(match history_start(db)? {
        Some(start) => start.saturating_sub(1).min(head),
        None => head,
    })
}

/// Append to `batch` the reverse diff of block `block`, whose changes are `diff`.
/// `pre` must be the state *before* `block` (the latest committed state when the
/// block is committed), and `batch` must also apply `diff` (same atomic commit).
pub fn write_state_history<R: DbRead + ?Sized>(
    batch: &mut WriteBatch,
    pre: &R,
    block: u64,
    diff: &StateDiff,
) -> Result<(), StateError> {
    let accounts: BTreeSet<Address> = diff.accounts.keys().copied().collect();
    let mut slots: BTreeSet<(Address, B256)> = BTreeSet::new();
    for (address, change) in &diff.accounts {
        if change.is_none() {
            // Destroyed: every existing slot is deleted.
            slots.extend(existing_slots(pre, *address)?);
        }
    }
    for (address, sd) in &diff.storage {
        if sd.wiped {
            slots.extend(existing_slots(pre, *address)?);
        }
        slots.extend(sd.slots.keys().map(|slot| (*address, *slot)));
    }
    if accounts.is_empty() && slots.is_empty() {
        return Ok(());
    }
    for address in &accounts {
        let before = pre.get::<tables::PlainAccounts>(address)?;
        batch.put::<tables::AccountHistory>(&(*address, block), &before);
    }
    for (address, slot) in &slots {
        let before = pre
            .get::<tables::PlainStorage>(&(*address, *slot))?
            .unwrap_or_default();
        batch.put::<tables::StorageHistory>(&(*address, *slot, block), &before);
    }
    batch.put::<tables::StateChangeSets>(
        &block,
        &StateChangeSet {
            accounts: accounts.into_iter().collect(),
            slots: slots.into_iter().collect(),
        },
    );
    Ok(())
}

fn existing_slots<R: DbRead + ?Sized>(
    db: &R,
    address: Address,
) -> Result<Vec<(Address, B256)>, DbError> {
    Ok(db
        .collect_prefix::<tables::PlainStorage>(address.as_slice())?
        .into_iter()
        .map(|(key, _)| key)
        .collect())
}

/// Append to `batch` the deletion of every history entry written by blocks
/// `from..to` (reads the change sets from `db`).
pub fn prune_state_history<R: DbRead + ?Sized>(
    batch: &mut WriteBatch,
    db: &R,
    from: u64,
    to: u64,
) -> Result<(), DbError> {
    for block in from..to {
        let Some(set) = db.get::<tables::StateChangeSets>(&block)? else {
            continue;
        };
        for address in set.accounts {
            batch.delete::<tables::AccountHistory>(&(address, block));
        }
        for (address, slot) in set.slots {
            batch.delete::<tables::StorageHistory>(&(address, slot, block));
        }
        batch.delete::<tables::StateChangeSets>(&block);
    }
    Ok(())
}

/// Record the reverse diff of `block` and prune history older than `keep` blocks,
/// updating `Meta[STATE_HISTORY_START]`; everything goes into `batch`.
///
/// After the commit the states `block - keep ..= block` are readable (`keep` is
/// clamped to at least 1; `u64::MAX` = archive, never prune). A database without a
/// history start (created before history existed) starts recording at `block`.
pub fn record_block_history<R: DbRead + ?Sized>(
    batch: &mut WriteBatch,
    pre: &R,
    block: u64,
    diff: &StateDiff,
    keep: u64,
) -> Result<(), StateError> {
    write_state_history(batch, pre, block, diff)?;
    let start = history_start(pre)?.unwrap_or(block);
    // Readable states block - keep ..= block need the diffs of blocks > block - keep.
    let keep_from = (block + 1).saturating_sub(keep.max(1));
    let new_start = keep_from.min(start.saturating_add(MAX_PRUNE_BLOCKS_PER_COMMIT));
    let new_start = new_start.max(start);
    prune_state_history(batch, pre, start, new_start)?;
    set_history_start(batch, new_start);
    Ok(())
}

/// [`StateProvider`] for the state right after block `block`, built from the latest
/// flat state plus the reverse diffs of the later blocks.
///
/// The caller must check `block >= oldest_available_state(..)` first; older blocks
/// would silently read pruned (wrong) values.
#[derive(Debug, Clone)]
pub struct HistoricalStateProvider<D> {
    db: D,
    block: u64,
}

impl<D: DbRead> HistoricalStateProvider<D> {
    /// State after block `block` over a database handle or snapshot.
    pub const fn new(db: D, block: u64) -> Self {
        Self { db, block }
    }

    /// The block whose post-state this provider reads.
    pub const fn block(&self) -> u64 {
        self.block
    }
}

impl<D: DbRead> StateProvider for HistoricalStateProvider<D> {
    fn basic_account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        let mut found = None;
        self.db.walk::<tables::AccountHistory>(
            &(address, self.block.saturating_add(1)),
            |(a, _), before| {
                if a == address {
                    found = Some(before);
                }
                Ok(false)
            },
        )?;
        match found {
            Some(before) => Ok(before),
            None => Ok(self.db.get::<tables::PlainAccounts>(&address)?),
        }
    }

    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        let mut found = None;
        self.db.walk::<tables::StorageHistory>(
            &(address, slot, self.block.saturating_add(1)),
            |(a, s, _), before| {
                if a == address && s == slot {
                    found = Some(before);
                }
                Ok(false)
            },
        )?;
        match found {
            Some(before) => Ok(before),
            None => Ok(self
                .db
                .get::<tables::PlainStorage>(&(address, slot))?
                .unwrap_or_default()),
        }
    }

    fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytes>, StateError> {
        // Bytecodes are content-addressed and never deleted.
        if code_hash == KECCAK256_EMPTY {
            return Ok(Some(Bytes::new()));
        }
        Ok(self.db.get::<tables::Bytecodes>(&code_hash)?)
    }

    fn block_hash(&self, number: u64) -> Result<Option<B256>, StateError> {
        if number > self.block {
            return Ok(None);
        }
        Ok(self
            .db
            .get::<tables::Headers>(&number)?
            .map(|h| h.hash_slow()))
    }

    fn state_root_with(&self, _diff: &StateDiff) -> Result<B256, StateError> {
        // Roots of past states are in their headers; recomputing one would need the
        // whole historical state. Not needed by calls / estimates.
        Err(StateError::Unavailable(format!(
            "state root of historical state at block {}",
            self.block
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{root::tests::Rng, write_state_diff, DbStateProvider, InMemoryState};
    use xdevs_storage::{Database, MemDb};

    /// Commit `diff` as block `block` with history (`keep` blocks).
    fn commit(db: &MemDb, block: u64, diff: &StateDiff, keep: u64) {
        let mut batch = WriteBatch::new();
        record_block_history(&mut batch, db, block, diff, keep).unwrap();
        write_state_diff(&mut batch, db, diff).unwrap();
        db.write(batch).unwrap();
    }

    fn assert_same_state(
        p: &dyn StateProvider,
        expected: &InMemoryState,
        keys: &[(Address, B256)],
    ) {
        for (a, s) in keys {
            assert_eq!(
                p.basic_account(*a).unwrap(),
                expected.basic_account(*a).unwrap(),
                "account {a}"
            );
            assert_eq!(
                p.storage(*a, *s).unwrap(),
                expected.storage(*a, *s).unwrap(),
                "slot {a} {s}"
            );
        }
    }

    fn random_diff(rng: &mut Rng) -> StateDiff {
        let mut d = StateDiff::new();
        for _ in 0..rng.below(6) {
            let a = Address::with_last_byte(rng.below(6) as u8);
            match rng.below(5) {
                0 => {
                    d.destroy_account(a);
                }
                1 => {
                    d.set_account(a, AccountInfo::new(rng.below(9), rng.u256()));
                    let sd = d.storage.entry(a).or_default();
                    sd.wiped = true;
                    sd.slots
                        .insert(B256::with_last_byte(rng.below(4) as u8), rng.u256());
                }
                2 => {
                    // Slot reset to zero (deleted).
                    d.set_storage(a, B256::with_last_byte(rng.below(4) as u8), U256::ZERO);
                }
                _ => {
                    d.set_account(a, AccountInfo::new(rng.below(9), rng.u256()));
                    for _ in 0..rng.below(3) {
                        d.set_storage(a, B256::with_last_byte(rng.below(4) as u8), rng.u256());
                    }
                }
            }
        }
        d
    }

    fn all_keys() -> Vec<(Address, B256)> {
        let mut keys = Vec::new();
        for a in 0..6u8 {
            for s in 0..4u8 {
                keys.push((Address::with_last_byte(a), B256::with_last_byte(s)));
            }
        }
        keys
    }

    #[test]
    fn every_past_state_is_reconstructed_exactly() {
        let db = MemDb::new();
        let mut rng = Rng(0xdead_beef_1234_5678);
        let mut states = vec![InMemoryState::new()];
        let mut genesis = WriteBatch::new();
        set_history_start(&mut genesis, 1);
        db.write(genesis).unwrap();
        for block in 1..=40u64 {
            let d = random_diff(&mut rng);
            commit(&db, block, &d, u64::MAX);
            let mut next = states.last().unwrap().clone();
            next.apply(&d);
            states.push(next);
        }
        let keys = all_keys();
        let snap = db.snapshot().unwrap();
        assert_eq!(oldest_available_state(&*snap, 40).unwrap(), 0);
        for (n, expected) in states.iter().enumerate() {
            let p = HistoricalStateProvider::new(&*snap, n as u64);
            assert_same_state(&p, expected, &keys);
        }
        // The head through the historical path equals the flat state.
        let latest = DbStateProvider::new(&*snap);
        let head = HistoricalStateProvider::new(&*snap, 40);
        for (a, s) in &keys {
            assert_eq!(
                head.basic_account(*a).unwrap(),
                latest.basic_account(*a).unwrap()
            );
            assert_eq!(
                head.storage(*a, *s).unwrap(),
                latest.storage(*a, *s).unwrap()
            );
        }
    }

    #[test]
    fn pruning_keeps_the_window_and_deletes_old_entries() {
        let db = MemDb::new();
        let mut rng = Rng(42);
        let mut states = vec![InMemoryState::new()];
        let mut genesis = WriteBatch::new();
        set_history_start(&mut genesis, 1);
        db.write(genesis).unwrap();
        let keep = 5;
        for block in 1..=30u64 {
            let d = random_diff(&mut rng);
            commit(&db, block, &d, keep);
            let mut next = states.last().unwrap().clone();
            next.apply(&d);
            states.push(next);
            assert_eq!(
                oldest_available_state(&db, block).unwrap(),
                block.saturating_sub(keep)
            );
        }
        let keys = all_keys();
        for n in 25..=30u64 {
            let p = HistoricalStateProvider::new(&db, n);
            assert_same_state(&p, &states[n as usize], &keys);
        }
        // Nothing older than the window is left behind.
        assert_eq!(history_start(&db).unwrap(), Some(26));
        let mut stale = 0;
        db.walk::<tables::StateChangeSets>(&0, |b, _| {
            assert!(b >= 26, "change set of pruned block {b}");
            stale += 1;
            Ok(true)
        })
        .unwrap();
        assert!(stale <= 5);
        db.walk::<tables::AccountHistory>(&(Address::ZERO, 0), |(_, b), _| {
            assert!(b >= 26, "account history of pruned block {b}");
            Ok(true)
        })
        .unwrap();
        db.walk::<tables::StorageHistory>(&(Address::ZERO, B256::ZERO, 0), |(_, _, b), _| {
            assert!(b >= 26, "storage history of pruned block {b}");
            Ok(true)
        })
        .unwrap();
    }

    #[test]
    fn database_without_history_starts_at_next_block() {
        let db = MemDb::new();
        let a = Address::repeat_byte(1);
        let mut d = StateDiff::new();
        d.set_account(a, AccountInfo::new(0, U256::from(5u64)));
        let mut batch = WriteBatch::new();
        write_state_diff(&mut batch, &db, &d).unwrap();
        db.write(batch).unwrap();
        // Pre-history database at head 7: only the head is readable.
        assert_eq!(oldest_available_state(&db, 7).unwrap(), 7);

        let mut d8 = StateDiff::new();
        d8.set_account(a, AccountInfo::new(1, U256::from(4u64)));
        commit(&db, 8, &d8, 100);
        assert_eq!(history_start(&db).unwrap(), Some(8));
        assert_eq!(oldest_available_state(&db, 8).unwrap(), 7);
        let at7 = HistoricalStateProvider::new(&db, 7);
        assert_eq!(
            at7.basic_account(a).unwrap().unwrap().balance,
            U256::from(5u64)
        );
        assert!(at7.state_root_with(&StateDiff::new()).is_err());
    }

    #[test]
    fn empty_blocks_write_no_history() {
        let db = MemDb::new();
        let mut batch = WriteBatch::new();
        write_state_history(&mut batch, &db, 3, &StateDiff::new()).unwrap();
        assert!(batch.is_empty());
    }
}

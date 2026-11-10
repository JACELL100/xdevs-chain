//! Correctness of the incremental tries against the full recompute (the oracle), the
//! independent Yellow-Paper reference trie and fresh rebuilds.

use super::*;
use crate::{
    db::write_flat_state_diff,
    root::{
        full_db_state_root,
        tests::{reference, Rng},
    },
    write_state_diff, DbStateProvider, InMemoryState, StateProvider,
};
use std::time::Instant;
use xdevs_primitives::{b256, Bytes, U256};
use xdevs_storage::{Database, MemDb, RedbDatabase};

/// Every `TrieNodes` entry of `db`, in key order.
fn trie_table(db: &dyn DbRead) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    db.walk_raw(TableId::TrieNodes, &[], &mut |k, v| {
        out.push((k.to_vec(), v.to_vec()));
        true
    })
    .unwrap();
    out
}

/// The `TrieNodes` entries a fresh build from the flat state of `db` produces.
fn rebuilt_table(db: &dyn DbRead) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut batch = WriteBatch::new();
    rebuild_state_trie(&mut batch, db).unwrap();
    let scratch = MemDb::new();
    scratch.write(batch).unwrap();
    trie_table(&scratch)
}

fn commit_overlay<D: DbRead + ?Sized>(db: &dyn Database, store: NodeOverlay<'_, D>) {
    let mut batch = WriteBatch::new();
    store.write_to(&mut batch);
    db.write(batch).unwrap();
}

/// The generic trie on short fixed-length keys with short values (so many nodes are
/// embedded in their parents) matches the independent reference trie after every batch
/// of random inserts/removes, and the stored nodes equal a fresh build.
#[test]
fn generic_trie_matches_reference_including_embedded_nodes() {
    let mut rng = Rng(0xdead_beef_1234_5678);
    let prefix = vec![0x07];
    let mut embedded_seen = 0usize;
    for _ in 0..300 {
        let key_len = 1 + rng.below(3) as usize;
        let db = MemDb::new();
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        for _ in 0..12 {
            let mut store = NodeOverlay::new(&db);
            let mut trie = Trie::open(&store, prefix.clone()).unwrap();
            for _ in 0..rng.below(12) {
                // Few distinct bytes => long shared prefixes, extensions and deep branches.
                let key: Vec<u8> = (0..key_len)
                    .map(|_| [0x00, 0x01, 0x10, 0x11, 0xf0, 0xff][rng.below(6) as usize])
                    .collect();
                if rng.below(3) == 0 {
                    trie.remove(&store, &nibbles(&key)).unwrap();
                    model.remove(&key);
                } else {
                    let value: Vec<u8> = (0..1 + rng.below(40)).map(|_| rng.next() as u8).collect();
                    trie.insert(&store, &nibbles(&key), value.clone()).unwrap();
                    model.insert(key, value);
                }
            }
            let root = trie.commit(&mut store, false);
            let pairs: Vec<_> = model.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            assert_eq!(root, reference::trie_root(&pairs));
            commit_overlay(&db, store);

            // Byte-for-byte the nodes of a trie built from scratch.
            let fresh = MemDb::new();
            let mut store = NodeOverlay::new(&fresh);
            let mut trie = Trie::empty(prefix.clone());
            for (k, v) in &model {
                trie.insert(&store, &nibbles(k), v.clone()).unwrap();
            }
            assert_eq!(trie.commit(&mut store, false), root);
            commit_overlay(&fresh, store);
            let stored = trie_table(&db);
            assert_eq!(stored, trie_table(&fresh));
            embedded_seen += stored.iter().filter(|(_, v)| v.len() < 32).count();
            // Reopening gives the same root.
            let store = NodeOverlay::new(&db);
            let mut reopened = Trie::open(&store, prefix.clone()).unwrap();
            let mut scratch = NodeOverlay::new(&db);
            if let Some((k, v)) = model.iter().next() {
                reopened.insert(&store, &nibbles(k), v.clone()).unwrap();
            }
            assert_eq!(reopened.commit(&mut scratch, false), root);
        }
    }
    assert!(
        embedded_seen > 100,
        "embedded nodes exercised: {embedded_seen}"
    );
}

/// A random block diff over `pool` addresses and `slots` storage slots: updates, storage
/// writes incl. zero values (deletions), storage of non-existent accounts, destroys,
/// destroy-and-recreate (wiped storage + new slots), contracts, empty diffs.
fn random_diff(rng: &mut Rng, pool: u64, slots: u64) -> StateDiff {
    let mut d = StateDiff::new();
    let slot = |rng: &mut Rng| B256::with_last_byte(rng.below(slots) as u8);
    for _ in 0..rng.below(10) {
        let a = Address::with_last_byte(rng.below(pool) as u8);
        match rng.below(8) {
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
                // Re-created at the same address: old storage wiped, new slots written.
                d.set_account(a, AccountInfo::new(rng.below(100), rng.u256()));
                let sd = d.storage.entry(a).or_default();
                sd.wiped = true;
                sd.slots.clear();
                for _ in 0..rng.below(4) {
                    sd.slots.insert(slot(rng), rng.u256());
                }
            }
            3..=5 => {
                d.set_account(a, AccountInfo::new(rng.below(100), rng.u256()));
                for _ in 0..rng.below(5) {
                    d.set_storage(a, slot(rng), rng.u256());
                }
            }
            6 => {
                // Storage-only change of a (possibly non-existent) account.
                d.set_storage(a, slot(rng), rng.u256());
            }
            _ => {
                // Explicit zero writes (slot deletions).
                for _ in 0..1 + rng.below(3) {
                    d.set_storage(a, slot(rng), U256::ZERO);
                }
            }
        }
    }
    d
}

/// Predict, write and re-read one diff; every root must agree with the full recompute
/// and the in-memory model, and the stored tries must equal a fresh rebuild.
fn step(db: &dyn Database, mem: &mut InMemoryState, d: &StateDiff, compare_tables: bool) -> B256 {
    let incremental = state_root_with(db, d).unwrap();
    assert_eq!(incremental, full_db_state_root(db, d).unwrap());
    assert_eq!(incremental, mem.state_root_with(d).unwrap());
    let mut batch = WriteBatch::new();
    assert_eq!(write_state_diff(&mut batch, db, d).unwrap(), incremental);
    db.write(batch).unwrap();
    mem.apply(d);
    assert!(state_trie_exists(db).unwrap());
    assert_eq!(state_root_with(db, &StateDiff::new()).unwrap(), incremental);
    assert_eq!(check_state_root(db).unwrap(), incremental);
    if compare_tables {
        assert_eq!(trie_table(db), rebuilt_table(db));
    }
    incremental
}

/// Thousands of random diff sequences (MemDb): incremental root == full recompute at every
/// step, before and after the write, and the trie table == a fresh build.
#[test]
fn incremental_root_equals_full_recompute_memdb() {
    let mut rng = Rng(0x5eed_0001_cafe_f00d);
    let mut diffs = 0;
    for _ in 0..400 {
        let pool = 2 + rng.below(40);
        let slots = 1 + rng.below(64);
        let db = MemDb::new();
        let mut mem = InMemoryState::new();
        for _ in 0..25 {
            let d = random_diff(&mut rng, pool, slots);
            step(&db, &mut mem, &d, true);
            diffs += 1;
        }
        assert_eq!(DbStateProvider::new(&db).load_all().unwrap(), mem);
    }
    assert_eq!(diffs, 10_000);
}

/// Same on redb, across closing and reopening the database.
#[test]
fn incremental_root_survives_redb_reopen() {
    let mut rng = Rng(0x5eed_0002_0bad_beef);
    for _ in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let mut mem = InMemoryState::new();
        let mut root = EMPTY_ROOT_HASH;
        for _ in 0..3 {
            let db = RedbDatabase::open_dir(dir.path()).unwrap();
            assert_eq!(state_root_with(&db, &StateDiff::new()).unwrap(), root);
            for _ in 0..25 {
                let d = random_diff(&mut rng, 30, 16);
                root = step(&db, &mut mem, &d, true);
            }
        }
        let db = RedbDatabase::open_dir(dir.path()).unwrap();
        assert_eq!(check_state_root(&db).unwrap(), root);
        assert_eq!(mem.state_root(), root);
    }
}

/// A database written without tries (Phase-1/2 data dir) answers with the full recompute,
/// gets its tries built by the first `write_state_diff`, and then stays exact.
#[test]
fn migration_from_flat_only_database() {
    let mut rng = Rng(0x5eed_0003_1111_2222);
    for round in 0..40 {
        let db = MemDb::new();
        let mut mem = InMemoryState::new();
        for _ in 0..1 + rng.below(15) {
            let d = random_diff(&mut rng, 25, 32);
            assert_eq!(
                state_root_with(&db, &d).unwrap(),
                mem.state_root_with(&d).unwrap()
            );
            let mut batch = WriteBatch::new();
            write_flat_state_diff(&mut batch, &d);
            db.write(batch).unwrap();
            mem.apply(&d);
        }
        assert!(!state_trie_exists(&db).unwrap());
        assert_eq!(check_state_root(&db).unwrap(), mem.state_root());
        // First write migrates (an empty diff on some rounds) in the same batch.
        let d = if round % 4 == 0 {
            StateDiff::new()
        } else {
            random_diff(&mut rng, 25, 32)
        };
        step(&db, &mut mem, &d, true);
        for _ in 0..10 {
            let d = random_diff(&mut rng, 25, 32);
            step(&db, &mut mem, &d, true);
        }
    }
}

/// Migration of an on-disk (redb) flat-only database after reopening it.
#[test]
fn migration_of_redb_database_after_reopen() {
    let mut rng = Rng(0x5eed_0004_3333_4444);
    let dir = tempfile::tempdir().unwrap();
    let mut mem = InMemoryState::new();
    {
        let db = RedbDatabase::open_dir(dir.path()).unwrap();
        for _ in 0..20 {
            let d = random_diff(&mut rng, 30, 16);
            let mut batch = WriteBatch::new();
            write_flat_state_diff(&mut batch, &d);
            db.write(batch).unwrap();
            mem.apply(&d);
        }
    }
    let db = RedbDatabase::open_dir(dir.path()).unwrap();
    assert!(!state_trie_exists(&db).unwrap());
    for _ in 0..20 {
        let d = random_diff(&mut rng, 30, 16);
        step(&db, &mut mem, &d, true);
    }
}

/// Fresh genesis: the empty state and the Morden genesis vector through the tries.
#[test]
fn empty_and_morden_roots_through_tries() {
    let db = MemDb::new();
    let mut batch = WriteBatch::new();
    assert_eq!(
        write_state_diff(&mut batch, &db, &StateDiff::new()).unwrap(),
        EMPTY_ROOT_HASH
    );
    db.write(batch).unwrap();
    assert_eq!(
        trie_table(&db),
        vec![(account_trie_key(&[]), vec![EMPTY_STRING_CODE])]
    );
    assert_eq!(
        state_root_with(&db, &StateDiff::new()).unwrap(),
        b256!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
    );

    let nonce = 1u64 << 20;
    let mut d = StateDiff::new();
    for i in 1..=4u8 {
        d.set_account(
            Address::with_last_byte(i),
            AccountInfo::new(nonce, U256::from(1u64)),
        );
    }
    d.set_account(
        xdevs_primitives::address!("102e61f5d8f9bc71d0ad4a084df4e65e05ce0e1c"),
        AccountInfo::new(nonce, U256::from(1u64) << 200),
    );
    let expected = b256!("f3f4696bbf3b3b07775128eb7a3763279a394e382130f27c21e70233e04946a9");
    for db in [MemDb::new(), db] {
        assert_eq!(state_root_with(&db, &d).unwrap(), expected);
        let mut batch = WriteBatch::new();
        assert_eq!(write_state_diff(&mut batch, &db, &d).unwrap(), expected);
        db.write(batch).unwrap();
        assert_eq!(check_state_root(&db).unwrap(), expected);
        // Deleting everything again gives the empty root and leaves only the root entry.
        let mut gone = StateDiff::new();
        for a in d.accounts.keys() {
            gone.destroy_account(*a);
        }
        let mut batch = WriteBatch::new();
        assert_eq!(
            write_state_diff(&mut batch, &db, &gone).unwrap(),
            EMPTY_ROOT_HASH
        );
        db.write(batch).unwrap();
        assert_eq!(trie_table(&db).len(), 1);
    }
}

/// Tampering with the flat state or the trie nodes is detected, never silently used.
#[test]
fn corruption_is_detected() {
    let mut rng = Rng(0x5eed_0005_5555_6666);
    let db = MemDb::new();
    let mut mem = InMemoryState::new();
    let mut d = StateDiff::new();
    for i in 0..64u8 {
        d.set_account(Address::with_last_byte(i), AccountInfo::new(1, rng.u256()));
    }
    step(&db, &mut mem, &d, true);

    // Flat state changed behind the trie's back: the start-up check fails.
    let mut batch = WriteBatch::new();
    batch.put::<tables::PlainAccounts>(&Address::repeat_byte(0x42), &AccountInfo::default());
    let tampered = MemDb::new();
    tampered.write(rebuild_into(&db)).unwrap();
    tampered.write(batch).unwrap();
    assert!(check_state_root(&tampered).is_err());

    // A trie node replaced by different bytes: reading through it is an error.
    let (key, _) = trie_table(&db)
        .into_iter()
        .find(|(k, _)| k.len() == 2)
        .expect("depth-1 node");
    let mut batch = WriteBatch::new();
    batch.put_raw(TableId::TrieNodes, key.clone(), vec![0xc2, 0x80, 0x80]);
    db.write(batch).unwrap();
    let victim = (0..64u8)
        .map(Address::with_last_byte)
        .find(|a| nibbles(keccak256(a).as_slice())[0] == key[1])
        .unwrap();
    let mut d = StateDiff::new();
    d.set_account(victim, AccountInfo::new(2, U256::from(7u64)));
    assert!(matches!(
        state_root_with(&db, &d),
        Err(StateError::Inconsistent(_))
    ));
    let mut batch = WriteBatch::new();
    assert!(write_state_diff(&mut batch, &db, &d).is_err());
}

/// Copy of `db`'s flat tables plus a fresh trie build, as one batch.
fn rebuild_into(db: &MemDb) -> WriteBatch {
    let mut batch = WriteBatch::new();
    let mut d = StateDiff::new();
    db.walk::<tables::PlainAccounts>(&Address::ZERO, |a, info| {
        d.set_account(a, info);
        Ok(true)
    })
    .unwrap();
    write_flat_state_diff(&mut batch, &d);
    let scratch = MemDb::new();
    scratch.write(batch.clone()).unwrap();
    rebuild_state_trie(&mut batch, &scratch).unwrap();
    batch
}

/// A state of `n` accounts (every 10th a contract with 8 storage slots).
fn large_state(n: u32) -> StateDiff {
    let mut d = StateDiff::new();
    for i in 0..n {
        let a = Address::from_slice(&keccak256(i.to_be_bytes())[..20]);
        d.set_account(
            a,
            AccountInfo::new(u64::from(i), U256::from(i) * U256::from(1_000u64)),
        );
        if i % 10 == 0 {
            for s in 0..8u8 {
                d.set_storage(a, B256::with_last_byte(s), U256::from(u32::from(s) + i + 1));
            }
        }
    }
    d
}

/// A block touching 10 accounts (balances, nonces, a few storage slots).
fn ten_account_block(n: u32, seed: u32) -> StateDiff {
    let mut d = StateDiff::new();
    for j in 0..10u32 {
        let i = (seed * 7919 + j * 104_729) % n;
        let a = Address::from_slice(&keccak256(i.to_be_bytes())[..20]);
        d.set_account(a, AccountInfo::new(u64::from(i) + 1, U256::from(seed + j)));
        if j % 3 == 0 {
            d.set_storage(a, B256::with_last_byte(3), U256::from(seed));
        }
    }
    d
}

fn bench_db(name: &str, db: &dyn Database, n: u32) {
    let genesis = large_state(n);
    let t = Instant::now();
    let mut batch = WriteBatch::new();
    write_state_diff(&mut batch, db, &genesis).unwrap();
    db.write(batch).unwrap();
    let build = t.elapsed();

    let (mut full, mut incr, mut write) =
        (Default::default(), Default::default(), Default::default());
    const ROUNDS: u32 = 5;
    for round in 0..ROUNDS {
        let d = ten_account_block(n, round + 1);
        let t = Instant::now();
        let a = full_db_state_root(db, &d).unwrap();
        full += t.elapsed();
        let t = Instant::now();
        let b = state_root_with(db, &d).unwrap();
        incr += t.elapsed();
        assert_eq!(a, b);
        let t = Instant::now();
        let mut batch = WriteBatch::new();
        assert_eq!(write_state_diff(&mut batch, db, &d).unwrap(), b);
        write += t.elapsed();
        db.write(batch).unwrap();
    }
    let per = |d: std::time::Duration| d / ROUNDS;
    println!(
        "{name} {n:>6} accounts: genesis build {build:?}; 10-account block root: \
         full {:?}, incremental {:?} (+ trie writes into batch {:?})",
        per(full),
        per(incr),
        per(write)
    );
}

/// Timing comparison (debug build): `cargo test -p xdevs-state bench_ -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark"]
fn bench_root_update_10_accounts() {
    for n in [10_000u32, 100_000] {
        bench_db("MemDb", &MemDb::new(), n);
        let dir = tempfile::tempdir().unwrap();
        bench_db("redb ", &RedbDatabase::open_dir(dir.path()).unwrap(), n);
    }
}

//! Ethereum Merkle-Patricia state/storage roots (plan §14.3, via `alloy-trie`).
//!
//! * account trie: `keccak256(address) -> RLP([nonce, balance, storage_root, code_hash])`
//! * storage trie (per account): `keccak256(slot) -> RLP(value)`, zero values skipped
//!
//! Every function here recomputes roots from scratch over the full (flat) state: O(n log n)
//! (hash + sort, then one `HashBuilder` pass) and deterministic. The node computes block
//! state roots incrementally over the persisted tries ([`crate::trie`]); these full
//! recomputes are the reference oracle (tests, start-up check, pre-trie databases).

use crate::{InMemoryState, StateDiff};
use std::collections::{BTreeMap, BTreeSet};
use xdevs_primitives::{
    reexports::alloy_trie::{root as trie_root, TrieAccount},
    AccountInfo, Address, B256, EMPTY_ROOT_HASH, U256,
};
use xdevs_storage::{tables, DbError, DbRead, DbReadExt};

/// Storage root of one account from its (unhashed) slots. Zero values are skipped.
pub fn storage_root<'a>(slots: impl IntoIterator<Item = (&'a B256, &'a U256)>) -> B256 {
    trie_root::storage_root_unhashed(
        slots
            .into_iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(k, v)| (*k, *v)),
    )
}

/// State root from flat accounts and the storage roots of the accounts that have
/// storage (missing entries mean [`EMPTY_ROOT_HASH`]).
pub fn state_root_from_storage_roots(
    accounts: &BTreeMap<Address, AccountInfo>,
    storage_roots: &BTreeMap<Address, B256>,
) -> B256 {
    trie_root::state_root_unhashed(accounts.iter().map(|(address, info)| {
        let sr = storage_roots
            .get(address)
            .copied()
            .unwrap_or(EMPTY_ROOT_HASH);
        (*address, TrieAccount::from(info.with_storage_root(sr)))
    }))
}

/// State root from flat accounts and their storage.
pub fn state_root(
    accounts: &BTreeMap<Address, AccountInfo>,
    storage: &BTreeMap<Address, BTreeMap<B256, U256>>,
) -> B256 {
    trie_root::state_root_unhashed(accounts.iter().map(|(address, info)| {
        let sr = storage.get(address).map_or(EMPTY_ROOT_HASH, storage_root);
        (*address, TrieAccount::from(info.with_storage_root(sr)))
    }))
}

/// State root of an [`InMemoryState`] (same as [`InMemoryState::state_root`]).
pub fn in_memory_state_root(state: &InMemoryState) -> B256 {
    state.state_root()
}

/// **Full recompute** (reference oracle): state root of the flat state stored in `db`
/// (`PlainAccounts` / `PlainStorage`) with `diff` applied on top, without writing anything
/// and without loading bytecode. O(state); block processing uses the incremental
/// [`crate::trie::state_root_with`] instead.
///
/// One ordered pass over each table: accounts are collected (O(accounts) memory),
/// storage is streamed address by address, so only one account's slots are held at a
/// time. Semantics of `diff` are those documented on [`StateDiff`]; storage of
/// accounts that do not exist in the resulting state is ignored.
pub fn full_db_state_root<D: DbRead + ?Sized>(db: &D, diff: &StateDiff) -> Result<B256, DbError> {
    let mut accounts = BTreeMap::new();
    db.walk::<tables::PlainAccounts>(&Address::ZERO, |address, info| {
        accounts.insert(address, info);
        Ok(true)
    })?;
    for (address, change) in &diff.accounts {
        match change {
            Some(info) => accounts.insert(*address, *info),
            None => accounts.remove(address),
        };
    }

    let mut storage_roots = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut current: Option<(Address, Vec<(B256, U256)>)> = None;
    db.walk::<tables::PlainStorage>(&(Address::ZERO, B256::ZERO), |(address, slot), value| {
        match &mut current {
            Some((a, slots)) if *a == address => slots.push((slot, value)),
            _ => {
                if let Some((a, slots)) = current.replace((address, vec![(slot, value)])) {
                    flush_storage(a, slots, &accounts, diff, &mut storage_roots);
                }
                seen.insert(address);
            }
        }
        Ok(true)
    })?;
    if let Some((a, slots)) = current.take() {
        flush_storage(a, slots, &accounts, diff, &mut storage_roots);
    }
    // Accounts that gain storage in `diff` but had none on disk.
    for address in diff.storage.keys().filter(|a| !seen.contains(*a)) {
        flush_storage(*address, Vec::new(), &accounts, diff, &mut storage_roots);
    }

    Ok(state_root_from_storage_roots(&accounts, &storage_roots))
}

/// Compute the final storage root of `address` from its stored `base` slots plus `diff`
/// and record it in `roots` (only when the account exists and has storage).
fn flush_storage(
    address: Address,
    base: Vec<(B256, U256)>,
    accounts: &BTreeMap<Address, AccountInfo>,
    diff: &StateDiff,
    roots: &mut BTreeMap<Address, B256>,
) {
    if !accounts.contains_key(&address) || matches!(diff.accounts.get(&address), Some(None)) {
        return;
    }
    let mut slots: BTreeMap<B256, U256> = match diff.storage.get(&address) {
        Some(sd) if sd.wiped => BTreeMap::new(),
        _ => base.into_iter().collect(),
    };
    if let Some(sd) = diff.storage.get(&address) {
        slots.extend(sd.slots.iter().map(|(k, v)| (*k, *v)));
    }
    if slots.values().any(|v| !v.is_zero()) {
        roots.insert(address, storage_root(&slots));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use xdevs_primitives::{b256, keccak256, KECCAK256_EMPTY};

    /// Minimal, independent Merkle-Patricia trie (Yellow Paper appendix D), written
    /// straight from the spec with hand-rolled RLP. Used to cross-check `alloy-trie`.
    pub(crate) mod reference {
        use xdevs_primitives::{keccak256, B256};

        fn len_prefix(offset: u8, len: usize) -> Vec<u8> {
            if len < 56 {
                vec![offset + len as u8]
            } else {
                let be: Vec<u8> = (len as u64)
                    .to_be_bytes()
                    .into_iter()
                    .skip_while(|b| *b == 0)
                    .collect();
                let mut out = vec![offset + 55 + be.len() as u8];
                out.extend(be);
                out
            }
        }

        pub fn rlp_bytes(b: &[u8]) -> Vec<u8> {
            if b.len() == 1 && b[0] < 0x80 {
                return b.to_vec();
            }
            let mut out = len_prefix(0x80, b.len());
            out.extend_from_slice(b);
            out
        }

        /// RLP of an unsigned integer given as big-endian bytes (leading zeros stripped).
        pub fn rlp_uint(be: &[u8]) -> Vec<u8> {
            let trimmed: Vec<u8> = be.iter().copied().skip_while(|b| *b == 0).collect();
            rlp_bytes(&trimmed)
        }

        pub fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
            let payload = items.concat();
            let mut out = len_prefix(0xc0, payload.len());
            out.extend(payload);
            out
        }

        fn hex_prefix(nibbles: &[u8], leaf: bool) -> Vec<u8> {
            let flag = if leaf { 2u8 } else { 0 };
            let (mut out, rest) = if nibbles.len() % 2 == 1 {
                (vec![((flag + 1) << 4) | nibbles[0]], &nibbles[1..])
            } else {
                (vec![flag << 4], nibbles)
            };
            out.extend(rest.chunks(2).map(|p| (p[0] << 4) | p[1]));
            out
        }

        fn child_ref(encoded: &[u8]) -> Vec<u8> {
            if encoded.len() < 32 {
                encoded.to_vec()
            } else {
                rlp_bytes(keccak256(encoded).as_slice())
            }
        }

        /// RLP of the node holding `items`, whose keys all share their first `depth` nibbles.
        fn node(items: &[(Vec<u8>, Vec<u8>)], depth: usize) -> Vec<u8> {
            if let [(key, value)] = items {
                return rlp_list(&[
                    rlp_bytes(&hex_prefix(&key[depth..], true)),
                    rlp_bytes(value),
                ]);
            }
            let first = &items[0].0;
            let mut common = first.len();
            for (key, _) in items {
                let mut i = depth;
                while i < common.min(key.len()) && key[i] == first[i] {
                    i += 1;
                }
                common = i;
            }
            if common > depth {
                let ext = hex_prefix(&first[depth..common], false);
                return rlp_list(&[rlp_bytes(&ext), child_ref(&node(items, common))]);
            }
            let mut children: Vec<Vec<u8>> = (0..16u8)
                .map(|n| {
                    let sub: Vec<_> = items
                        .iter()
                        .filter(|(k, _)| k.len() > depth && k[depth] == n)
                        .cloned()
                        .collect();
                    if sub.is_empty() {
                        rlp_bytes(&[])
                    } else {
                        child_ref(&node(&sub, depth + 1))
                    }
                })
                .collect();
            let value = items
                .iter()
                .find(|(k, _)| k.len() == depth)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            children.push(rlp_bytes(&value));
            rlp_list(&children)
        }

        /// Root of the trie mapping raw byte keys to raw byte values.
        pub fn trie_root(pairs: &[(Vec<u8>, Vec<u8>)]) -> B256 {
            if pairs.is_empty() {
                return keccak256([0x80u8]);
            }
            let items: Vec<(Vec<u8>, Vec<u8>)> = pairs
                .iter()
                .map(|(k, v)| {
                    (
                        k.iter().flat_map(|b| [b >> 4, b & 0x0f]).collect(),
                        v.clone(),
                    )
                })
                .collect();
            keccak256(node(&items, 0))
        }
    }

    /// Reference state root built from the Yellow Paper definitions only.
    fn reference_state_root(
        accounts: &BTreeMap<Address, AccountInfo>,
        storage: &BTreeMap<Address, BTreeMap<B256, U256>>,
    ) -> B256 {
        use reference::{rlp_bytes, rlp_list, rlp_uint, trie_root};
        let pairs: Vec<_> = accounts
            .iter()
            .map(|(address, info)| {
                let slots: Vec<_> = storage
                    .get(address)
                    .into_iter()
                    .flatten()
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(k, v)| (keccak256(k).to_vec(), rlp_uint(&v.to_be_bytes::<32>())))
                    .collect();
                let value = rlp_list(&[
                    rlp_uint(&info.nonce.to_be_bytes()),
                    rlp_uint(&info.balance.to_be_bytes::<32>()),
                    rlp_bytes(trie_root(&slots).as_slice()),
                    rlp_bytes(info.code_hash.as_slice()),
                ]);
                (keccak256(address).to_vec(), value)
            })
            .collect();
        trie_root(&pairs)
    }

    fn kv(k: &str, v: &str) -> (Vec<u8>, Vec<u8>) {
        (k.as_bytes().to_vec(), v.as_bytes().to_vec())
    }

    /// The reference trie reproduces published go-ethereum / ethereum-tests vectors
    /// (go-ethereum `trie/trie_test.go` `TestInsert`; ethereum/tests
    /// `TrieTests/trieanyorder.json` "puppy" & "foo"), so it is a trustworthy oracle.
    #[test]
    fn reference_trie_matches_published_vectors() {
        assert_eq!(reference::trie_root(&[]), EMPTY_ROOT_HASH);
        let dogs = [
            kv("doe", "reindeer"),
            kv("dog", "puppy"),
            kv("dogglesworth", "cat"),
        ];
        assert_eq!(
            reference::trie_root(&dogs),
            b256!("8aad789dff2f538bca5d8ea56e8abe10f4c7ba3a5dea95fea4cd6e7c3a1168d3")
        );
        let single = [kv("A", &"a".repeat(50))];
        assert_eq!(
            reference::trie_root(&single),
            b256!("d23786fb4a010da3ce639d66d5e904a11dbc02746d1ce25029e53290cabf28ab")
        );
        let puppy = [
            kv("do", "verb"),
            kv("horse", "stallion"),
            kv("doge", "coin"),
            kv("dog", "puppy"),
        ];
        assert_eq!(
            reference::trie_root(&puppy),
            b256!("5991bb8c6514148a29db676a14ac506cd2cd5775ace63c30a4fe457715e9ac84")
        );
        let foo = [kv("foo", "bar"), kv("food", "bass")];
        assert_eq!(
            reference::trie_root(&foo),
            b256!("17beaa1648bafa633cda809c90c04af50fc8aed3cb40d16efbddee6fdf63c4c3")
        );
    }

    /// Plan §28 critical check: the empty state root is the empty-trie root.
    #[test]
    fn empty_state_root() {
        let expected = b256!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421");
        assert_eq!(EMPTY_ROOT_HASH, expected);
        assert_eq!(state_root(&BTreeMap::new(), &BTreeMap::new()), expected);
        assert_eq!(storage_root(&BTreeMap::new()), expected);
        assert_eq!(in_memory_state_root(&InMemoryState::new()), expected);
        assert_eq!(
            full_db_state_root(&xdevs_storage::MemDb::new(), &StateDiff::new()).unwrap(),
            expected
        );
    }

    /// Known Ethereum genesis vector: the Morden testnet (chain id 2) genesis state.
    ///
    /// Source: the Morden genesis as shipped by go-ethereum / Parity (`morden.json`):
    /// precompiles `0x..01`-`0x..04` hold 1 wei, `0x102e61f5d8f9bc71d0ad4a084df4e65e05ce0e1c`
    /// holds 2^200 wei, and every account starts at Morden's `accountStartNonce` 2^20.
    /// Published Morden block 0 (hash `0x0cd786a2425d16f152c658316c423e6ce1181e15c3295826d7c9904cba9ce303`)
    /// has `stateRoot = 0xf3f4696bbf3b3b07775128eb7a3763279a394e382130f27c21e70233e04946a9`.
    #[test]
    fn morden_genesis_state_root() {
        let nonce = 1u64 << 20;
        let mut accounts = BTreeMap::new();
        for i in 1..=4u8 {
            accounts.insert(
                Address::with_last_byte(i),
                AccountInfo::new(nonce, U256::from(1u64)),
            );
        }
        accounts.insert(
            xdevs_primitives::address!("102e61f5d8f9bc71d0ad4a084df4e65e05ce0e1c"),
            AccountInfo::new(nonce, U256::from(1u64) << 200),
        );
        let expected = b256!("f3f4696bbf3b3b07775128eb7a3763279a394e382130f27c21e70233e04946a9");
        assert_eq!(state_root(&accounts, &BTreeMap::new()), expected);
        assert_eq!(reference_state_root(&accounts, &BTreeMap::new()), expected);

        // Same vector through the persistent path (flat tables -> streaming root).
        let mut diff = StateDiff::new();
        for (a, info) in &accounts {
            diff.set_account(*a, *info);
        }
        let db = xdevs_storage::MemDb::new();
        assert_eq!(full_db_state_root(&db, &diff).unwrap(), expected);
        assert_eq!(
            in_memory_state_root(&InMemoryState::from_diff(&diff)),
            expected
        );
    }

    /// Small deterministic PRNG (xorshift64*) so the test needs no extra crates.
    pub(crate) struct Rng(pub(crate) u64);
    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        pub(crate) fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        pub(crate) fn b256(&mut self) -> B256 {
            let mut b = [0u8; 32];
            for chunk in b.chunks_mut(8) {
                chunk.copy_from_slice(&self.next().to_be_bytes());
            }
            B256::from(b)
        }
        pub(crate) fn u256(&mut self) -> U256 {
            match self.below(4) {
                0 => U256::ZERO,
                1 => U256::from(self.below(256)),
                2 => U256::from(self.next()),
                _ => U256::from_be_bytes(self.b256().0),
            }
        }
    }

    type RandomState = (
        BTreeMap<Address, AccountInfo>,
        BTreeMap<Address, BTreeMap<B256, U256>>,
    );

    fn random_state(rng: &mut Rng) -> RandomState {
        let mut accounts = BTreeMap::new();
        let mut storage = BTreeMap::new();
        for _ in 0..rng.below(40) {
            let address = Address::from_slice(&rng.b256()[..20]);
            let code_hash = if rng.below(3) == 0 {
                rng.b256()
            } else {
                KECCAK256_EMPTY
            };
            accounts.insert(
                address,
                AccountInfo {
                    nonce: rng.below(1 << 20),
                    balance: rng.u256(),
                    code_hash,
                },
            );
            let slots: BTreeMap<B256, U256> = (0..rng.below(12))
                .map(|_| (B256::with_last_byte(rng.below(32) as u8), rng.u256()))
                .collect();
            if !slots.is_empty() {
                storage.insert(address, slots);
            }
        }
        (accounts, storage)
    }

    /// `alloy-trie` agrees with the independent reference trie on random states
    /// (including zero-valued slots, large balances and contracts).
    #[test]
    fn alloy_trie_matches_reference_on_random_states() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..200 {
            let (accounts, storage) = random_state(&mut rng);
            assert_eq!(
                state_root(&accounts, &storage),
                reference_state_root(&accounts, &storage)
            );
        }
    }

    #[test]
    fn zero_slots_ignored() {
        let mut slots = BTreeMap::new();
        slots.insert(B256::with_last_byte(1), U256::ZERO);
        assert_eq!(storage_root(&slots), EMPTY_ROOT_HASH);
    }

    #[test]
    fn single_account_root_is_stable() {
        let mut accounts = BTreeMap::new();
        accounts.insert(
            Address::with_last_byte(1),
            AccountInfo::new(0, U256::from(1u64)),
        );
        let r1 = state_root(&accounts, &BTreeMap::new());
        assert_eq!(r1, state_root(&accounts, &BTreeMap::new()));
        assert_ne!(r1, EMPTY_ROOT_HASH);
        accounts.insert(
            Address::with_last_byte(1),
            AccountInfo::new(0, U256::from(2u64)),
        );
        assert_ne!(state_root(&accounts, &BTreeMap::new()), r1);
    }
}

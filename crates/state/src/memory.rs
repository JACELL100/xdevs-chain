//! Complete in-memory state (for tests, genesis computation and tooling).

use crate::{root, StateDiff, StateError, StateProvider};
use std::collections::BTreeMap;
use xdevs_primitives::{keccak256, Account, AccountInfo, Address, Bytes, B256, U256};

/// In-memory world state with deterministic iteration and Ethereum state root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InMemoryState {
    accounts: BTreeMap<Address, AccountInfo>,
    storage: BTreeMap<Address, BTreeMap<B256, U256>>,
    codes: BTreeMap<B256, Bytes>,
    block_hashes: BTreeMap<u64, B256>,
}

impl InMemoryState {
    /// Empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// State obtained by applying `diff` to an empty state (e.g. genesis alloc).
    pub fn from_diff(diff: &StateDiff) -> Self {
        let mut s = Self::new();
        s.apply(diff);
        s
    }

    /// Apply a diff with the semantics documented on [`StateDiff`].
    pub fn apply(&mut self, diff: &StateDiff) {
        for (address, change) in &diff.accounts {
            match change {
                Some(info) => {
                    self.accounts.insert(*address, *info);
                }
                None => {
                    self.accounts.remove(address);
                    self.storage.remove(address);
                }
            }
        }
        for (address, sd) in &diff.storage {
            if matches!(diff.accounts.get(address), Some(None)) {
                continue; // account destroyed: its storage is gone
            }
            if sd.wiped {
                self.storage.remove(address);
            }
            for (slot, value) in &sd.slots {
                if value.is_zero() {
                    if let Some(slots) = self.storage.get_mut(address) {
                        slots.remove(slot);
                    }
                } else {
                    self.storage
                        .entry(*address)
                        .or_default()
                        .insert(*slot, *value);
                }
            }
            if self.storage.get(address).is_some_and(BTreeMap::is_empty) {
                self.storage.remove(address);
            }
        }
        for (hash, code) in &diff.codes {
            self.codes.insert(*hash, code.clone());
        }
    }

    /// Insert or replace an account.
    pub fn insert_account(&mut self, address: Address, info: AccountInfo) -> &mut Self {
        self.accounts.insert(address, info);
        self
    }

    /// Store bytecode and return its hash.
    pub fn insert_code(&mut self, code: Bytes) -> B256 {
        let hash = keccak256(&code);
        self.codes.insert(hash, code);
        hash
    }

    /// Set a storage slot (zero deletes it).
    pub fn set_storage(&mut self, address: Address, slot: B256, value: U256) -> &mut Self {
        let mut d = StateDiff::new();
        d.set_storage(address, slot, value);
        self.apply(&d);
        self
    }

    /// Record a canonical block hash (for `BLOCKHASH`).
    pub fn set_block_hash(&mut self, number: u64, hash: B256) -> &mut Self {
        self.block_hashes.insert(number, hash);
        self
    }

    /// All accounts in address order.
    pub fn accounts(&self) -> &BTreeMap<Address, AccountInfo> {
        &self.accounts
    }

    /// Storage of one account in slot order (`None` if it has none).
    pub fn account_storage(&self, address: &Address) -> Option<&BTreeMap<B256, U256>> {
        self.storage.get(address)
    }

    /// Full Ethereum account (with storage root).
    pub fn account(&self, address: &Address) -> Option<Account> {
        self.accounts
            .get(address)
            .map(|info| info.with_storage_root(self.storage_root(address)))
    }

    /// Storage root of one account.
    pub fn storage_root(&self, address: &Address) -> B256 {
        self.storage
            .get(address)
            .map_or(xdevs_primitives::EMPTY_ROOT_HASH, root::storage_root)
    }

    /// Ethereum MPT state root.
    pub fn state_root(&self) -> B256 {
        root::state_root(&self.accounts, &self.storage)
    }
}

impl StateProvider for InMemoryState {
    fn basic_account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        Ok(self.accounts.get(&address).copied())
    }

    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        Ok(self
            .storage
            .get(&address)
            .and_then(|s| s.get(&slot))
            .copied()
            .unwrap_or_default())
    }

    fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytes>, StateError> {
        if code_hash == xdevs_primitives::KECCAK256_EMPTY {
            return Ok(Some(Bytes::new()));
        }
        Ok(self.codes.get(&code_hash).cloned())
    }

    fn block_hash(&self, number: u64) -> Result<Option<B256>, StateError> {
        Ok(self.block_hashes.get(&number).copied())
    }

    fn state_root_with(&self, diff: &StateDiff) -> Result<B256, StateError> {
        if diff.is_empty() {
            return Ok(self.state_root());
        }
        let mut next = self.clone();
        next.apply(diff);
        Ok(next.state_root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::{EMPTY_ROOT_HASH, KECCAK256_EMPTY};

    fn addr(b: u8) -> Address {
        Address::repeat_byte(b)
    }

    #[test]
    fn empty_state() {
        let s = InMemoryState::new();
        assert_eq!(s.state_root(), EMPTY_ROOT_HASH);
        assert_eq!(s.basic_account(addr(1)).unwrap(), None);
        assert_eq!(s.storage(addr(1), B256::ZERO).unwrap(), U256::ZERO);
        assert_eq!(s.code_by_hash(KECCAK256_EMPTY).unwrap(), Some(Bytes::new()));
    }

    #[test]
    fn apply_and_root_with_diff_agree() {
        let mut s = InMemoryState::new();
        s.insert_account(addr(1), AccountInfo::new(0, U256::from(100u64)));
        let mut d = StateDiff::new();
        d.set_account(addr(2), AccountInfo::new(1, U256::from(5u64)));
        let code = Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x55]);
        let ch = d.insert_code(code.clone());
        d.set_account(
            addr(3),
            AccountInfo {
                nonce: 1,
                balance: U256::ZERO,
                code_hash: ch,
            },
        );
        d.set_storage(addr(3), B256::with_last_byte(1), U256::from(7u64));

        let predicted = s.state_root_with(&d).unwrap();
        s.apply(&d);
        assert_eq!(s.state_root(), predicted);
        assert_eq!(s.code(addr(3)).unwrap(), code);
        assert_eq!(
            s.storage(addr(3), B256::with_last_byte(1)).unwrap(),
            U256::from(7u64)
        );
        assert_ne!(s.account(&addr(3)).unwrap().storage_root, EMPTY_ROOT_HASH);
    }

    #[test]
    fn destroy_removes_storage_and_restores_root() {
        let mut s = InMemoryState::new();
        s.insert_account(addr(1), AccountInfo::new(0, U256::from(1u64)));
        let before = s.state_root();

        let mut d = StateDiff::new();
        d.set_account(addr(9), AccountInfo::new(1, U256::ZERO));
        d.set_storage(addr(9), B256::with_last_byte(4), U256::from(4u64));
        s.apply(&d);
        assert_ne!(s.state_root(), before);

        let mut d = StateDiff::new();
        d.destroy_account(addr(9));
        s.apply(&d);
        assert_eq!(s.state_root(), before);
        assert!(s.account_storage(&addr(9)).is_none());
    }

    #[test]
    fn zero_write_deletes_slot() {
        let mut s = InMemoryState::new();
        s.insert_account(addr(1), AccountInfo::default());
        s.set_storage(addr(1), B256::with_last_byte(1), U256::from(1u64));
        s.set_storage(addr(1), B256::with_last_byte(1), U256::ZERO);
        assert!(s.account_storage(&addr(1)).is_none());
        assert_eq!(s.storage_root(&addr(1)), EMPTY_ROOT_HASH);
    }

    #[test]
    fn root_independent_of_insertion_order() {
        let mut a = InMemoryState::new();
        let mut b = InMemoryState::new();
        for i in 1..=20u8 {
            a.insert_account(addr(i), AccountInfo::new(i as u64, U256::from(i)));
        }
        for i in (1..=20u8).rev() {
            b.insert_account(addr(i), AccountInfo::new(i as u64, U256::from(i)));
        }
        assert_eq!(a.state_root(), b.state_root());
    }
}

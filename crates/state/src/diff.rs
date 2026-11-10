//! State changes produced by execution.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use xdevs_primitives::{keccak256, AccountInfo, Address, Bytes, B256, U256};

/// Storage changes of one account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDiff {
    /// Clear *all* existing storage of the account before applying `slots`
    /// (account destroyed, or re-created at the same address).
    pub wiped: bool,
    /// New slot values; `U256::ZERO` means "delete the slot".
    pub slots: BTreeMap<B256, U256>,
}

/// Deterministic set of world-state changes (plan §10.6: BTreeMap only).
///
/// Application order (see [`crate::InMemoryState::apply`] / [`crate::write_state_diff`]):
/// 1. `accounts`: `Some(info)` upserts, `None` deletes the account **and all its storage**.
/// 2. `storage`: per account, `wiped` clears existing storage, then `slots` are written
///    (zero = delete). Storage of accounts deleted in step 1 is ignored.
/// 3. `codes`: inserted (content-addressed, never deleted).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDiff {
    /// Changed accounts; `None` = destroyed / removed (EIP-161 / EIP-6780).
    pub accounts: BTreeMap<Address, Option<AccountInfo>>,
    /// Changed storage per account.
    pub storage: BTreeMap<Address, StorageDiff>,
    /// New bytecodes by code hash.
    pub codes: BTreeMap<B256, Bytes>,
}

impl StateDiff {
    /// Empty diff.
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` if the diff changes nothing.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.codes.is_empty()
    }

    /// Set (create or update) an account.
    pub fn set_account(&mut self, address: Address, info: AccountInfo) -> &mut Self {
        self.accounts.insert(address, Some(info));
        self
    }

    /// Destroy an account and all of its storage.
    pub fn destroy_account(&mut self, address: Address) -> &mut Self {
        self.accounts.insert(address, None);
        self.storage.insert(
            address,
            StorageDiff {
                wiped: true,
                slots: BTreeMap::new(),
            },
        );
        self
    }

    /// Set one storage slot (zero deletes it).
    pub fn set_storage(&mut self, address: Address, slot: B256, value: U256) -> &mut Self {
        self.storage
            .entry(address)
            .or_default()
            .slots
            .insert(slot, value);
        self
    }

    /// Add bytecode; returns its hash (to put in `AccountInfo::code_hash`).
    pub fn insert_code(&mut self, code: Bytes) -> B256 {
        let hash = keccak256(&code);
        self.codes.insert(hash, code);
        hash
    }

    /// Fold `later` (changes that happened after `self`) into `self`; later wins.
    pub fn merge(&mut self, later: StateDiff) {
        self.accounts.extend(later.accounts);
        for (address, sd) in later.storage {
            if sd.wiped {
                self.storage.insert(address, sd);
            } else {
                self.storage
                    .entry(address)
                    .or_default()
                    .slots
                    .extend(sd.slots);
            }
        }
        self.codes.extend(later.codes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_later_wins_and_wipe_resets() {
        let a = Address::repeat_byte(1);
        let mut d1 = StateDiff::new();
        d1.set_account(a, AccountInfo::new(1, U256::from(10u64)));
        d1.set_storage(a, B256::with_last_byte(1), U256::from(1u64));
        d1.set_storage(a, B256::with_last_byte(2), U256::from(2u64));

        let mut d2 = StateDiff::new();
        d2.set_storage(a, B256::with_last_byte(2), U256::from(3u64));
        d1.merge(d2);
        assert_eq!(d1.storage[&a].slots.len(), 2);
        assert_eq!(
            d1.storage[&a].slots[&B256::with_last_byte(2)],
            U256::from(3u64)
        );

        let mut d3 = StateDiff::new();
        d3.destroy_account(a);
        d1.merge(d3);
        assert_eq!(d1.accounts[&a], None);
        assert!(d1.storage[&a].wiped);
        assert!(d1.storage[&a].slots.is_empty());
    }

    #[test]
    fn insert_code_hashes() {
        let mut d = StateDiff::new();
        let code = Bytes::from_static(&[0x60, 0x00]);
        let h = d.insert_code(code.clone());
        assert_eq!(h, keccak256(&code));
        assert!(!d.is_empty());
    }
}

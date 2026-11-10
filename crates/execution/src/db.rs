//! revm [`Database`] adapter over any [`StateProvider`].
//!
//! [`StateOverlay`] is the bridge between revm and xDEVS state:
//!
//! * a **read cache** of the base state (`provider`) so repeated reads within a block do
//!   not hit storage again;
//! * a **write buffer** that is itself a [`StateDiff`]: every committed transaction is
//!   folded into it, later transactions read through it, and at the end of the block it is
//!   exactly the block's state diff (deterministic `BTreeMap`s, §10.6).

use revm::{
    bytecode::Bytecode,
    database_interface::DBErrorMarker,
    primitives::{AddressMap, StorageKey, StorageValue, KECCAK_EMPTY},
    state::{Account as RevmAccount, AccountInfo as RevmAccountInfo},
    Database,
};
use std::collections::BTreeMap;
use xdevs_primitives::{AccountInfo, Address, Bytes, B256, U256};
use xdevs_state::{StateDiff, StateProvider, StorageDiff};

/// Error surfaced to revm when the underlying state cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StateDbError(pub String);

impl DBErrorMarker for StateDbError {}

impl From<xdevs_state::StateError> for StateDbError {
    fn from(e: xdevs_state::StateError) -> Self {
        Self(e.to_string())
    }
}

/// Read-through cache + write buffer on top of a [`StateProvider`].
pub(crate) struct StateOverlay<'a> {
    provider: &'a dyn StateProvider,
    /// Changes committed so far (the write buffer).
    diff: StateDiff,
    /// Cached base-state accounts (`None` = does not exist).
    accounts: BTreeMap<Address, Option<AccountInfo>>,
    /// Cached base-state storage slots.
    storage: BTreeMap<(Address, B256), U256>,
}

impl<'a> StateOverlay<'a> {
    /// Empty overlay over `provider`.
    pub(crate) fn new(provider: &'a dyn StateProvider) -> Self {
        Self {
            provider,
            diff: StateDiff::new(),
            accounts: BTreeMap::new(),
            storage: BTreeMap::new(),
        }
    }

    /// The changes accumulated so far.
    pub(crate) fn into_diff(self) -> StateDiff {
        self.diff
    }

    /// Current (overlay-aware) account.
    pub(crate) fn account(
        &mut self,
        address: Address,
    ) -> Result<Option<AccountInfo>, StateDbError> {
        if let Some(acc) = self.diff.accounts.get(&address) {
            return Ok(*acc);
        }
        if let Some(acc) = self.accounts.get(&address) {
            return Ok(*acc);
        }
        let acc = self.provider.basic_account(address)?;
        self.accounts.insert(address, acc);
        Ok(acc)
    }

    /// Current (overlay-aware) storage slot.
    pub(crate) fn slot(&mut self, address: Address, slot: B256) -> Result<U256, StateDbError> {
        if let Some(sd) = self.diff.storage.get(&address) {
            if let Some(v) = sd.slots.get(&slot) {
                return Ok(*v);
            }
            if sd.wiped {
                return Ok(U256::ZERO);
            }
        }
        if matches!(self.diff.accounts.get(&address), Some(None)) {
            return Ok(U256::ZERO);
        }
        if let Some(v) = self.storage.get(&(address, slot)) {
            return Ok(*v);
        }
        let v = self.provider.storage(address, slot)?;
        self.storage.insert((address, slot), v);
        Ok(v)
    }

    /// Bytecode by hash, overlay first.
    fn code(&self, code_hash: B256) -> Result<Bytes, StateDbError> {
        if code_hash == KECCAK_EMPTY || code_hash == B256::ZERO {
            return Ok(Bytes::new());
        }
        if let Some(code) = self.diff.codes.get(&code_hash) {
            return Ok(code.clone());
        }
        self.provider
            .code_by_hash(code_hash)?
            .ok_or_else(|| StateDbError(format!("missing bytecode {code_hash}")))
    }

    /// Add `amount` wei to `address` (creates the account if needed). Used for the
    /// treasury share of the base fee (§15.1). Zero amounts are a no-op so no empty
    /// account is ever created.
    pub(crate) fn credit(&mut self, address: Address, amount: U256) -> Result<(), StateDbError> {
        if amount.is_zero() {
            return Ok(());
        }
        let mut info = self.account(address)?.unwrap_or_default();
        info.balance = info.balance.saturating_add(amount);
        self.diff.set_account(address, info);
        Ok(())
    }

    /// Fold one account's post-transaction state into the write buffer.
    fn commit_account(
        &mut self,
        address: Address,
        account: RevmAccount,
    ) -> Result<(), StateDbError> {
        if !account.is_touched() {
            return Ok(());
        }
        if account.is_selfdestructed() {
            self.diff.destroy_account(address);
            return Ok(());
        }
        let created = account.is_created();
        if !created && account.is_empty() {
            // EIP-161: touched empty accounts are removed.
            if !account.is_loaded_as_not_existing() && self.account(address)?.is_some() {
                self.diff.destroy_account(address);
            }
            return Ok(());
        }
        if created {
            // A (re)created contract starts with empty storage.
            self.diff.storage.insert(
                address,
                StorageDiff {
                    wiped: true,
                    slots: BTreeMap::new(),
                },
            );
        }

        let mut code_hash = account.info.code_hash;
        if code_hash == B256::ZERO {
            code_hash = KECCAK_EMPTY;
        }
        if code_hash != KECCAK_EMPTY && (created || code_hash != account.original_info().code_hash)
        {
            if let Some(code) = &account.info.code {
                self.diff.codes.insert(code_hash, code.original_bytes());
            }
        }

        let mut slots_changed = false;
        let mut changed: Vec<(B256, U256)> = account
            .changed_storage_slots()
            .map(|(k, s)| (B256::from(k.to_be_bytes::<32>()), s.present_value()))
            .collect();
        changed.sort_unstable_by_key(|(k, _)| *k);
        for (slot, value) in changed {
            slots_changed = true;
            self.diff.set_storage(address, slot, value);
        }

        let info = AccountInfo {
            nonce: account.info.nonce,
            balance: account.info.balance,
            code_hash,
        };
        if created || slots_changed || self.account(address)? != Some(info) {
            self.diff.set_account(address, info);
        }
        Ok(())
    }

    /// Commit revm's post-transaction state (sorted by address for determinism).
    pub(crate) fn try_commit(
        &mut self,
        changes: AddressMap<RevmAccount>,
    ) -> Result<(), StateDbError> {
        let mut changes: Vec<(Address, RevmAccount)> = changes.into_iter().collect();
        changes.sort_unstable_by_key(|(a, _)| *a);
        for (address, account) in changes {
            self.commit_account(address, account)?;
        }
        Ok(())
    }
}

/// Raw bytes to revm bytecode (EIP-7702 delegations are recognised; malformed `0xEF`
/// code falls back to legacy analysis instead of panicking).
fn to_bytecode(code: Bytes) -> Bytecode {
    if code.is_empty() {
        return Bytecode::new();
    }
    Bytecode::new_raw_checked(code.clone()).unwrap_or_else(|_| Bytecode::new_legacy(code))
}

impl Database for StateOverlay<'_> {
    type Error = StateDbError;

    fn basic(&mut self, address: Address) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let Some(acc) = self.account(address)? else {
            return Ok(None);
        };
        let code_hash = if acc.has_code() {
            acc.code_hash
        } else {
            KECCAK_EMPTY
        };
        let mut info = RevmAccountInfo::default().with_code_hash(code_hash);
        info.balance = acc.balance;
        info.nonce = acc.nonce;
        if code_hash != KECCAK_EMPTY {
            info = info.with_code_and_hash(to_bytecode(self.code(code_hash)?), code_hash);
        }
        Ok(Some(info))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(to_bytecode(self.code(code_hash)?))
    }

    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        self.slot(address, B256::from(index.to_be_bytes::<32>()))
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(self.provider.block_hash(number)?.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_state::InMemoryState;

    #[test]
    fn reads_through_and_buffers_writes() {
        let a = Address::repeat_byte(1);
        let mut base = InMemoryState::new();
        base.insert_account(a, AccountInfo::new(3, U256::from(100u64)));
        base.set_storage(a, B256::with_last_byte(1), U256::from(7u64));

        let mut o = StateOverlay::new(&base);
        let info = o.basic(a).unwrap().unwrap();
        assert_eq!(info.nonce, 3);
        assert_eq!(o.storage(a, U256::from(1u64)).unwrap(), U256::from(7u64));
        assert!(o.basic(Address::repeat_byte(2)).unwrap().is_none());

        o.credit(a, U256::from(5u64)).unwrap();
        o.credit(Address::repeat_byte(3), U256::ZERO).unwrap();
        assert_eq!(o.basic(a).unwrap().unwrap().balance, U256::from(105u64));

        o.diff.destroy_account(a);
        assert!(o.basic(a).unwrap().is_none());
        assert_eq!(o.storage(a, U256::from(1u64)).unwrap(), U256::ZERO);
        let diff = o.into_diff();
        assert_eq!(diff.accounts.len(), 1);
        assert!(diff.storage[&a].wiped);
    }

    #[test]
    fn code_is_loaded_with_account() {
        let a = Address::repeat_byte(1);
        let mut base = InMemoryState::new();
        let code = Bytes::from_static(&[0x60, 0x00, 0x00]);
        let hash = base.insert_code(code.clone());
        base.insert_account(
            a,
            AccountInfo {
                nonce: 1,
                balance: U256::ZERO,
                code_hash: hash,
            },
        );
        let mut o = StateOverlay::new(&base);
        let info = o.basic(a).unwrap().unwrap();
        assert_eq!(info.code_hash, hash);
        assert_eq!(info.code.unwrap().original_bytes(), code);
        assert_eq!(o.code_by_hash(hash).unwrap().original_bytes(), code);
        assert!(o.code_by_hash(B256::repeat_byte(9)).is_err());
    }
}

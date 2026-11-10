//! World state for xDEVS Chain (plan §14).
//!
//! * [`StateProvider`] — read interface the executor, mempool and RPC use.
//! * [`StateDiff`] — deterministic (BTreeMap) set of changes produced by executing a
//!   block; applied to [`InMemoryState`] or written to storage with [`write_state_diff`].
//! * [`InMemoryState`] — complete in-memory state with Ethereum MPT root, for tests.
//! * [`DbStateProvider`] — latest state read from the flat `PlainAccounts` /
//!   `PlainStorage` / `Bytecodes` tables.
//! * [`root`] — Ethereum state/storage root computation (alloy-trie).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod db;
mod diff;
mod memory;
pub mod root;

pub use db::{write_state_diff, DbStateProvider};
pub use diff::{StateDiff, StorageDiff};
pub use memory::InMemoryState;

use std::sync::Arc;
use xdevs_primitives::{AccountInfo, Address, Bytes, B256, U256};
use xdevs_storage::DbError;

/// Errors from state access.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Underlying storage failed.
    #[error(transparent)]
    Db(#[from] DbError),
    /// State is inconsistent (e.g. code hash without bytecode).
    #[error("inconsistent state: {0}")]
    Inconsistent(String),
    /// Requested historical state is not available (pruned / not supported yet).
    #[error("state not available: {0}")]
    Unavailable(String),
}

/// Read access to the world state at one point (usually "after block N").
///
/// Implementations must be deterministic. They are *not* required to be
/// `Send + Sync`; use `dyn StateProvider + Send + Sync` where that is needed.
pub trait StateProvider {
    /// Flat account (`None` if the account does not exist).
    fn basic_account(&self, address: Address) -> Result<Option<AccountInfo>, StateError>;

    /// Storage value of `slot` (zero if unset).
    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError>;

    /// Bytecode by `keccak256(code)`.
    fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytes>, StateError>;

    /// Hash of canonical block `number` (for the `BLOCKHASH` opcode; only the last 256
    /// blocks are required).
    fn block_hash(&self, number: u64) -> Result<Option<B256>, StateError>;

    /// Ethereum MPT state root of this state with `diff` applied on top
    /// (`diff = StateDiff::default()` gives the current root).
    fn state_root_with(&self, diff: &StateDiff) -> Result<B256, StateError>;

    /// Convenience: code of an account (empty if none).
    fn code(&self, address: Address) -> Result<Bytes, StateError> {
        match self.basic_account(address)? {
            Some(acc) if acc.has_code() => {
                Ok(self.code_by_hash(acc.code_hash)?.unwrap_or_default())
            }
            _ => Ok(Bytes::new()),
        }
    }
}

macro_rules! forward_provider {
    ($($ty:ty),*) => {$(
        impl<T: StateProvider + ?Sized> StateProvider for $ty {
            fn basic_account(&self, a: Address) -> Result<Option<AccountInfo>, StateError> {
                (**self).basic_account(a)
            }
            fn storage(&self, a: Address, s: B256) -> Result<U256, StateError> {
                (**self).storage(a, s)
            }
            fn code_by_hash(&self, h: B256) -> Result<Option<Bytes>, StateError> {
                (**self).code_by_hash(h)
            }
            fn block_hash(&self, n: u64) -> Result<Option<B256>, StateError> {
                (**self).block_hash(n)
            }
            fn state_root_with(&self, d: &StateDiff) -> Result<B256, StateError> {
                (**self).state_root_with(d)
            }
        }
    )*};
}
forward_provider!(&T, Box<T>, Arc<T>);

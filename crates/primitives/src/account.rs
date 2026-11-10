//! Account types (plan §8.3).

use alloy_primitives::{B256, KECCAK256_EMPTY, U256};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use alloy_trie::{TrieAccount, EMPTY_ROOT_HASH};
use serde::{Deserialize, Serialize};

/// Flat-state account: what is stored per address in `PlainAccounts` and what
/// [`StateProvider::basic_account`](../xdevs_state/trait.StateProvider.html) returns.
///
/// It deliberately has no `storage_root`: the storage root is derived from the
/// account's storage slots when the state root is computed. Use [`Account`] when the
/// full Ethereum account (with storage root) is needed (state trie, `eth_getProof`).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    /// Number of transactions sent (EOA) or contracts created (contract).
    pub nonce: u64,
    /// Balance in wei.
    pub balance: U256,
    /// `keccak256(code)`; [`KECCAK256_EMPTY`] for accounts without code.
    pub code_hash: B256,
}

impl Default for AccountInfo {
    fn default() -> Self {
        Self {
            nonce: 0,
            balance: U256::ZERO,
            code_hash: KECCAK256_EMPTY,
        }
    }
}

impl AccountInfo {
    /// A code-less account with the given balance and nonce.
    pub const fn new(nonce: u64, balance: U256) -> Self {
        Self {
            nonce,
            balance,
            code_hash: KECCAK256_EMPTY,
        }
    }

    /// `true` if the account has code.
    pub fn has_code(&self) -> bool {
        self.code_hash != KECCAK256_EMPTY && self.code_hash != B256::ZERO
    }

    /// EIP-161 "empty": zero nonce, zero balance and no code.
    pub fn is_empty(&self) -> bool {
        self.nonce == 0 && self.balance.is_zero() && !self.has_code()
    }

    /// Combine with a storage root into a full Ethereum [`Account`].
    pub const fn with_storage_root(self, storage_root: B256) -> Account {
        Account {
            nonce: self.nonce,
            balance: self.balance,
            code_hash: self.code_hash,
            storage_root,
        }
    }
}

/// A full Ethereum account `{ nonce, balance, code_hash, storage_root }` (plan §8.3),
/// i.e. the value stored in the state trie under `keccak256(address)`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Account nonce.
    pub nonce: u64,
    /// Balance in wei.
    pub balance: U256,
    /// `keccak256(code)`.
    pub code_hash: B256,
    /// Root of the account's storage trie ([`EMPTY_ROOT_HASH`] when it has no storage).
    pub storage_root: B256,
}

impl Default for Account {
    fn default() -> Self {
        AccountInfo::default().with_storage_root(EMPTY_ROOT_HASH)
    }
}

impl Account {
    /// The flat-state part of this account.
    pub const fn info(&self) -> AccountInfo {
        AccountInfo {
            nonce: self.nonce,
            balance: self.balance,
            code_hash: self.code_hash,
        }
    }
}

impl From<Account> for TrieAccount {
    fn from(a: Account) -> Self {
        TrieAccount {
            nonce: a.nonce,
            balance: a.balance,
            storage_root: a.storage_root,
            code_hash: a.code_hash,
        }
    }
}

impl From<TrieAccount> for Account {
    fn from(a: TrieAccount) -> Self {
        Account {
            nonce: a.nonce,
            balance: a.balance,
            code_hash: a.code_hash,
            storage_root: a.storage_root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_account_is_empty() {
        let a = AccountInfo::default();
        assert!(a.is_empty());
        assert!(!a.has_code());
        assert_eq!(Account::default().storage_root, EMPTY_ROOT_HASH);
    }

    #[test]
    fn account_rlp_matches_trie_account() {
        // The trie encodes [nonce, balance, storage_root, code_hash]; our `Account`
        // field order differs, so the trie must always go through `TrieAccount`.
        let acc = AccountInfo::new(7, U256::from(1_000u64)).with_storage_root(EMPTY_ROOT_HASH);
        let trie: TrieAccount = acc.into();
        assert_eq!(Account::from(trie), acc);
    }

    #[test]
    fn account_info_rlp_roundtrip() {
        let info = AccountInfo::new(3, U256::from(42u64));
        let enc = alloy_rlp::encode(info);
        let dec: AccountInfo = alloy_rlp::Decodable::decode(&mut enc.as_slice()).unwrap();
        assert_eq!(dec, info);
    }
}

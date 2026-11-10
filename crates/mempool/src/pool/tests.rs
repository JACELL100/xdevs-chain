use super::*;
use crate::validation::{intrinsic_gas, min_max_fee};
use xdevs_primitives::{
    constants::GWEI,
    reexports::alloy_consensus::{SignableTransaction, TxEip4844},
    reexports::alloy_eips::eip2930::{AccessList, AccessListItem},
    reexports::alloy_eips::eip7702::Authorization,
    xip0001::{DESIGNATOR_HASH, LOCK_SLOT},
    Bytes, Header, Signature, TxEip1559, TxEip2930, TxEip7702, TxEnvelope, TxKind, TxLegacy, B256,
};

const CHAIN: u64 = 7117;
const BASE: u64 = GWEI;
const RICH: u128 = 1_000_000_000_000_000_000_000; // 1000 XDS

type Accounts = Arc<RwLock<BTreeMap<Address, AccountInfo>>>;
type Slots = Arc<RwLock<BTreeMap<(Address, B256), U256>>>;

/// Test state: accounts plus storage slots (for the XIP-0001 lock word).
struct TestReader {
    accounts: Accounts,
    slots: Slots,
}

impl AccountReader for TestReader {
    fn account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        Ok(self.accounts.read().get(&address).copied())
    }
    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        Ok(self
            .slots
            .read()
            .get(&(address, slot))
            .copied()
            .unwrap_or_default())
    }
}

fn addr(i: u8) -> Address {
    Address::with_last_byte(i)
}

fn sig() -> Signature {
    Signature::new(U256::from(1), U256::from(2), false)
}

fn to() -> TxKind {
    TxKind::Call(Address::repeat_byte(0xee))
}

/// EIP-1559 transfer from `sender` (signature is fake; the sender is attached unchecked).
fn tx1559(sender: Address, nonce: u64, max_fee: u128, tip: u128, gas: u64) -> RecoveredTx {
    let tx = TxEip1559 {
        chain_id: CHAIN,
        nonce,
        gas_limit: gas,
        max_fee_per_gas: max_fee,
        max_priority_fee_per_gas: tip,
        to: to(),
        // The signature is fake, so make the hash sender-specific through the value.
        value: U256::from_be_slice(sender.as_slice()),
        access_list: AccessList::default(),
        input: Bytes::new(),
    };
    RecoveredTx::new_unchecked(TxEnvelope::from(tx.into_signed(sig())), sender)
}

fn transfer(sender: Address, nonce: u64, max_fee: u128, tip: u128) -> RecoveredTx {
    tx1559(sender, nonce, max_fee, tip, 21_000)
}

fn legacy(sender: Address, nonce: u64, chain_id: Option<u64>, gas_price: u128) -> RecoveredTx {
    let tx = TxLegacy {
        chain_id,
        nonce,
        gas_price,
        gas_limit: 21_000,
        to: to(),
        value: U256::ZERO,
        input: Bytes::new(),
    };
    RecoveredTx::new_unchecked(TxEnvelope::from(tx.into_signed(sig())), sender)
}

struct Fixture {
    pool: Pool,
    accounts: Accounts,
    slots: Slots,
}

impl Fixture {
    fn with_params(config: PoolConfig, params: PoolParams) -> Self {
        let accounts: Accounts = Arc::default();
        let slots: Slots = Arc::default();
        let reader = TestReader {
            accounts: accounts.clone(),
            slots: slots.clone(),
        };
        let pool = Pool::new(params, config, Arc::new(reader), BASE);
        Self {
            pool,
            accounts,
            slots,
        }
    }

    fn with_config(config: PoolConfig) -> Self {
        let params = PoolParams::from_chain_config(&xdevs_primitives::ChainConfig::dev());
        Self::with_params(config, params)
    }

    /// Make `a` an EIP-7702 account delegated to `0x…1005` with lock word `word`.
    fn delegate(&self, a: Address, nonce: u64, word: u64) {
        let mut info = AccountInfo::new(nonce, U256::from(RICH));
        info.code_hash = DESIGNATOR_HASH;
        self.accounts.write().insert(a, info);
        self.set_word(a, word);
    }

    fn set_word(&self, a: Address, word: u64) {
        self.slots.write().insert((a, LOCK_SLOT), U256::from(word));
    }

    fn new() -> Self {
        Self::with_config(PoolConfig::default())
    }

    fn fund(&self, a: Address, nonce: u64, balance: u128) {
        self.accounts
            .write()
            .insert(a, AccountInfo::new(nonce, U256::from(balance)));
    }

    fn add(&self, tx: RecoveredTx) -> Result<TxHash, PoolError> {
        self.pool.add(tx, TxOrigin::Local)
    }

    fn commit(&self, txs: Vec<TxEnvelope>, timestamp: u64) {
        let header = Header {
            timestamp,
            base_fee_per_gas: Some(BASE),
            ..Header::default()
        };
        self.pool.on_block_committed(&Block::new(header, txs), &[]);
    }
}

fn hashes(txs: &[RecoveredTx]) -> Vec<TxHash> {
    txs.iter().map(RecoveredTx::hash).collect()
}

const FEE: u128 = 2 * GWEI as u128;

// ---------------------------------------------------------------------------
// Entry validation
// ---------------------------------------------------------------------------

#[test]
fn add_get_contains_and_already_known() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let tx = transfer(addr(1), 0, FEE, GWEI as u128);
    let h = f.add(tx.clone()).unwrap();
    assert_eq!(h, tx.hash());
    assert!(f.pool.contains(&h));
    assert_eq!(f.pool.get(&h), Some(tx.clone()));
    assert_eq!(f.add(tx).unwrap_err(), PoolError::AlreadyKnown);
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 1,
            queued: 0
        }
    );
    assert_eq!(f.pool.pending_nonce(&addr(1)), Some(1));
    assert_eq!(f.pool.pending_nonce(&addr(2)), None);
    assert_eq!(f.pool.total_bytes(), f.pool.get(&h).unwrap().encoded_len());
}

#[test]
fn rejects_unsupported_types() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let blob = TxEip4844 {
        chain_id: CHAIN,
        nonce: 0,
        gas_limit: 21_000,
        max_fee_per_gas: FEE,
        max_priority_fee_per_gas: 1,
        to: Address::repeat_byte(1),
        value: U256::ZERO,
        access_list: AccessList::default(),
        blob_versioned_hashes: vec![B256::repeat_byte(1)],
        max_fee_per_blob_gas: 1,
        input: Bytes::new(),
    };
    let env = TxEnvelope::from(blob.into_signed(sig()));
    assert_eq!(
        f.add(RecoveredTx::new_unchecked(env, addr(1))).unwrap_err(),
        PoolError::UnsupportedType(3)
    );

    // EIP-7702 is supported, but needs a non-empty authorization list.
    assert_eq!(
        f.add(set_code(addr(1), 0, Address::repeat_byte(1), 0, 100_000))
            .unwrap_err(),
        PoolError::EmptyAuthorizationList
    );
    // Intrinsic gas counts 25,000 per authorization.
    assert!(matches!(
        f.add(set_code(addr(1), 0, Address::repeat_byte(1), 2, 70_000)),
        Err(PoolError::IntrinsicGasTooLow {
            intrinsic: 71_000,
            ..
        })
    ));
    let h = f
        .add(set_code(addr(1), 0, Address::repeat_byte(1), 2, 71_000))
        .unwrap();
    assert_eq!(f.pool.get(&h).unwrap().tx_type_byte(), 4);
}

/// Type-4 tx with `auths` dummy authorizations (fake signatures).
fn set_code(sender: Address, nonce: u64, to: Address, auths: usize, gas: u64) -> RecoveredTx {
    let auth = TxEip7702 {
        chain_id: CHAIN,
        nonce,
        gas_limit: gas,
        max_fee_per_gas: FEE,
        max_priority_fee_per_gas: 1,
        to,
        value: U256::ZERO,
        access_list: AccessList::default(),
        authorization_list: (0..auths)
            .map(|i| {
                Authorization {
                    chain_id: U256::from(CHAIN),
                    address: Address::repeat_byte(0x10 + i as u8),
                    nonce: 0,
                }
                .into_signed(sig())
            })
            .collect(),
        input: Bytes::new(),
    };
    RecoveredTx::new_unchecked(TxEnvelope::from(auth.into_signed(sig())), sender)
}

#[test]
fn chain_id_and_replay_protection() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    assert_eq!(
        f.add(legacy(addr(1), 0, None, FEE)).unwrap_err(),
        PoolError::WrongChainId {
            expected: CHAIN,
            got: None
        }
    );
    assert_eq!(
        f.add(legacy(addr(1), 0, Some(1), FEE)).unwrap_err(),
        PoolError::WrongChainId {
            expected: CHAIN,
            got: Some(1)
        }
    );
    f.add(legacy(addr(1), 0, Some(CHAIN), FEE)).unwrap();

    let al = TxEip2930 {
        chain_id: 5,
        nonce: 1,
        gas_price: FEE,
        gas_limit: 30_000,
        to: to(),
        value: U256::ZERO,
        access_list: AccessList::default(),
        input: Bytes::new(),
    };
    let env = TxEnvelope::from(al.into_signed(sig()));
    assert!(matches!(
        f.add(RecoveredTx::new_unchecked(env, addr(1))),
        Err(PoolError::WrongChainId { got: Some(5), .. })
    ));
}

#[test]
fn oversized_rejected() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let mut tx = TxEip1559 {
        chain_id: CHAIN,
        nonce: 0,
        gas_limit: 25_000_000,
        max_fee_per_gas: FEE,
        max_priority_fee_per_gas: 0,
        to: to(),
        value: U256::ZERO,
        access_list: AccessList::default(),
        input: Bytes::from(vec![1u8; 128 * 1024]),
    };
    let env = TxEnvelope::from(tx.clone().into_signed(sig()));
    assert!(matches!(
        f.add(RecoveredTx::new_unchecked(env, addr(1))),
        Err(PoolError::Oversized { max: 131_072, .. })
    ));
    // Just under the limit is fine size-wise.
    tx.input = Bytes::from(vec![1u8; 120 * 1024]);
    let env = TxEnvelope::from(tx.into_signed(sig()));
    f.add(RecoveredTx::new_unchecked(env, addr(1))).unwrap();
}

#[test]
fn gas_checks() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    assert_eq!(
        f.add(tx1559(addr(1), 0, FEE, 1, 20_999)).unwrap_err(),
        PoolError::IntrinsicGasTooLow {
            gas_limit: 20_999,
            intrinsic: 21_000
        }
    );
    assert_eq!(
        f.add(tx1559(addr(1), 0, FEE, 1, 30_000_001)).unwrap_err(),
        PoolError::GasLimitExceedsBlock
    );
    assert!(matches!(
        f.add(tx1559(addr(1), 0, FEE, FEE + 1, 21_000)),
        Err(PoolError::TipAboveFeeCap { .. })
    ));
    f.add(tx1559(addr(1), 0, FEE, FEE, 30_000_000)).unwrap();
}

#[test]
fn per_tx_gas_cap_when_configured() {
    let accounts: Accounts = Arc::default();
    accounts
        .write()
        .insert(addr(1), AccountInfo::new(0, U256::from(RICH)));
    let reader = move |a: Address| -> Result<Option<AccountInfo>, StateError> {
        Ok(accounts.read().get(&a).copied())
    };
    let mut params = PoolParams::from_chain_config(&xdevs_primitives::ChainConfig::dev());
    params.max_tx_gas = Some(1 << 24);
    let pool = Pool::new(params, PoolConfig::default(), Arc::new(reader), BASE);
    assert_eq!(
        pool.add(tx1559(addr(1), 0, FEE, 1, (1 << 24) + 1), TxOrigin::Local)
            .unwrap_err(),
        PoolError::GasLimitTooHigh {
            gas_limit: (1 << 24) + 1,
            cap: 1 << 24
        }
    );
    pool.add(tx1559(addr(1), 0, FEE, 1, 1 << 24), TxOrigin::Local)
        .unwrap();
}

#[test]
fn intrinsic_gas_rules() {
    let call = |input: Vec<u8>, kind: TxKind, al: AccessList| {
        TxEnvelope::from(
            TxEip1559 {
                chain_id: CHAIN,
                to: kind,
                input: input.into(),
                access_list: al,
                ..TxEip1559::default()
            }
            .into_signed(sig()),
        )
    };
    // Plain transfer.
    assert_eq!(
        intrinsic_gas(&call(vec![], to(), AccessList::default())),
        21_000
    );
    // 2 zero + 2 non-zero bytes: 21000 + 8 + 32 = 21040; floor = 21000 + 10*(2+8) = 21100.
    assert_eq!(
        intrinsic_gas(&call(vec![0, 0, 1, 2], to(), AccessList::default())),
        21_100
    );
    // Large non-zero calldata: standard 21000+16n vs floor 21000+40n -> floor wins.
    assert_eq!(
        intrinsic_gas(&call(vec![1; 100], to(), AccessList::default())),
        21_000 + 4_000
    );
    // Create with 33 bytes of zero init code: 21000 + 132 + 32000 + 2*2 = 53136.
    assert_eq!(
        intrinsic_gas(&call(vec![0; 33], TxKind::Create, AccessList::default())),
        53_136
    );
    // Access list: 1 address with 2 keys.
    let al = AccessList(vec![AccessListItem {
        address: addr(9),
        storage_keys: vec![B256::ZERO, B256::repeat_byte(1)],
    }]);
    assert_eq!(
        intrinsic_gas(&call(vec![], to(), al)),
        21_000 + 2_400 + 3_800
    );
}

#[test]
fn init_code_size_limit() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let tx = TxEip1559 {
        chain_id: CHAIN,
        gas_limit: 10_000_000,
        max_fee_per_gas: FEE,
        to: TxKind::Create,
        input: vec![1u8; 49_153].into(),
        ..TxEip1559::default()
    };
    let env = TxEnvelope::from(tx.into_signed(sig()));
    assert_eq!(
        f.add(RecoveredTx::new_unchecked(env, addr(1))).unwrap_err(),
        PoolError::InitCodeTooLarge {
            size: 49_153,
            max: 49_152
        }
    );
}

#[test]
fn underpriced_threshold_is_ninety_percent_of_base_fee() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let min = min_max_fee(BASE);
    assert_eq!(min, 900_000_000);
    assert_eq!(
        f.add(transfer(addr(1), 0, min - 1, 0)).unwrap_err(),
        PoolError::Underpriced {
            max_fee: min - 1,
            required: min
        }
    );
    f.add(transfer(addr(1), 0, min, 0)).unwrap();
    assert_eq!(min_max_fee(7), 7); // ceil(6.3)
    f.pool.set_base_fee(10 * GWEI);
    assert_eq!(f.pool.base_fee(), 10 * GWEI);
    assert!(matches!(
        f.add(transfer(addr(1), 1, FEE, 0)),
        Err(PoolError::Underpriced { .. })
    ));
}

#[test]
fn nonce_rules() {
    let f = Fixture::new();
    f.fund(addr(1), 5, RICH);
    assert_eq!(
        f.add(transfer(addr(1), 4, FEE, 1)).unwrap_err(),
        PoolError::NonceTooLow { state: 5, tx: 4 }
    );
    assert_eq!(
        f.add(transfer(addr(1), 5 + 65, FEE, 1)).unwrap_err(),
        PoolError::NonceGapTooLarge { state: 5, tx: 70 }
    );
    f.add(transfer(addr(1), 5 + 64, FEE, 1)).unwrap();
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 0,
            queued: 1
        }
    );
    assert_eq!(f.pool.pending_nonce(&addr(1)), None);
}

#[test]
fn unknown_sender_has_nothing() {
    let f = Fixture::new();
    assert!(matches!(
        f.add(transfer(addr(7), 0, FEE, 1)),
        Err(PoolError::InsufficientFunds { balance, .. }) if balance.is_zero()
    ));
}

/// Regression (security review, tx-gossip flood): txs from unfunded senders and
/// stale / far-future nonces are rejected from a state read alone, without waiting for
/// the pool's write lock (blocked here by a read lock held for the whole check).
#[test]
fn cheap_rejections_do_not_take_the_write_lock() {
    let f = Fixture::new();
    f.fund(addr(1), 5, RICH);
    // A held read lock blocks writers, not readers.
    let guard = f.pool.inner.read();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(|| {
            let results = vec![
                f.add(transfer(addr(9), 0, FEE, 1)),
                f.add(transfer(addr(1), 4, FEE, 1)),
                f.add(transfer(addr(1), 5 + 1_000, FEE, 1)),
            ];
            tx.send(results).unwrap();
        });
        let results = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("rejections waited for the write lock");
        drop(guard);
        assert!(matches!(
            results[0],
            Err(PoolError::InsufficientFunds { .. })
        ));
        assert!(matches!(results[1], Err(PoolError::NonceTooLow { .. })));
        assert!(matches!(
            results[2],
            Err(PoolError::NonceGapTooLarge { .. })
        ));
    });
}

#[test]
fn insufficient_funds_is_cumulative() {
    let f = Fixture::new();
    let cost = 21_000 * FEE + 1;
    f.fund(addr(1), 0, 2 * cost);
    f.add(transfer(addr(1), 0, FEE, 1)).unwrap();
    f.add(transfer(addr(1), 1, FEE, 1)).unwrap();
    assert_eq!(
        f.add(transfer(addr(1), 2, FEE, 1)).unwrap_err(),
        PoolError::InsufficientFunds {
            balance: U256::from(2 * cost),
            cost: U256::from(3 * cost)
        }
    );
}

#[test]
fn sender_limit() {
    let f = Fixture::with_config(PoolConfig {
        max_per_sender: 3,
        ..PoolConfig::default()
    });
    f.fund(addr(1), 0, RICH);
    for n in 0..3 {
        f.add(transfer(addr(1), n, FEE, 1)).unwrap();
    }
    assert_eq!(
        f.add(transfer(addr(1), 3, FEE, 1)).unwrap_err(),
        PoolError::SenderLimit(addr(1))
    );
    // Replacing does not count against the limit.
    f.add(transfer(addr(1), 2, 2 * FEE, 2)).unwrap();
}

// ---------------------------------------------------------------------------
// Replacement, pending / queued, promotion
// ---------------------------------------------------------------------------

#[test]
fn replacement_needs_ten_percent_bump_on_fee_and_tip() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let old = transfer(addr(1), 0, 10 * GWEI as u128, GWEI as u128);
    let old_hash = f.add(old).unwrap();

    // +9% fee and tip: rejected.
    assert_eq!(
        f.add(transfer(addr(1), 0, 10_900_000_000, 1_090_000_000))
            .unwrap_err(),
        PoolError::ReplacementUnderpriced
    );
    // +10% fee but same tip: rejected.
    assert_eq!(
        f.add(transfer(addr(1), 0, 11 * GWEI as u128, GWEI as u128))
            .unwrap_err(),
        PoolError::ReplacementUnderpriced
    );
    // +10% on both: accepted, old one gone.
    let new = transfer(addr(1), 0, 11 * GWEI as u128, 1_100_000_000);
    let new_hash = f.add(new.clone()).unwrap();
    assert!(!f.pool.contains(&old_hash));
    assert_eq!(f.pool.get(&new_hash), Some(new.clone()));
    assert_eq!(f.pool.len(), 1);
    assert_eq!(f.pool.total_bytes(), new.encoded_len());
    assert_eq!(f.pool.pending(), vec![new]);
}

#[test]
fn legacy_replacement() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    f.add(legacy(addr(1), 0, Some(CHAIN), 10 * GWEI as u128))
        .unwrap();
    assert_eq!(
        f.add(legacy(addr(1), 0, Some(CHAIN), 10_999_999_999))
            .unwrap_err(),
        PoolError::ReplacementUnderpriced
    );
    f.add(legacy(addr(1), 0, Some(CHAIN), 11 * GWEI as u128))
        .unwrap();
    assert_eq!(f.pool.len(), 1);
}

#[test]
fn gap_fill_promotes_queued() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    f.add(transfer(addr(1), 1, FEE, 1)).unwrap();
    f.add(transfer(addr(1), 2, FEE, 1)).unwrap();
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 0,
            queued: 2
        }
    );
    assert!(f.pool.pending().is_empty());
    assert_eq!(f.pool.queued().len(), 2);
    assert!(f.pool.best_txs(BASE, 30_000_000).is_empty());

    f.add(transfer(addr(1), 0, FEE, 1)).unwrap();
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 3,
            queued: 0
        }
    );
    assert_eq!(f.pool.pending_nonce(&addr(1)), Some(3));
    let nonces: Vec<u64> = f.pool.pending().iter().map(|t| t.nonce()).collect();
    assert_eq!(nonces, vec![0, 1, 2]);
}

#[test]
fn block_commit_removes_included_and_promotes() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let t0 = transfer(addr(1), 0, FEE, 1);
    f.add(t0.clone()).unwrap();
    f.add(transfer(addr(1), 2, FEE, 1)).unwrap();
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 1,
            queued: 1
        }
    );

    // Block includes t0 and (from elsewhere) the account's nonce-1 tx.
    f.fund(addr(1), 2, RICH);
    f.commit(vec![t0.tx.clone()], 100);
    assert!(!f.pool.contains(&t0.hash()));
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 1,
            queued: 0
        }
    );
    assert_eq!(f.pool.pending_nonce(&addr(1)), Some(3));
    assert_eq!(f.pool.pending()[0].nonce(), 2);
}

#[test]
fn block_commit_drops_stale_nonces_and_unaffordable() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    f.fund(addr(2), 0, RICH);
    let a0 = transfer(addr(1), 0, FEE, 1);
    let a1 = transfer(addr(1), 1, FEE, 1);
    let b0 = transfer(addr(2), 0, FEE, 1);
    for t in [&a0, &a1, &b0] {
        f.add(t.clone()).unwrap();
    }
    // addr(1)'s nonce 0 was mined via another node; addr(2) got drained.
    f.fund(addr(1), 1, RICH);
    f.fund(addr(2), 0, 1);
    f.commit(vec![], 10);
    assert!(!f.pool.contains(&a0.hash()));
    assert!(f.pool.contains(&a1.hash()));
    assert!(!f.pool.contains(&b0.hash()));
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 1,
            queued: 0
        }
    );
    assert_eq!(f.pool.base_fee(), BASE);
}

#[test]
fn add_refreshes_sender_state() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let t0 = transfer(addr(1), 0, FEE, 1);
    f.add(t0.clone()).unwrap();
    // Chain advanced but the pool was not told yet.
    f.fund(addr(1), 1, RICH);
    f.add(transfer(addr(1), 1, FEE, 1)).unwrap();
    assert!(!f.pool.contains(&t0.hash()));
    assert_eq!(f.pool.pending_nonce(&addr(1)), Some(2));
}

#[test]
fn remove_demotes_later_nonces() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let t0 = transfer(addr(1), 0, FEE, 1);
    f.add(t0.clone()).unwrap();
    f.add(transfer(addr(1), 1, FEE, 1)).unwrap();
    f.pool.remove(&[t0.hash(), TxHash::repeat_byte(9)]);
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 0,
            queued: 1
        }
    );
    assert_eq!(f.pool.total_bytes(), f.pool.queued()[0].encoded_len());
}

#[test]
fn queued_ttl_uses_block_timestamps() {
    let f = Fixture::with_config(PoolConfig {
        queued_ttl_secs: 100,
        ..PoolConfig::default()
    });
    f.fund(addr(1), 0, RICH);
    f.fund(addr(2), 0, RICH);
    let early = transfer(addr(1), 5, FEE, 1); // queued, arrives before any block
    let pending = transfer(addr(2), 0, FEE, 1);
    f.add(early.clone()).unwrap();
    f.add(pending.clone()).unwrap();

    f.commit(vec![], 1_000); // stamps `early` with 1000
    let late = transfer(addr(1), 7, FEE, 1);
    f.commit(vec![], 1_050);
    f.add(late.clone()).unwrap(); // stamped 1050

    f.commit(vec![], 1_100); // 1000 + 100 is not < 1100: kept
    assert!(f.pool.contains(&early.hash()));
    f.commit(vec![], 1_101);
    assert!(!f.pool.contains(&early.hash()));
    assert!(f.pool.contains(&late.hash()));
    // Pending txs never expire.
    f.commit(vec![], 10_000);
    assert!(f.pool.contains(&pending.hash()));
    assert!(!f.pool.contains(&late.hash()));
}

// ---------------------------------------------------------------------------
// Global cap & eviction
// ---------------------------------------------------------------------------

#[test]
fn full_pool_evicts_cheapest_tail() {
    let f = Fixture::with_config(PoolConfig {
        max_txs: 3,
        ..PoolConfig::default()
    });
    for i in 1..=4 {
        f.fund(addr(i), 0, RICH);
    }
    let a0 = transfer(addr(1), 0, 2 * GWEI as u128, 1);
    let a1 = transfer(addr(1), 1, 2 * GWEI as u128, 1);
    let b0 = transfer(addr(2), 0, 3 * GWEI as u128, 1);
    for t in [&a0, &a1, &b0] {
        f.add(t.clone()).unwrap();
    }
    // Not more than the cheapest: refused.
    assert_eq!(
        f.add(transfer(addr(3), 0, 2 * GWEI as u128, 1))
            .unwrap_err(),
        PoolError::PoolFull
    );
    // Pays more: evicts addr(1)'s tail (nonce 1), never its nonce 0.
    let c0 = transfer(addr(3), 0, 4 * GWEI as u128, 1);
    f.add(c0.clone()).unwrap();
    assert_eq!(f.pool.len(), 3);
    assert!(f.pool.contains(&a0.hash()));
    assert!(!f.pool.contains(&a1.hash()));
    // Next eviction among tails {a0: 2, b0: 3, c0: 4}: a0.
    f.add(transfer(addr(4), 0, 5 * GWEI as u128, 1)).unwrap();
    assert!(!f.pool.contains(&a0.hash()));
    assert!(f.pool.contains(&b0.hash()));
    assert_eq!(f.pool.status().pending, 3);
}

#[test]
fn full_pool_never_evicts_own_sender_and_respects_bytes() {
    let one = transfer(addr(1), 0, FEE, 1).encoded_len();
    let f = Fixture::with_config(PoolConfig {
        // Room for two txs (+ slack: a higher fee may encode one byte longer).
        max_bytes: 2 * one + 4,
        ..PoolConfig::default()
    });
    f.fund(addr(1), 0, RICH);
    f.fund(addr(2), 0, RICH);
    f.add(transfer(addr(1), 0, GWEI as u128, 1)).unwrap();
    f.add(transfer(addr(1), 1, GWEI as u128, 1)).unwrap();
    // Same sender, higher fee: cannot evict its own txs.
    assert_eq!(
        f.add(transfer(addr(1), 2, 9 * GWEI as u128, 1))
            .unwrap_err(),
        PoolError::PoolFull
    );
    // Another sender can push out the tail.
    f.add(transfer(addr(2), 0, 9 * GWEI as u128, 1)).unwrap();
    assert_eq!(f.pool.len(), 2);
    assert!(f.pool.total_bytes() <= 2 * one + 4);
}

// ---------------------------------------------------------------------------
// Block building order
// ---------------------------------------------------------------------------

#[test]
fn best_txs_orders_by_tip_then_arrival_with_nonce_order() {
    let f = Fixture::new();
    for i in 1..=3 {
        f.fund(addr(i), 0, RICH);
    }
    let g = GWEI as u128;
    // addr(1): low tip first nonce, high tip second nonce.
    let a0 = transfer(addr(1), 0, 10 * g, 1);
    let a1 = transfer(addr(1), 1, 10 * g, 9 * g);
    // addr(2) and addr(3) tie on tip 3 gwei; addr(2) arrives first.
    let b0 = transfer(addr(2), 0, 10 * g, 3 * g);
    let c0 = transfer(addr(3), 0, 10 * g, 3 * g);
    for t in [&a0, &a1, &b0, &c0] {
        f.add(t.clone()).unwrap();
    }
    let best = f.pool.best_txs(BASE, 30_000_000);
    assert_eq!(hashes(&best), hashes(&[b0, c0, a0, a1]));
}

#[test]
fn best_txs_effective_tip_depends_on_base_fee() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    f.fund(addr(2), 0, RICH);
    let g = GWEI as u128;
    // At base 1 gwei: a tip = min(5, 6-1) = 5, b tip = min(3, 10-1) = 3 -> a first.
    // At base 4 gwei: a tip = min(5, 6-4) = 2, b tip = 3 -> b first.
    let a = transfer(addr(1), 0, 6 * g, 5 * g);
    let b = transfer(addr(2), 0, 10 * g, 3 * g);
    f.add(a.clone()).unwrap();
    f.add(b.clone()).unwrap();
    assert_eq!(
        hashes(&f.pool.best_txs(GWEI, 30_000_000)),
        hashes(&[a.clone(), b.clone()])
    );
    assert_eq!(
        hashes(&f.pool.best_txs(4 * GWEI, 30_000_000)),
        hashes(&[b.clone(), a.clone()])
    );
    // Base fee above a's max fee: a excluded.
    assert_eq!(hashes(&f.pool.best_txs(7 * GWEI, 30_000_000)), hashes(&[b]));
}

#[test]
fn best_txs_stops_sender_below_base_fee() {
    let f = Fixture::new();
    f.fund(addr(1), 0, RICH);
    let g = GWEI as u128;
    let t0 = transfer(addr(1), 0, 10 * g, g);
    let t1 = transfer(addr(1), 1, g, g); // max fee 1 gwei
    let t2 = transfer(addr(1), 2, 10 * g, g);
    for t in [&t0, &t1, &t2] {
        f.add(t.clone()).unwrap();
    }
    assert_eq!(
        hashes(&f.pool.best_txs(2 * GWEI, 30_000_000)),
        hashes(&[t0])
    );
    // Does not remove anything.
    assert_eq!(f.pool.len(), 3);
}

#[test]
fn best_txs_respects_block_gas_limit() {
    let f = Fixture::new();
    for i in 1..=3 {
        f.fund(addr(i), 0, RICH);
    }
    let g = GWEI as u128;
    let big = tx1559(addr(1), 0, 10 * g, 5 * g, 50_000); // best tip, too big
    let big_next = tx1559(addr(1), 1, 10 * g, 5 * g, 21_000);
    let small = tx1559(addr(2), 0, 10 * g, 2 * g, 21_000);
    let small2 = tx1559(addr(3), 0, 10 * g, g, 21_000);
    for t in [&big, &big_next, &small, &small2] {
        f.add(t.clone()).unwrap();
    }
    // 45k gas: big does not fit -> skip addr(1) entirely, take both small ones.
    assert_eq!(
        hashes(&f.pool.best_txs(BASE, 45_000)),
        hashes(&[small.clone(), small2.clone()])
    );
    // 71k: big (50k) then its successor (21k) fill the block exactly.
    assert_eq!(
        hashes(&f.pool.best_txs(BASE, 71_000)),
        hashes(&[big, big_next])
    );
    assert!(f.pool.best_txs(BASE, 20_000).is_empty());
}

#[test]
fn best_txs_is_deterministic() {
    let build = || {
        let f = Fixture::new();
        for i in 1..=20u8 {
            f.fund(addr(i), 0, RICH);
            for n in 0..3u64 {
                let tip = u128::from((i as u64 * 7 + n * 13) % 5) * GWEI as u128;
                f.add(transfer(addr(i), n, 10 * GWEI as u128, tip)).unwrap();
            }
        }
        hashes(&f.pool.best_txs(BASE, 30_000_000))
    };
    let a = build();
    assert_eq!(a.len(), 60);
    assert_eq!(a, build());
}

#[test]
fn concurrent_adds_are_safe() {
    let f = Arc::new(Fixture::new());
    for i in 1..=8 {
        f.fund(addr(i), 0, RICH);
    }
    let handles: Vec<_> = (1..=8u8)
        .map(|i| {
            let f = f.clone();
            std::thread::spawn(move || {
                for n in 0..10 {
                    f.add(transfer(addr(i), n, FEE, 1)).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        f.pool.status(),
        PoolStatus {
            pending: 80,
            queued: 0
        }
    );
    let dyn_pool: &dyn TxPool = &f.pool;
    assert_eq!(dyn_pool.best_txs(BASE, 30_000_000).len(), 80);
}

#[test]
fn from_state_adapter() {
    let mut state = xdevs_state::InMemoryState::new();
    state.insert_account(addr(1), AccountInfo::new(3, U256::from(RICH)));
    let reader = FromState(state);
    assert_eq!(reader.account(addr(1)).unwrap().unwrap().nonce, 3);
    assert_eq!(reader.account(addr(2)).unwrap(), None);
}

// ---------------------------------------------------------------------------
// Property test: random operation sequences keep the pool's invariants.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    Add {
        sender: u8,
        nonce: u64,
        fee: u8,
        tip: u8,
    },
    Mine {
        sender: u8,
    },
    Remove {
        index: usize,
    },
}

fn op() -> impl proptest::strategy::Strategy<Value = Op> {
    use proptest::prelude::*;
    prop_oneof![
        6 => (1..5u8, 0..8u64, 1..6u8, 0..6u8)
            .prop_map(|(sender, nonce, fee, tip)| Op::Add { sender, nonce, fee, tip }),
        1 => (1..5u8).prop_map(|sender| Op::Mine { sender }),
        1 => (0..20usize).prop_map(|index| Op::Remove { index }),
    ]
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
    #[test]
    fn invariants_hold(ops in proptest::collection::vec(op(), 1..60)) {
        let f = Fixture::with_config(PoolConfig {
            max_txs: 10,
            max_per_sender: 4,
            ..PoolConfig::default()
        });
        for i in 1..5 {
            f.fund(addr(i), 0, RICH);
        }
        let mut ts = 0;
        for op in ops {
            match op {
                Op::Add { sender, nonce, fee, tip } => {
                    let fee = u128::from(fee) * GWEI as u128;
                    let tip = (u128::from(tip) * GWEI as u128).min(fee);
                    let _ = f.add(transfer(addr(sender), nonce, fee, tip));
                }
                Op::Mine { sender } => {
                    let mined: Vec<RecoveredTx> = f
                        .pool
                        .pending()
                        .into_iter()
                        .filter(|t| t.sender == addr(sender))
                        .take(1)
                        .collect();
                    if let Some(t) = mined.first() {
                        f.fund(addr(sender), t.nonce() + 1, RICH);
                    }
                    ts += 2;
                    f.commit(mined.into_iter().map(|t| t.tx).collect(), ts);
                }
                Op::Remove { index } => {
                    let all = f.pool.pending();
                    if let Some(t) = all.get(index) {
                        f.pool.remove(&[t.hash()]);
                    }
                }
            }

            let status = f.pool.status();
            let pending = f.pool.pending();
            let queued = f.pool.queued();
            proptest::prop_assert_eq!(status.pending, pending.len());
            proptest::prop_assert_eq!(status.queued, queued.len());
            proptest::prop_assert_eq!(f.pool.len(), pending.len() + queued.len());
            proptest::prop_assert!(f.pool.len() <= 10);
            let bytes: usize = pending.iter().chain(&queued).map(|t| t.encoded_len()).sum();
            proptest::prop_assert_eq!(f.pool.total_bytes(), bytes);
            for t in pending.iter().chain(&queued) {
                proptest::prop_assert!(f.pool.contains(&t.hash()));
            }

            for limit in [21_000u64, 100_000, 30_000_000] {
                let best = f.pool.best_txs(BASE, limit);
                let gas: u64 = best.iter().map(|t| t.gas_limit()).sum();
                proptest::prop_assert!(gas <= limit);
                // Per sender: contiguous nonces starting at the account nonce.
                let mut next: BTreeMap<Address, u64> = BTreeMap::new();
                for t in &best {
                    let expected = next
                        .entry(t.sender)
                        .or_insert_with(|| f.accounts.read()[&t.sender].nonce);
                    proptest::prop_assert_eq!(t.nonce(), *expected);
                    *expected += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// XIP-0001 (EOA lock), spec §5 e and §10 M1 / M2 / M6
// ---------------------------------------------------------------------------

/// EIP-1559 tx with explicit target, value, gas and max fee (unique hash per sender).
fn xtx(
    sender: Address,
    nonce: u64,
    to: Address,
    value: u64,
    gas: u64,
    max_fee: u128,
) -> RecoveredTx {
    let tx = TxEip1559 {
        chain_id: CHAIN,
        nonce,
        gas_limit: gas,
        max_fee_per_gas: max_fee,
        max_priority_fee_per_gas: 1,
        to: TxKind::Call(to),
        value: U256::from(value),
        access_list: AccessList::default(),
        input: Bytes::copy_from_slice(sender.as_slice()),
    };
    RecoveredTx::new_unchecked(TxEnvelope::from(tx.into_signed(sig())), sender)
}

/// V2 shape from `a`: value-0 self-call, 300k gas, 2 gwei.
fn self_call(a: Address, nonce: u64) -> RecoveredTx {
    xtx(a, nonce, a, 0, 300_000, FEE)
}

fn xip_reason(r: Result<TxHash, PoolError>) -> &'static str {
    match r {
        Err(PoolError::Xip0001(e)) => e.violation.reason(),
        other => panic!("expected an XIP-0001 rejection, got {other:?}"),
    }
}

#[test]
fn xip0001_admission_rules() {
    let f = Fixture::new();
    let a = addr(0xa1);
    f.delegate(a, 0, 1);
    let other = Address::repeat_byte(0xf1);
    // M1 / V4, V3, V7b, V8b.
    assert_eq!(
        xip_reason(f.add(xtx(a, 0, other, 0, 50_000, FEE))),
        "XIP0001_NOT_SELF_CALL"
    );
    assert_eq!(
        xip_reason(f.add(xtx(a, 0, a, 1, 50_000, FEE))),
        "XIP0001_VALUE_NOT_ZERO"
    );
    assert_eq!(
        xip_reason(f.add(xtx(a, 0, a, 0, 2_000_001, FEE))),
        "XIP0001_GAS_LIMIT"
    );
    assert_eq!(
        xip_reason(f.add(xtx(a, 0, a, 0, 50_000, FEE + 1))),
        "XIP0001_FEE_CAP"
    );
    // V2 shape is accepted; the error message names the sender.
    f.add(self_call(a, 0)).unwrap();
    let msg = f
        .add(xtx(a, 1, other, 0, 50_000, FEE))
        .unwrap_err()
        .to_string();
    assert!(
        msg.contains("is EOA-locked: only value-0 calls to itself"),
        "{msg}"
    );
    // A set-code tx from a locked sender must be a self-call too.
    assert_eq!(
        xip_reason(f.add(set_code(a, 1, other, 1, 100_000))),
        "XIP0001_NOT_SELF_CALL"
    );
    f.add(set_code(a, 1, a, 1, 100_000)).unwrap();

    // M2: sealed rejects everything.
    let s = addr(0xa2);
    f.delegate(s, 0, 2);
    assert_eq!(xip_reason(f.add(self_call(s, 0))), "XIP0001_SEALED");
    // Garbage word = locked.
    let g = addr(0xa3);
    f.delegate(g, 0, 0xdead);
    f.add(self_call(g, 0)).unwrap();
    assert_eq!(
        xip_reason(f.add(xtx(g, 1, other, 0, 50_000, FEE))),
        "XIP0001_NOT_SELF_CALL"
    );
    // Word 0 / plain EOA with a word: unrestricted.
    let u = addr(0xa4);
    f.delegate(u, 0, 0);
    f.add(xtx(u, 0, other, 5, 50_000, 100 * FEE)).unwrap();
    let plain = addr(0xa5);
    f.fund(plain, 0, RICH);
    f.set_word(plain, 2);
    f.add(xtx(plain, 0, other, 5, 50_000, FEE)).unwrap();
}

#[test]
fn xip0001_locked_sender_limits() {
    let f = Fixture::new();
    let a = addr(0xa1);
    f.delegate(a, 0, 1);
    for n in 0..4 {
        f.add(self_call(a, n)).unwrap();
    }
    assert_eq!(
        f.add(self_call(a, 4)).unwrap_err(),
        PoolError::SenderLimit(a)
    );
    // One tx per block for the locked sender, the lowest nonce.
    let b = addr(0xb1);
    f.fund(b, 0, RICH);
    f.add(transfer(b, 0, FEE, 1)).unwrap();
    f.add(transfer(b, 1, FEE, 1)).unwrap();
    let best = f.pool.best_txs(BASE, 30_000_000);
    let from_a: Vec<u64> = best
        .iter()
        .filter(|t| t.sender == a)
        .map(|t| t.nonce())
        .collect();
    assert_eq!(from_a, vec![0]);
    assert_eq!(best.iter().filter(|t| t.sender == b).count(), 2);
    assert_eq!(f.pool.pending().len(), 6, "the others stay pending");
}

#[test]
fn xip0001_recheck_on_new_head() {
    let f = Fixture::new();
    let other = Address::repeat_byte(0xf1);
    // M6: a pending self-call is evicted once the sender is sealed.
    let a = addr(0xa1);
    f.delegate(a, 0, 1);
    let h = f.add(self_call(a, 0)).unwrap();
    f.set_word(a, 2);
    f.commit(vec![], 10);
    assert!(!f.pool.contains(&h), "evicted after the seal");
    assert!(f.pool.best_txs(BASE, 30_000_000).is_empty());

    // Lock enabled while unlocked txs wait: transfers go, self-calls stay, max 4.
    let b = addr(0xb1);
    f.delegate(b, 0, 0);
    let xfer = f.add(xtx(b, 0, other, 1, 50_000, FEE)).unwrap();
    let mut selfs = Vec::new();
    for n in 1..7 {
        selfs.push(f.add(self_call(b, n)).unwrap());
    }
    // Unlocked: 7 txs allowed.
    assert_eq!(f.pool.len(), 7);
    f.set_word(b, 1);
    f.commit(vec![], 20);
    assert!(!f.pool.contains(&xfer));
    assert_eq!(f.pool.len(), 4, "trimmed to the 4 lowest nonces");
    for h in &selfs[..4] {
        assert!(f.pool.contains(h));
    }
    // Fee-cap failures stay queued: base fee halves, tx (2 gwei) is above 2 x 0.5 gwei.
    let c = addr(0xc1);
    f.delegate(c, 0, 1);
    let h = f.add(self_call(c, 0)).unwrap();
    f.pool.set_base_fee(BASE / 2);
    f.commit(vec![], 30);
    assert!(f.pool.contains(&h), "c3 failures are kept");
    // A lifted lock lifts the limits.
    f.set_word(c, 0);
    f.commit(vec![], 40);
    f.add(xtx(c, 1, other, 1, 50_000, FEE)).unwrap();
}

#[test]
fn xip0001_activation_height() {
    let mut params = PoolParams::from_chain_config(&xdevs_primitives::ChainConfig::dev());
    params.xip0001_block = Some(100);
    let f = Fixture::with_params(PoolConfig::default(), params);
    let a = addr(0xa1);
    f.delegate(a, 0, 1);
    let other = Address::repeat_byte(0xf1);
    // Head unknown (treated as before block 1) and head 98: next block < 100.
    assert!(!f.pool.xip0001_active());
    f.add(xtx(a, 0, other, 0, 50_000, FEE)).unwrap();
    f.pool.set_head_number(98);
    f.add(xtx(a, 1, other, 0, 50_000, FEE)).unwrap();
    // Head 99: the next block is 100, the rules apply.
    f.pool.set_head_number(99);
    assert!(f.pool.xip0001_active());
    assert_eq!(
        xip_reason(f.add(xtx(a, 2, other, 0, 50_000, FEE))),
        "XIP0001_NOT_SELF_CALL"
    );
    // F2: never active without the field.
    let mut params = PoolParams::from_chain_config(&xdevs_primitives::ChainConfig::dev());
    params.xip0001_block = None;
    let f = Fixture::with_params(PoolConfig::default(), params);
    f.delegate(a, 0, 2);
    f.pool.set_head_number(1_000_000);
    f.add(xtx(a, 0, other, 0, 50_000, FEE)).unwrap();
}

#[test]
fn xip0001_set_code_tx_with_locked_authority_is_admitted() {
    // The pool never inspects tuples: a locked authority's tuple is skipped at execution.
    let f = Fixture::new();
    let relayer = addr(0x77);
    f.fund(relayer, 0, RICH);
    f.delegate(addr(0xa1), 0, 2);
    f.add(set_code(relayer, 0, Address::repeat_byte(1), 1, 100_000))
        .unwrap();
}

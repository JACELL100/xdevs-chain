//! The concrete transaction pool.
//!
//! Layout: one [`SenderTxs`] per sender holding its txs keyed by nonce, plus a hash
//! index. A sender's txs whose nonces form a contiguous run starting at the account
//! nonce are **pending** (executable); the rest are **queued**. The split is computed
//! from the nonces, so promotion after a block or a gap-filling tx is automatic.
//!
//! Everything is in `BTreeMap`s and ties are broken by a monotonically increasing
//! arrival sequence number, so every query is deterministic.

use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    sync::Arc,
};

use parking_lot::RwLock;
use xdevs_primitives::{
    xip0001::{
        check_shape, check_tx, eoa_lock_status, TxShape, LOCK_SLOT, MAX_GAS,
        MAX_POOL_TXS_PER_LOCKED_SENDER,
    },
    AccountInfo, Address, Block, EoaLockStatus, ReceiptEnvelope, RecoveredTx, Transaction, TxHash,
    B256, U256,
};
use xdevs_state::{StateError, StateProvider};

use crate::{
    validation::{validate_stateless, PoolParams, TX_BASE_GAS},
    PoolConfig, PoolError, PoolStatus, TxOrigin, TxPool,
};

/// Read access to the **latest committed** account state (nonce + balance, and the
/// XIP-0001 lock slot of EIP-7702 accounts).
pub trait AccountReader: Send + Sync {
    /// Account at the latest committed block (`None` = does not exist).
    fn account(&self, address: Address) -> Result<Option<AccountInfo>, StateError>;

    /// Storage slot at the latest committed block. Only read for accounts delegated to
    /// the xDEVS Smart Account (XIP-0001 lock word). The default returns zero, i.e. such a
    /// reader cannot see EOA locks: node readers must implement it (see [`FromState`]).
    fn storage(&self, _address: Address, _slot: B256) -> Result<U256, StateError> {
        Ok(U256::ZERO)
    }
}

impl<F> AccountReader for F
where
    F: Fn(Address) -> Result<Option<AccountInfo>, StateError> + Send + Sync,
{
    fn account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        self(address)
    }
}

/// Adapter: any thread-safe [`StateProvider`] as an [`AccountReader`].
#[derive(Debug, Clone)]
pub struct FromState<S>(pub S);

impl<S: StateProvider + Send + Sync> AccountReader for FromState<S> {
    fn account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        self.0.basic_account(address)
    }

    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        self.0.storage(address, slot)
    }
}

/// A pooled transaction with its bookkeeping.
#[derive(Clone, Debug)]
struct PooledTx {
    tx: RecoveredTx,
    hash: TxHash,
    /// Arrival order (unique).
    seq: u64,
    /// EIP-2718 encoded size.
    size: usize,
    /// `gas_limit * max_fee + value`.
    cost: U256,
    /// Timestamp of the latest committed block when the tx arrived (`None` before the
    /// first block was seen; stamped at the next commit).
    added_at: Option<u64>,
}

impl PooledTx {
    fn max_fee(&self) -> u128 {
        self.tx.max_fee_per_gas()
    }
    fn tip_cap(&self) -> u128 {
        self.tx.priority_fee_or_price()
    }
}

/// All txs of one sender plus its cached account state.
#[derive(Debug, Default)]
struct SenderTxs {
    /// Account nonce at the latest state read.
    nonce: u64,
    /// Account balance at the latest state read.
    balance: U256,
    /// XIP-0001 lock status at the latest state read (`Unlocked` while inactive).
    lock: EoaLockStatus,
    txs: BTreeMap<u64, PooledTx>,
}

impl SenderTxs {
    /// First nonce after the contiguous run starting at the account nonce
    /// (`== nonce` if nothing is executable).
    fn pending_end(&self) -> u64 {
        let mut next = self.nonce;
        for n in self.txs.range(self.nonce..).map(|(n, _)| *n) {
            if n != next {
                break;
            }
            next += 1;
        }
        next
    }

    fn pending_count(&self) -> usize {
        (self.pending_end() - self.nonce) as usize
    }
}

#[derive(Debug, Default)]
struct Inner {
    senders: BTreeMap<Address, SenderTxs>,
    by_hash: BTreeMap<TxHash, (Address, u64)>,
    next_seq: u64,
    total_bytes: usize,
    base_fee: u64,
    /// Timestamp of the latest committed block seen by the pool.
    last_timestamp: Option<u64>,
    /// Number of the latest committed block (`None` = unknown, treated as before block 1).
    head: Option<u64>,
}

impl Inner {
    fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// Remove one tx by sender + nonce; drops empty senders.
    fn remove_at(&mut self, sender: Address, nonce: u64) -> Option<PooledTx> {
        let entry = self.senders.get_mut(&sender)?;
        let removed = entry.txs.remove(&nonce)?;
        if entry.txs.is_empty() {
            self.senders.remove(&sender);
        }
        self.by_hash.remove(&removed.hash);
        self.total_bytes -= removed.size;
        Some(removed)
    }

    fn remove_hash(&mut self, hash: &TxHash) -> Option<PooledTx> {
        let (sender, nonce) = *self.by_hash.get(hash)?;
        self.remove_at(sender, nonce)
    }

    /// Number of the block the pool fills next.
    fn next_block(&self) -> u64 {
        self.head.map_or(0, |h| h.saturating_add(1))
    }

    /// Apply a fresh account read: drop stale nonces and unaffordable txs, and (XIP-0001)
    /// everything a `SEALED` sender holds, a `LOCKED` sender's txs that are not value-0
    /// self-calls or exceed the gas cap, and its txs beyond the 4 lowest nonces. Txs that
    /// only fail the fee cap (c3) stay: the base fee may move.
    fn reset_sender(
        &mut self,
        sender: Address,
        account: AccountInfo,
        lock: EoaLockStatus,
    ) -> Vec<TxHash> {
        let Some(entry) = self.senders.get_mut(&sender) else {
            return Vec::new();
        };
        entry.nonce = account.nonce;
        entry.balance = account.balance;
        entry.lock = lock;
        let xip_invalid = |t: &PooledTx| {
            let shape = TxShape::from(&t.tx);
            check_shape(lock, &shape).is_err()
                || (lock == EoaLockStatus::Locked && shape.gas_limit > MAX_GAS)
        };
        let mut doomed: Vec<u64> = entry
            .txs
            .iter()
            .filter(|(n, t)| **n < account.nonce || t.cost > account.balance || xip_invalid(t))
            .map(|(n, _)| *n)
            .collect();
        if lock == EoaLockStatus::Locked {
            doomed.extend(
                entry
                    .txs
                    .keys()
                    .filter(|n| !doomed.contains(n))
                    .skip(MAX_POOL_TXS_PER_LOCKED_SENDER)
                    .copied()
                    .collect::<Vec<_>>(),
            );
        }
        doomed
            .into_iter()
            .filter_map(|n| self.remove_at(sender, n).map(|t| t.hash))
            .collect()
    }
}

/// Eviction candidate: a sender's highest-nonce tx (so eviction never opens a gap).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Victim {
    /// Cheapest first.
    max_fee: u128,
    /// Then newest first.
    newest: Reverse<u64>,
    sender: Address,
    nonce: u64,
}

/// Entry of the block-building priority queue: max effective tip, then earliest arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Candidate {
    tip: u128,
    seq: u64,
    sender: Address,
    nonce: u64,
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.tip
            .cmp(&other.tip)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Thread-safe transaction pool (plan §13). Share it as `Arc<Pool>` / `Arc<dyn TxPool>`.
pub struct Pool {
    params: PoolParams,
    config: PoolConfig,
    state: Arc<dyn AccountReader>,
    inner: RwLock<Inner>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("params", &self.params)
            .field("config", &self.config)
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl Pool {
    /// New empty pool. `base_fee` is the current (latest block's or next block's) base
    /// fee used for the `max_fee >= 0.9 × base_fee` entry check.
    pub fn new(
        params: PoolParams,
        config: PoolConfig,
        state: Arc<dyn AccountReader>,
        base_fee: u64,
    ) -> Self {
        Self {
            params,
            config,
            state,
            inner: RwLock::new(Inner {
                base_fee,
                ..Inner::default()
            }),
        }
    }

    /// Update the base fee used by the entry check (e.g. to `next_base_fee(head)`).
    pub fn set_base_fee(&self, base_fee: u64) {
        self.inner.write().base_fee = base_fee;
    }

    /// Set the number of the latest committed block (call once at start-up; afterwards
    /// [`TxPool::on_block_committed`] keeps it current). Decides whether XIP-0001 applies
    /// to the next block.
    pub fn set_head_number(&self, number: u64) {
        self.inner.write().head = Some(number);
    }

    /// `true` if XIP-0001 applies to the block the pool fills next.
    pub fn xip0001_active(&self) -> bool {
        let next = self.inner.read().next_block();
        self.params.xip0001_active(next)
    }

    /// Base fee used by the entry check.
    pub fn base_fee(&self) -> u64 {
        self.inner.read().base_fee
    }

    /// Validation parameters.
    pub fn params(&self) -> &PoolParams {
        &self.params
    }

    /// Limits.
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    /// Number of pooled txs (pending + queued).
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// `true` if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total encoded bytes held.
    pub fn total_bytes(&self) -> usize {
        self.inner.read().total_bytes
    }

    /// All queued (future-nonce) txs, nonce-ordered per sender.
    pub fn queued(&self) -> Vec<RecoveredTx> {
        let inner = self.inner.read();
        let mut out = Vec::new();
        for s in inner.senders.values() {
            let end = s.pending_end();
            out.extend(s.txs.range(end..).map(|(_, t)| t.tx.clone()));
        }
        out
    }

    fn read_account(&self, address: Address) -> Result<AccountInfo, PoolError> {
        self.state
            .account(address)
            .map(Option::unwrap_or_default)
            .map_err(|e| PoolError::State(e.to_string()))
    }

    /// XIP-0001 status of `address` (whose account is `account`) at the head state, or
    /// `Unlocked` if XIP-0001 does not apply to the next block.
    fn read_lock(
        &self,
        address: Address,
        account: &AccountInfo,
    ) -> Result<EoaLockStatus, PoolError> {
        if !self.xip0001_active() {
            return Ok(EoaLockStatus::Unlocked);
        }
        eoa_lock_status(Some(account.code_hash), || {
            self.state.storage(address, LOCK_SLOT)
        })
        .map_err(|e| PoolError::State(e.to_string()))
    }

    /// Plan which txs to evict so that `extra_bytes` more bytes and (if `new_slot`) one
    /// more tx fit. Only other senders' tail txs strictly cheaper than `max_fee` qualify.
    fn plan_eviction(
        &self,
        inner: &Inner,
        sender: Address,
        new_slot: bool,
        extra_bytes: usize,
        max_fee: u128,
    ) -> Result<Vec<(Address, u64)>, PoolError> {
        let mut count = inner.len() + usize::from(new_slot);
        let mut bytes = inner.total_bytes + extra_bytes;
        if count <= self.config.max_txs && bytes <= self.config.max_bytes {
            return Ok(Vec::new());
        }
        let victim_of = |addr: Address, s: &SenderTxs, below: u64| {
            s.txs.range(..below).next_back().map(|(n, t)| Victim {
                max_fee: t.max_fee(),
                newest: Reverse(t.seq),
                sender: addr,
                nonce: *n,
            })
        };
        let mut heap: BTreeSet<Victim> = inner
            .senders
            .iter()
            .filter(|(a, _)| **a != sender)
            .filter_map(|(a, s)| victim_of(*a, s, u64::MAX))
            .collect();
        let mut plan = Vec::new();
        while count > self.config.max_txs || bytes > self.config.max_bytes {
            let Some(v) = heap.pop_first() else {
                return Err(PoolError::PoolFull);
            };
            if v.max_fee >= max_fee {
                return Err(PoolError::PoolFull);
            }
            let s = &inner.senders[&v.sender];
            count -= 1;
            bytes -= s.txs[&v.nonce].size;
            plan.push((v.sender, v.nonce));
            if let Some(next) = victim_of(v.sender, s, v.nonce) {
                heap.insert(next);
            }
        }
        Ok(plan)
    }

    /// State-only checks of a new tx (no pool lock): nonce not below the account nonce
    /// nor too far ahead, and the balance covers at least this tx alone.
    fn precheck(&self, nonce: u64, cost: U256, account: &AccountInfo) -> Result<(), PoolError> {
        if nonce < account.nonce {
            return Err(PoolError::NonceTooLow {
                state: account.nonce,
                tx: nonce,
            });
        }
        if nonce - account.nonce > self.config.max_nonce_gap {
            return Err(PoolError::NonceGapTooLarge {
                state: account.nonce,
                tx: nonce,
            });
        }
        if cost > account.balance {
            return Err(PoolError::InsufficientFunds {
                balance: account.balance,
                cost,
            });
        }
        Ok(())
    }

    /// Drop queued txs older than the TTL (measured in block timestamps).
    fn expire_queued(&self, inner: &mut Inner, now: u64) {
        let ttl = self.config.queued_ttl_secs;
        let mut expired = Vec::new();
        for (addr, s) in &inner.senders {
            let end = s.pending_end();
            for (n, t) in s.txs.range(end..) {
                if t.added_at.is_some_and(|at| at.saturating_add(ttl) < now) {
                    expired.push((*addr, *n));
                }
            }
        }
        for (addr, n) in expired {
            if let Some(t) = inner.remove_at(addr, n) {
                tracing::debug!(hash = %t.hash, "dropping expired queued tx");
            }
        }
    }
}

impl TxPool for Pool {
    fn add(&self, tx: RecoveredTx, _origin: TxOrigin) -> Result<TxHash, PoolError> {
        // Phase 1 treats local and external txs identically (no local exemptions).
        let hash = tx.hash();
        if self.inner.read().by_hash.contains_key(&hash) {
            return Err(PoolError::AlreadyKnown);
        }
        validate_stateless(&tx, &self.params, self.base_fee())?;

        let sender = tx.sender;
        let nonce = tx.nonce();
        let size = tx.encoded_len();
        let cost = U256::from(tx.gas_limit())
            .saturating_mul(U256::from(tx.max_fee_per_gas()))
            .saturating_add(tx.value());
        // Read state before taking the write lock (the reader may be slow).
        let account = self.read_account(sender)?;
        let lock = self.read_lock(sender, &account)?;
        // XIP-0001 (§5 e): as if the tx were the first one of the next block.
        check_tx(lock, &TxShape::from(&tx), self.base_fee(), false)?;
        // Cheap rejections before the write lock (security review: forged gossip txs
        // from random unfunded senders must not serialise on the pool lock). Each is
        // re-checked below against the pooled txs of the sender.
        self.precheck(nonce, cost, &account)?;

        let mut inner = self.inner.write();
        if inner.by_hash.contains_key(&hash) {
            return Err(PoolError::AlreadyKnown);
        }
        // Refresh the sender: the chain may have moved since its last read.
        if inner.senders.contains_key(&sender) {
            inner.reset_sender(sender, account, lock);
        }

        if nonce < account.nonce {
            return Err(PoolError::NonceTooLow {
                state: account.nonce,
                tx: nonce,
            });
        }
        if nonce - account.nonce > self.config.max_nonce_gap {
            return Err(PoolError::NonceGapTooLarge {
                state: account.nonce,
                tx: nonce,
            });
        }

        let existing = inner.senders.get(&sender);
        let replaced = existing.and_then(|s| s.txs.get(&nonce));
        if let Some(old) = replaced {
            let bump = 100 + u128::from(self.config.price_bump_percent);
            let new_fee = tx.max_fee_per_gas();
            let new_tip = tx.priority_fee_or_price();
            let fee_ok = new_fee > old.max_fee()
                && new_fee.saturating_mul(100) >= old.max_fee().saturating_mul(bump);
            let tip_ok = new_tip.saturating_mul(100) >= old.tip_cap().saturating_mul(bump);
            if !(fee_ok && tip_ok) {
                return Err(PoolError::ReplacementUnderpriced);
            }
        }

        // Balance must cover this tx plus every other pooled tx of the sender.
        let others: U256 = existing
            .map(|s| {
                s.txs
                    .iter()
                    .filter(|(n, _)| **n != nonce)
                    .fold(U256::ZERO, |acc, (_, t)| acc.saturating_add(t.cost))
            })
            .unwrap_or_default();
        let total_cost = others.saturating_add(cost);
        if total_cost > account.balance {
            return Err(PoolError::InsufficientFunds {
                balance: account.balance,
                cost: total_cost,
            });
        }

        let old_size = replaced.map(|t| t.size);
        let max_per_sender = if lock == EoaLockStatus::Locked {
            self.config
                .max_per_sender
                .min(MAX_POOL_TXS_PER_LOCKED_SENDER)
        } else {
            self.config.max_per_sender
        };
        if old_size.is_none() && existing.is_some_and(|s| s.txs.len() >= max_per_sender) {
            return Err(PoolError::SenderLimit(sender));
        }

        let extra_bytes = size.saturating_sub(old_size.unwrap_or(0));
        let evict = self.plan_eviction(
            &inner,
            sender,
            old_size.is_none(),
            extra_bytes,
            tx.max_fee_per_gas(),
        )?;
        for (addr, n) in evict {
            if let Some(t) = inner.remove_at(addr, n) {
                tracing::debug!(hash = %t.hash, "evicted underpriced tx (pool full)");
            }
        }
        if old_size.is_some() {
            if let Some(old) = inner.remove_at(sender, nonce) {
                tracing::debug!(old = %old.hash, new = %hash, "replaced tx");
            }
        }

        let seq = inner.next_seq;
        inner.next_seq += 1;
        let added_at = inner.last_timestamp;
        let entry = inner.senders.entry(sender).or_default();
        entry.nonce = account.nonce;
        entry.balance = account.balance;
        entry.lock = lock;
        entry.txs.insert(
            nonce,
            PooledTx {
                tx,
                hash,
                seq,
                size,
                cost,
                added_at,
            },
        );
        inner.by_hash.insert(hash, (sender, nonce));
        inner.total_bytes += size;
        Ok(hash)
    }

    fn best_txs(&self, base_fee: u64, gas_limit: u64) -> Vec<RecoveredTx> {
        let inner = self.inner.read();
        let fee_ok = |t: &PooledTx| t.max_fee() >= u128::from(base_fee);
        let candidate = |addr: Address, t: &PooledTx| {
            t.tx.effective_tip(base_fee).map(|tip| Candidate {
                tip,
                seq: t.seq,
                sender: addr,
                nonce: t.tx.nonce(),
            })
        };

        let mut heap = BinaryHeap::new();
        for (addr, s) in &inner.senders {
            if s.lock == EoaLockStatus::Sealed {
                continue;
            }
            if let Some(t) = s.txs.get(&s.nonce).filter(|t| fee_ok(t)) {
                heap.extend(candidate(*addr, t));
            }
        }

        let mut out = Vec::new();
        let mut remaining = gas_limit;
        while let Some(c) = heap.pop() {
            if remaining < TX_BASE_GAS {
                break;
            }
            let s = &inner.senders[&c.sender];
            let t = &s.txs[&c.nonce];
            if t.tx.gas_limit() > remaining {
                // Later nonces depend on this one: skip the whole sender.
                continue;
            }
            remaining -= t.tx.gas_limit();
            out.push(t.tx.clone());
            if s.lock == EoaLockStatus::Locked {
                // XIP-0001 c1: at most one tx (the lowest nonce) of a LOCKED sender.
                continue;
            }
            if let Some(next) = s.txs.get(&(c.nonce + 1)).filter(|t| fee_ok(t)) {
                heap.extend(candidate(c.sender, next));
            }
        }
        out
    }

    fn on_block_committed(&self, block: &Block, _receipts: &[ReceiptEnvelope]) {
        // Snapshot the sender set, read state without holding the lock, then apply.
        let senders: Vec<Address> = {
            let mut inner = self.inner.write();
            for hash in block.tx_hashes() {
                inner.remove_hash(&hash);
            }
            if let Some(fee) = block.header.base_fee_per_gas {
                inner.base_fee = fee;
            }
            let ts = block.header.timestamp;
            inner.last_timestamp = Some(ts);
            inner.head = Some(block.header.number);
            for s in inner.senders.values_mut() {
                for t in s.txs.values_mut() {
                    t.added_at.get_or_insert(ts);
                }
            }
            inner.senders.keys().copied().collect()
        };

        let accounts: Vec<(Address, AccountInfo, EoaLockStatus)> = senders
            .into_iter()
            .filter_map(|addr| {
                let read = self
                    .read_account(addr)
                    .and_then(|acc| Ok((acc, self.read_lock(addr, &acc)?)));
                match read {
                    Ok((acc, lock)) => Some((addr, acc, lock)),
                    Err(e) => {
                        tracing::warn!(%addr, error = %e, "mempool: account refresh failed");
                        None
                    }
                }
            })
            .collect();

        let mut inner = self.inner.write();
        for (addr, acc, lock) in accounts {
            for hash in inner.reset_sender(addr, acc, lock) {
                tracing::trace!(%hash, "dropped stale / unaffordable / XIP-0001-invalid tx");
            }
        }
        self.expire_queued(&mut inner, block.header.timestamp);
    }

    fn remove(&self, hashes: &[TxHash]) {
        let mut inner = self.inner.write();
        for hash in hashes {
            inner.remove_hash(hash);
        }
    }

    fn get(&self, hash: &TxHash) -> Option<RecoveredTx> {
        let inner = self.inner.read();
        let (sender, nonce) = inner.by_hash.get(hash)?;
        inner
            .senders
            .get(sender)?
            .txs
            .get(nonce)
            .map(|t| t.tx.clone())
    }

    fn contains(&self, hash: &TxHash) -> bool {
        self.inner.read().by_hash.contains_key(hash)
    }

    fn pending_nonce(&self, sender: &Address) -> Option<u64> {
        let inner = self.inner.read();
        let s = inner.senders.get(sender)?;
        let end = s.pending_end();
        (end > s.nonce).then_some(end)
    }

    fn pending(&self) -> Vec<RecoveredTx> {
        let inner = self.inner.read();
        let mut out = Vec::new();
        for s in inner.senders.values() {
            let end = s.pending_end();
            out.extend(s.txs.range(s.nonce..end).map(|(_, t)| t.tx.clone()));
        }
        out
    }

    fn status(&self) -> PoolStatus {
        let inner = self.inner.read();
        let pending: usize = inner.senders.values().map(SenderTxs::pending_count).sum();
        PoolStatus {
            pending,
            queued: inner.len() - pending,
        }
    }
}

#[cfg(test)]
mod tests;

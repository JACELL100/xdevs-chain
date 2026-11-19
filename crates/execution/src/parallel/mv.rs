//! Multi-version memory of Block-STM: for every state location, the value written by
//! each transaction index (or an `ESTIMATE` marker while that transaction is being
//! re-executed), plus each transaction's last read set for validation.
//!
//! Locations mirror how a [`StateDiff`] changes state:
//!
//! * [`Loc::Account`] — the whole account record. An entry may carry an absolute write
//!   and/or a **balance delta**: the beneficiary tip and treasury share of a transaction
//!   are deltas, so transactions that only *pay fees* never conflict with each other;
//!   only a transaction that actually reads one of those accounts depends on the deltas
//!   of all earlier transactions.
//! * [`Loc::Slot`] — one storage slot.
//! * [`Loc::Wipe`] — "all storage of this account cleared" (destroyed / re-created); a
//!   slot read returns zero if the newest wipe below the reader is newer than the newest
//!   write of that slot.
//!
//! Bytecode is content-addressed and never changes, so it is kept in a plain map.
//!
//! The maps are only used for lookups, never iterated, so `HashMap` does not leak any
//! nondeterminism into results (plan §10.6).

use crate::executor::FeeCredits;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Mutex, MutexGuard},
};
use xdevs_primitives::{AccountInfo, Address, Bytes, B256, U256};
use xdevs_state::{StateDiff, StateError, StateProvider};

const SHARDS: usize = 64;

/// Lock ignoring poisoning (a panicking worker aborts the whole parallel run anyway).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A state location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Loc {
    /// Account record (nonce, balance, code hash; `None` = does not exist).
    Account(Address),
    /// One storage slot.
    Slot(Address, B256),
    /// Storage of the account cleared.
    Wipe(Address),
}

impl Loc {
    fn shard(&self) -> usize {
        let (a, extra) = match self {
            Loc::Account(a) | Loc::Wipe(a) => (a, 0u8),
            Loc::Slot(a, s) => (a, s.as_slice()[31] ^ s.as_slice()[0]),
        };
        let b = a.as_slice();
        usize::from(b[19] ^ b[0] ^ extra) % SHARDS
    }
}

/// A value observed by a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Val {
    Account(Option<AccountInfo>),
    Slot(U256),
}

/// One transaction's entry at a location.
#[derive(Clone, Debug)]
pub(crate) enum Entry {
    /// The writing transaction is being re-executed: readers must wait for it.
    Estimate,
    /// Account: `write` (if any) replaces the record, then `delta` is credited.
    Account {
        write: Option<Option<AccountInfo>>,
        delta: U256,
    },
    Slot(U256),
    Wipe,
}

/// Result of resolving a location for a reader.
pub(crate) enum Resolved<T> {
    Value(T),
    /// Hit an `ESTIMATE` of this transaction.
    Blocked(usize),
}

/// `StateOverlay::credit` semantics: a zero amount changes nothing, otherwise a missing
/// account is created.
fn credit(account: Option<AccountInfo>, amount: U256) -> Option<AccountInfo> {
    if amount.is_zero() {
        return account;
    }
    let mut info = account.unwrap_or_default();
    info.balance = info.balance.saturating_add(amount);
    Some(info)
}

/// The state before the block's transactions: the parent state plus the changes of the
/// pre-transaction system calls (`pre`), read through a cache.
pub(crate) struct Base<'a> {
    provider: &'a (dyn StateProvider + Sync),
    pre: StateDiff,
    cache: Vec<Mutex<HashMap<Loc, Val>>>,
}

impl<'a> Base<'a> {
    pub(crate) fn new(provider: &'a (dyn StateProvider + Sync), pre: StateDiff) -> Self {
        Self {
            provider,
            pre,
            cache: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
        }
    }

    fn cached(&self, loc: Loc) -> Option<Val> {
        lock(&self.cache[loc.shard()]).get(&loc).copied()
    }

    fn remember(&self, loc: Loc, val: Val) {
        lock(&self.cache[loc.shard()]).insert(loc, val);
    }

    /// Same lookup order as `StateOverlay::account`.
    pub(crate) fn account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        if let Some(acc) = self.pre.accounts.get(&address) {
            return Ok(*acc);
        }
        let loc = Loc::Account(address);
        if let Some(Val::Account(v)) = self.cached(loc) {
            return Ok(v);
        }
        let v = self.provider.basic_account(address)?;
        self.remember(loc, Val::Account(v));
        Ok(v)
    }

    /// Same lookup order as `StateOverlay::slot`.
    pub(crate) fn slot(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        if let Some(sd) = self.pre.storage.get(&address) {
            if let Some(v) = sd.slots.get(&slot) {
                return Ok(*v);
            }
            if sd.wiped {
                return Ok(U256::ZERO);
            }
        }
        if matches!(self.pre.accounts.get(&address), Some(None)) {
            return Ok(U256::ZERO);
        }
        let loc = Loc::Slot(address, slot);
        if let Some(Val::Slot(v)) = self.cached(loc) {
            return Ok(v);
        }
        let v = self.provider.storage(address, slot)?;
        self.remember(loc, Val::Slot(v));
        Ok(v)
    }

    fn code(&self, code_hash: B256) -> Result<Option<Bytes>, StateError> {
        if let Some(code) = self.pre.codes.get(&code_hash) {
            return Ok(Some(code.clone()));
        }
        self.provider.code_by_hash(code_hash)
    }
}

type Versions = HashMap<Loc, BTreeMap<usize, Entry>>;

/// The multi-version data structure shared by all workers.
pub(crate) struct MvMemory {
    data: Vec<Mutex<Versions>>,
    /// Locations written by the last incarnation of each transaction.
    written: Vec<Mutex<Vec<Loc>>>,
    /// Read set of the last incarnation of each transaction.
    reads: Vec<Mutex<Vec<(Loc, Val)>>>,
    /// Bytecode deployed by any incarnation (content-addressed).
    codes: Mutex<HashMap<B256, Bytes>>,
}

impl MvMemory {
    pub(crate) fn new(txs: usize) -> Self {
        Self {
            data: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            written: (0..txs).map(|_| Mutex::new(Vec::new())).collect(),
            reads: (0..txs).map(|_| Mutex::new(Vec::new())).collect(),
            codes: Mutex::new(HashMap::new()),
        }
    }

    fn shard(&self, loc: &Loc) -> MutexGuard<'_, Versions> {
        lock(&self.data[loc.shard()])
    }

    /// Newest entry of `loc` below transaction `idx`.
    fn below(&self, loc: &Loc, idx: usize) -> Option<(usize, Entry)> {
        self.shard(loc)
            .get(loc)
            .and_then(|v| v.range(..idx).next_back().map(|(j, e)| (*j, e.clone())))
    }

    /// The account as transaction `idx` must see it.
    pub(crate) fn account(
        &self,
        address: Address,
        idx: usize,
        base: &Base<'_>,
    ) -> Result<Resolved<Option<AccountInfo>>, StateError> {
        let loc = Loc::Account(address);
        let mut delta = U256::ZERO;
        {
            let shard = self.shard(&loc);
            if let Some(versions) = shard.get(&loc) {
                for (j, entry) in versions.range(..idx).rev() {
                    match entry {
                        Entry::Estimate => return Ok(Resolved::Blocked(*j)),
                        Entry::Account {
                            write: Some(v),
                            delta: d,
                        } => return Ok(Resolved::Value(credit(*v, d.saturating_add(delta)))),
                        Entry::Account {
                            write: None,
                            delta: d,
                        } => {
                            delta = delta.saturating_add(*d);
                        }
                        Entry::Slot(_) | Entry::Wipe => {}
                    }
                }
            }
        }
        Ok(Resolved::Value(credit(base.account(address)?, delta)))
    }

    /// The storage slot as transaction `idx` must see it.
    pub(crate) fn slot(
        &self,
        address: Address,
        slot: B256,
        idx: usize,
        base: &Base<'_>,
    ) -> Result<Resolved<U256>, StateError> {
        let written = self.below(&Loc::Slot(address, slot), idx);
        let wiped = self.below(&Loc::Wipe(address), idx);
        // Within one transaction a wipe comes before its slot writes.
        let newest = match (written, wiped) {
            (Some(s), Some(w)) => Some(if w.0 > s.0 { w } else { s }),
            (s, w) => s.or(w),
        };
        Ok(match newest {
            None => Resolved::Value(base.slot(address, slot)?),
            Some((j, Entry::Estimate)) => Resolved::Blocked(j),
            Some((_, Entry::Slot(v))) => Resolved::Value(v),
            Some((_, Entry::Wipe | Entry::Account { .. })) => Resolved::Value(U256::ZERO),
        })
    }

    /// Install the write set and read set of a finished incarnation of `idx`. Returns
    /// `true` if it wrote a location the previous incarnation did not write.
    pub(crate) fn record(
        &self,
        idx: usize,
        writes: Vec<(Loc, Entry)>,
        reads: Vec<(Loc, Val)>,
        codes: &BTreeMap<B256, Bytes>,
    ) -> bool {
        if !codes.is_empty() {
            let mut known = lock(&self.codes);
            for (hash, code) in codes {
                known.entry(*hash).or_insert_with(|| code.clone());
            }
        }
        let locs: Vec<Loc> = writes.iter().map(|(l, _)| *l).collect();
        for (loc, entry) in writes {
            self.shard(&loc).entry(loc).or_default().insert(idx, entry);
        }
        let mut prev = lock(&self.written[idx]);
        let new: HashSet<Loc> = locs.iter().copied().collect();
        let old: HashSet<Loc> = prev.iter().copied().collect();
        for loc in prev.iter().filter(|l| !new.contains(l)) {
            if let Some(versions) = self.shard(loc).get_mut(loc) {
                versions.remove(&idx);
            }
        }
        let wrote_new = locs.iter().any(|l| !old.contains(l));
        *prev = locs;
        drop(prev);
        *lock(&self.reads[idx]) = reads;
        wrote_new
    }

    /// Mark every write of `idx` as an estimate (its incarnation was aborted).
    pub(crate) fn convert_writes_to_estimates(&self, idx: usize) {
        let written = lock(&self.written[idx]).clone();
        for loc in written {
            self.shard(&loc)
                .entry(loc)
                .or_default()
                .insert(idx, Entry::Estimate);
        }
    }

    /// `true` if every value read by the last incarnation of `idx` is still what `idx`
    /// would read now (value-based validation).
    pub(crate) fn validate(&self, idx: usize, base: &Base<'_>) -> bool {
        let reads = lock(&self.reads[idx]).clone();
        reads.iter().all(|(loc, seen)| {
            let now = match *loc {
                Loc::Account(a) => self.account(a, idx, base).map(|r| match r {
                    Resolved::Value(v) => Some(Val::Account(v)),
                    Resolved::Blocked(_) => None,
                }),
                Loc::Slot(a, s) => self.slot(a, s, idx, base).map(|r| match r {
                    Resolved::Value(v) => Some(Val::Slot(v)),
                    Resolved::Blocked(_) => None,
                }),
                Loc::Wipe(_) => return true,
            };
            matches!(now, Ok(Some(v)) if v == *seen)
        })
    }
}

/// The write set of one transaction: its state diff plus its deferred fee credits.
pub(crate) fn write_set(
    diff: &StateDiff,
    credits: &FeeCredits,
    beneficiary: Address,
    treasury: Address,
) -> Vec<(Loc, Entry)> {
    let mut accounts: BTreeMap<Address, (Option<Option<AccountInfo>>, U256)> = BTreeMap::new();
    for (address, info) in &diff.accounts {
        accounts.entry(*address).or_default().0 = Some(*info);
    }
    for (address, amount) in [(beneficiary, credits.tip), (treasury, credits.treasury)] {
        if !amount.is_zero() {
            let e = accounts.entry(address).or_default();
            e.1 = e.1.saturating_add(amount);
        }
    }
    let mut out: Vec<(Loc, Entry)> = accounts
        .into_iter()
        .map(|(a, (write, delta))| (Loc::Account(a), Entry::Account { write, delta }))
        .collect();
    for (address, sd) in &diff.storage {
        if sd.wiped {
            out.push((Loc::Wipe(*address), Entry::Wipe));
        }
        for (slot, value) in &sd.slots {
            out.push((Loc::Slot(*address, *slot), Entry::Slot(*value)));
        }
    }
    out
}

const BLOCKED: &str = "Block-STM: read of an estimated location";

/// The state seen by one incarnation of transaction `idx`: reads resolve through the
/// multi-version memory and are recorded for validation. Wrapped in a `StateOverlay`,
/// so the transaction runs through exactly the same code as sequential execution.
pub(crate) struct View<'m, 'a> {
    mv: &'m MvMemory,
    base: &'m Base<'a>,
    idx: usize,
    reads: RefCell<Vec<(Loc, Val)>>,
    blocked: Cell<Option<usize>>,
}

impl<'m, 'a> View<'m, 'a> {
    pub(crate) fn new(mv: &'m MvMemory, base: &'m Base<'a>, idx: usize) -> Self {
        Self {
            mv,
            base,
            idx,
            reads: RefCell::new(Vec::new()),
            blocked: Cell::new(None),
        }
    }

    /// The transaction this incarnation must wait for, if it read an estimate.
    pub(crate) fn blocked(&self) -> Option<usize> {
        self.blocked.get()
    }

    pub(crate) fn into_reads(self) -> Vec<(Loc, Val)> {
        self.reads.into_inner()
    }

    fn block(&self, on: usize) -> StateError {
        self.blocked.set(Some(on));
        StateError::Unavailable(BLOCKED.into())
    }
}

impl StateProvider for View<'_, '_> {
    fn basic_account(&self, address: Address) -> Result<Option<AccountInfo>, StateError> {
        if let Some(on) = self.blocked.get() {
            return Err(self.block(on));
        }
        match self.mv.account(address, self.idx, self.base)? {
            Resolved::Value(v) => {
                self.reads
                    .borrow_mut()
                    .push((Loc::Account(address), Val::Account(v)));
                Ok(v)
            }
            Resolved::Blocked(on) => Err(self.block(on)),
        }
    }

    fn storage(&self, address: Address, slot: B256) -> Result<U256, StateError> {
        if let Some(on) = self.blocked.get() {
            return Err(self.block(on));
        }
        match self.mv.slot(address, slot, self.idx, self.base)? {
            Resolved::Value(v) => {
                self.reads
                    .borrow_mut()
                    .push((Loc::Slot(address, slot), Val::Slot(v)));
                Ok(v)
            }
            Resolved::Blocked(on) => Err(self.block(on)),
        }
    }

    fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytes>, StateError> {
        if let Some(code) = lock(&self.mv.codes).get(&code_hash) {
            return Ok(Some(code.clone()));
        }
        self.base.code(code_hash)
    }

    fn block_hash(&self, number: u64) -> Result<Option<B256>, StateError> {
        self.base.provider.block_hash(number)
    }

    fn state_root_with(&self, _diff: &StateDiff) -> Result<B256, StateError> {
        Err(StateError::Unavailable(
            "state root of a Block-STM transaction view".into(),
        ))
    }
}

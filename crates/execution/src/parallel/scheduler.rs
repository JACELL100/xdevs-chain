//! The collaborative scheduler of Block-STM (Gelashvili et al., "Block-STM: Scaling
//! Blockchain Execution by Turning Ordering Curse to a Performance Blessing", 2022,
//! Algorithms 4–5): two shared indices hand out execution and validation tasks in
//! transaction order, aborted transactions lower them again, and a transaction that
//! read an `ESTIMATE` waits on the transaction that wrote it.

use super::mv::lock;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    Mutex,
};

/// `(transaction index, incarnation)`.
pub(crate) type Version = (usize, usize);

/// Work handed to a worker.
pub(crate) enum Task {
    Execute(Version),
    Validate(Version),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    ReadyToExecute,
    Executing,
    Executed,
    Aborting,
}

pub(crate) struct Scheduler {
    txs: usize,
    execution_idx: AtomicUsize,
    validation_idx: AtomicUsize,
    decrease_cnt: AtomicUsize,
    num_active_tasks: AtomicUsize,
    done_marker: AtomicBool,
    /// Transactions waiting for each transaction to finish executing.
    dependencies: Vec<Mutex<Vec<usize>>>,
    /// `(incarnation, status)` per transaction.
    status: Vec<Mutex<(usize, Status)>>,
}

impl Scheduler {
    pub(crate) fn new(txs: usize) -> Self {
        Self {
            txs,
            execution_idx: AtomicUsize::new(0),
            validation_idx: AtomicUsize::new(0),
            decrease_cnt: AtomicUsize::new(0),
            num_active_tasks: AtomicUsize::new(0),
            done_marker: AtomicBool::new(false),
            dependencies: (0..txs).map(|_| Mutex::new(Vec::new())).collect(),
            status: (0..txs)
                .map(|_| Mutex::new((0, Status::ReadyToExecute)))
                .collect(),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.done_marker.load(SeqCst)
    }

    fn decrease_execution_idx(&self, target: usize) {
        self.execution_idx.fetch_min(target, SeqCst);
        self.decrease_cnt.fetch_add(1, SeqCst);
    }

    fn decrease_validation_idx(&self, target: usize) {
        self.validation_idx.fetch_min(target, SeqCst);
        self.decrease_cnt.fetch_add(1, SeqCst);
    }

    fn check_done(&self) {
        let observed = self.decrease_cnt.load(SeqCst);
        if self
            .execution_idx
            .load(SeqCst)
            .min(self.validation_idx.load(SeqCst))
            >= self.txs
            && self.num_active_tasks.load(SeqCst) == 0
            && observed == self.decrease_cnt.load(SeqCst)
        {
            self.done_marker.store(true, SeqCst);
        }
    }

    /// `READY_TO_EXECUTE(i)` → `EXECUTING(i)`. Does not touch the task counter.
    fn try_incarnate(&self, idx: usize) -> Option<Version> {
        if idx < self.txs {
            let mut s = lock(&self.status[idx]);
            if s.1 == Status::ReadyToExecute {
                s.1 = Status::Executing;
                return Some((idx, s.0));
            }
        }
        None
    }

    fn next_version_to_execute(&self) -> Option<Version> {
        if self.execution_idx.load(SeqCst) >= self.txs {
            self.check_done();
            return None;
        }
        self.num_active_tasks.fetch_add(1, SeqCst);
        let idx = self.execution_idx.fetch_add(1, SeqCst);
        let version = self.try_incarnate(idx);
        if version.is_none() {
            self.num_active_tasks.fetch_sub(1, SeqCst);
        }
        version
    }

    fn next_version_to_validate(&self) -> Option<Version> {
        if self.validation_idx.load(SeqCst) >= self.txs {
            self.check_done();
            return None;
        }
        self.num_active_tasks.fetch_add(1, SeqCst);
        let idx = self.validation_idx.fetch_add(1, SeqCst);
        if idx < self.txs {
            let s = lock(&self.status[idx]);
            if s.1 == Status::Executed {
                return Some((idx, s.0));
            }
        }
        self.num_active_tasks.fetch_sub(1, SeqCst);
        None
    }

    /// Validation first whenever it lags behind execution.
    pub(crate) fn next_task(&self) -> Option<Task> {
        if self.validation_idx.load(SeqCst) < self.execution_idx.load(SeqCst) {
            self.next_version_to_validate().map(Task::Validate)
        } else {
            self.next_version_to_execute().map(Task::Execute)
        }
    }

    /// `idx` read an estimate of `blocking`: park it until `blocking` is executed.
    /// `false` if `blocking` finished meanwhile (re-execute `idx` right away).
    pub(crate) fn add_dependency(&self, idx: usize, blocking: usize) -> bool {
        let mut deps = lock(&self.dependencies[blocking]);
        if lock(&self.status[blocking]).1 == Status::Executed {
            return false;
        }
        lock(&self.status[idx]).1 = Status::Aborting;
        deps.push(idx);
        drop(deps);
        self.num_active_tasks.fetch_sub(1, SeqCst);
        true
    }

    /// `ABORTING(i)` → `READY_TO_EXECUTE(i + 1)`.
    fn set_ready_status(&self, idx: usize) {
        let mut s = lock(&self.status[idx]);
        if s.1 == Status::Aborting {
            *s = (s.0 + 1, Status::ReadyToExecute);
        }
    }

    pub(crate) fn finish_execution(
        &self,
        idx: usize,
        incarnation: usize,
        wrote_new_location: bool,
    ) -> Option<Task> {
        *lock(&self.status[idx]) = (incarnation, Status::Executed);
        let deps = std::mem::take(&mut *lock(&self.dependencies[idx]));
        if let Some(min) = deps.iter().copied().min() {
            for d in &deps {
                self.set_ready_status(*d);
            }
            self.decrease_execution_idx(min);
        }
        if self.validation_idx.load(SeqCst) > idx {
            if wrote_new_location {
                // Later transactions may have read around the new location.
                self.decrease_validation_idx(idx);
            } else {
                return Some(Task::Validate((idx, incarnation)));
            }
        }
        self.num_active_tasks.fetch_sub(1, SeqCst);
        None
    }

    pub(crate) fn try_validation_abort(&self, idx: usize, incarnation: usize) -> bool {
        let mut s = lock(&self.status[idx]);
        if *s == (incarnation, Status::Executed) {
            s.1 = Status::Aborting;
            return true;
        }
        false
    }

    pub(crate) fn finish_validation(&self, idx: usize, aborted: bool) -> Option<Task> {
        if aborted {
            self.set_ready_status(idx);
            self.decrease_validation_idx(idx + 1);
            if self.execution_idx.load(SeqCst) > idx {
                if let Some(version) = self.try_incarnate(idx) {
                    return Some(Task::Execute(version));
                }
            }
        }
        self.num_active_tasks.fetch_sub(1, SeqCst);
        None
    }
}

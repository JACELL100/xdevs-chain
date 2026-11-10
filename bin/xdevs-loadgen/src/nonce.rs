//! Local per-sender nonce tracking (no `eth_getTransactionCount` per tx).
//!
//! Each sender has:
//! * `next`: the next fresh nonce to sign with;
//! * `confirmed`: one past the highest nonce seen included in a block (or the chain's
//!   `latest` nonce after a resync), so `next - confirmed` is the sender's unconfirmed
//!   window, kept below `window` (the node's per-sender pool limit is 64);
//! * `free`: nonces whose tx was definitively rejected (e.g. underpriced, pool full),
//!   reused first so the sender's nonce sequence has no gap;
//! * `sending`: RPC sends in flight;
//! * resync state: after an ambiguous outcome (timeout) or a nonce error (`nonce too
//!   low`, `replacement transaction underpriced`, ...) the sender is paused, and once
//!   its in-flight sends drained, its nonce is re-read from the node (`pending` tag).

use std::collections::BTreeSet;

#[derive(Debug, Clone, Default)]
struct SenderNonce {
    next: u64,
    confirmed: u64,
    free: BTreeSet<u64>,
    sending: u32,
    needs_resync: bool,
    resync_in_progress: bool,
}

/// Nonce manager for all senders.
#[derive(Debug, Clone)]
pub struct NonceManager {
    senders: Vec<SenderNonce>,
    window: u64,
    max_inflight: u64,
    cursor: usize,
}

impl NonceManager {
    /// `initial[i]` is sender `i`'s current (pending) nonce. `window` bounds each
    /// sender's unconfirmed txs, `max_inflight` all senders' together.
    pub fn new(initial: &[u64], window: u64, max_inflight: u64) -> Self {
        Self {
            senders: initial
                .iter()
                .map(|&n| SenderNonce {
                    next: n,
                    confirmed: n,
                    ..Default::default()
                })
                .collect(),
            window: window.max(1),
            max_inflight: max_inflight.max(1),
            cursor: 0,
        }
    }

    /// Number of senders.
    pub fn len(&self) -> usize {
        self.senders.len()
    }

    /// Whether there are no senders.
    pub fn is_empty(&self) -> bool {
        self.senders.is_empty()
    }

    /// Sender `i`'s unconfirmed window (sent or being sent, not seen included).
    pub fn outstanding(&self, i: usize) -> u64 {
        let s = &self.senders[i];
        s.next.saturating_sub(s.confirmed)
    }

    /// All senders' unconfirmed txs.
    pub fn total_outstanding(&self) -> u64 {
        (0..self.senders.len()).map(|i| self.outstanding(i)).sum()
    }

    /// Next nonce sender `i` would sign with (for inspection).
    pub fn next_nonce(&self, i: usize) -> u64 {
        let s = &self.senders[i];
        s.free.first().copied().unwrap_or(s.next)
    }

    /// Reserve a nonce for sender `i` if it is not paused and its window has room.
    /// Marks one send in flight (call [`Self::send_done`] when the RPC returns).
    pub fn reserve(&mut self, i: usize) -> Option<u64> {
        let total = self.total_outstanding();
        let window = self.window;
        let s = &mut self.senders[i];
        if s.needs_resync || s.resync_in_progress {
            return None;
        }
        let nonce = if let Some(n) = s.free.pop_first() {
            n
        } else {
            if s.next.saturating_sub(s.confirmed) >= window || total >= self.max_inflight {
                return None;
            }
            s.next += 1;
            s.next - 1
        };
        s.sending += 1;
        Some(nonce)
    }

    /// Round-robin over senders: reserve a nonce from the next sender that can send.
    pub fn pick(&mut self) -> Option<(usize, u64)> {
        let n = self.senders.len();
        for k in 0..n {
            let i = (self.cursor + k) % n;
            if let Some(nonce) = self.reserve(i) {
                self.cursor = (i + 1) % n;
                return Some((i, nonce));
            }
        }
        None
    }

    /// The RPC send for sender `i` returned (any outcome).
    pub fn send_done(&mut self, i: usize) {
        let s = &mut self.senders[i];
        s.sending = s.sending.saturating_sub(1);
    }

    /// Sender `i`'s tx with `nonce` was definitively rejected: reuse the nonce.
    pub fn release(&mut self, i: usize, nonce: u64) {
        let s = &mut self.senders[i];
        if nonce >= s.confirmed && nonce < s.next {
            s.free.insert(nonce);
        }
    }

    /// Sender `i`'s tx with `nonce` was seen in a block.
    pub fn included(&mut self, i: usize, nonce: u64) {
        let s = &mut self.senders[i];
        s.confirmed = s.confirmed.max(nonce + 1);
        if s.next < s.confirmed {
            s.next = s.confirmed;
        }
        let confirmed = s.confirmed;
        s.free.retain(|&n| n >= confirmed);
    }

    /// Pause sender `i` until its nonce was re-read from the node.
    pub fn mark_resync(&mut self, i: usize) {
        self.senders[i].needs_resync = true;
    }

    /// Whether sender `i` is paused for a resync.
    pub fn is_resyncing(&self, i: usize) -> bool {
        let s = &self.senders[i];
        s.needs_resync || s.resync_in_progress
    }

    /// Senders whose resync can start now (paused and no sends in flight); marks them
    /// in progress so each is returned once.
    pub fn take_resync_ready(&mut self) -> Vec<usize> {
        let mut ready = Vec::new();
        for (i, s) in self.senders.iter_mut().enumerate() {
            if s.needs_resync && !s.resync_in_progress && s.sending == 0 {
                s.needs_resync = false;
                s.resync_in_progress = true;
                ready.push(i);
            }
        }
        ready
    }

    /// Resync of sender `i` finished: the node reports `pending` (pool-aware) and
    /// `latest` (mined) nonces.
    pub fn resynced(&mut self, i: usize, pending: u64, latest: u64) {
        let s = &mut self.senders[i];
        s.confirmed = s.confirmed.max(latest);
        s.next = pending.max(s.confirmed);
        s.free.clear();
        s.resync_in_progress = false;
    }

    /// Resync of sender `i` failed (RPC error): retry later.
    pub fn resync_failed(&mut self, i: usize) {
        let s = &mut self.senders[i];
        s.resync_in_progress = false;
        s.needs_resync = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_nonces_and_window() {
        let mut m = NonceManager::new(&[5, 0], 3, 100);
        assert_eq!(m.reserve(0), Some(5));
        assert_eq!(m.reserve(0), Some(6));
        assert_eq!(m.reserve(0), Some(7));
        assert_eq!(m.reserve(0), None, "window of 3 full");
        assert_eq!(m.outstanding(0), 3);
        m.included(0, 5);
        assert_eq!(m.outstanding(0), 2);
        assert_eq!(m.reserve(0), Some(8));
        // Inclusion of a later nonce implies the earlier ones.
        m.included(0, 8);
        assert_eq!(m.outstanding(0), 0);
        assert_eq!(m.reserve(1), Some(0));
    }

    #[test]
    fn global_inflight_cap() {
        let mut m = NonceManager::new(&[0, 0, 0], 10, 4);
        let picks: Vec<_> = std::iter::from_fn(|| m.pick()).collect();
        assert_eq!(picks.len(), 4);
        // Round-robin across senders.
        assert_eq!(picks[0], (0, 0));
        assert_eq!(picks[1], (1, 0));
        assert_eq!(picks[2], (2, 0));
        assert_eq!(picks[3], (0, 1));
        assert_eq!(m.total_outstanding(), 4);
    }

    #[test]
    fn rejected_nonce_is_reused_first() {
        let mut m = NonceManager::new(&[0], 10, 100);
        for _ in 0..3 {
            m.reserve(0);
        }
        m.release(0, 1);
        assert_eq!(m.next_nonce(0), 1);
        assert_eq!(m.reserve(0), Some(1));
        assert_eq!(m.reserve(0), Some(3));
        // Stale releases (already confirmed) are ignored.
        m.included(0, 3);
        m.release(0, 2);
        assert_eq!(m.reserve(0), Some(4));
    }

    #[test]
    fn resync_waits_for_inflight_sends() {
        let mut m = NonceManager::new(&[0, 0], 10, 100);
        assert_eq!(m.reserve(0), Some(0));
        assert_eq!(m.reserve(0), Some(1));
        m.mark_resync(0);
        assert_eq!(m.reserve(0), None, "paused");
        assert!(m.is_resyncing(0));
        assert!(m.take_resync_ready().is_empty(), "sends in flight");
        m.send_done(0);
        m.send_done(0);
        assert_eq!(m.take_resync_ready(), vec![0]);
        assert!(m.take_resync_ready().is_empty(), "returned once");
        assert_eq!(m.reserve(0), None, "still paused while in progress");
        // Picking skips the paused sender.
        assert_eq!(m.pick(), Some((1, 0)));

        // Failed resync goes back to the queue.
        m.resync_failed(0);
        assert_eq!(m.take_resync_ready(), vec![0]);
        // The node says: 1 mined, nonce 1 lost (pending = latest = 1).
        m.resynced(0, 1, 1);
        assert!(!m.is_resyncing(0));
        assert_eq!(m.outstanding(0), 0);
        assert_eq!(m.reserve(0), Some(1));
    }

    #[test]
    fn resync_after_nonce_too_low() {
        // Someone else used the account: the node is at 10 while we think 2.
        let mut m = NonceManager::new(&[2], 64, 1000);
        assert_eq!(m.reserve(0), Some(2));
        m.send_done(0);
        m.mark_resync(0);
        assert_eq!(m.take_resync_ready(), vec![0]);
        m.resynced(0, 12, 10);
        assert_eq!(m.outstanding(0), 2, "2 pending in the pool");
        assert_eq!(m.reserve(0), Some(12));
    }
}

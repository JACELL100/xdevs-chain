//! Chain events for `eth_subscribe` (plan §19.1, §19.3).
//!
//! [`ChainEvents`] is a pair of **bounded** broadcast channels the node publishes into:
//! one [`BlockEvent`] per committed block (in commit order) and one tx hash per mempool
//! admission. Each subscription holds a receiver; a subscriber that falls more than the
//! channel capacity behind loses events, and the server then closes that subscription
//! with an error notification (it never buffers without bound).

use crate::types::{RpcHeader, RpcLog};
use std::sync::Arc;
use tokio::sync::broadcast;
use xdevs_primitives::B256;

/// Committed blocks kept for slow subscribers (≈ 1 minute at 1 s blocks).
pub const DEFAULT_BLOCK_EVENT_CAPACITY: usize = 64;
/// Pending tx hashes kept for slow subscribers.
pub const DEFAULT_PENDING_TX_CAPACITY: usize = 4096;

/// A committed block as delivered to `newHeads` and `logs` subscribers.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockEvent {
    /// Header in the `eth_getBlockByNumber(n, false)` shape (with `hash` and `size`).
    pub header: RpcHeader,
    /// Every log of the block in block order, shaped exactly like `eth_getLogs`
    /// results (block hash/number, tx hash/index, block-wide `logIndex`,
    /// `removed: false`). Subscriptions filter them.
    pub logs: Vec<RpcLog>,
    /// Number of transactions in the block (for metrics; not part of `newHeads`).
    pub tx_count: usize,
}

/// Broadcaster of chain events (cheap to share behind an [`Arc`]).
#[derive(Debug)]
pub struct ChainEvents {
    blocks: broadcast::Sender<Arc<BlockEvent>>,
    pending_txs: broadcast::Sender<B256>,
}

impl Default for ChainEvents {
    fn default() -> Self {
        Self::new(DEFAULT_BLOCK_EVENT_CAPACITY, DEFAULT_PENDING_TX_CAPACITY)
    }
}

impl ChainEvents {
    /// New broadcaster keeping at most `block_capacity` blocks and `pending_capacity`
    /// tx hashes for slow receivers (both at least 1).
    pub fn new(block_capacity: usize, pending_capacity: usize) -> Self {
        Self {
            blocks: broadcast::channel(block_capacity.max(1)).0,
            pending_txs: broadcast::channel(pending_capacity.max(1)).0,
        }
    }

    /// `true` if anyone listens for blocks (lets the publisher skip building events).
    pub fn has_block_subscribers(&self) -> bool {
        self.blocks.receiver_count() > 0
    }

    /// `true` if anyone listens for pending tx hashes.
    pub fn has_pending_tx_subscribers(&self) -> bool {
        self.pending_txs.receiver_count() > 0
    }

    /// Publish a committed block (call in commit order). No-op without subscribers.
    pub fn publish_block(&self, event: BlockEvent) {
        if self.has_block_subscribers() {
            let _ = self.blocks.send(Arc::new(event));
        }
    }

    /// Publish the hash of a transaction admitted to the mempool. No-op without
    /// subscribers.
    pub fn publish_pending_tx(&self, hash: B256) {
        if self.has_pending_tx_subscribers() {
            let _ = self.pending_txs.send(hash);
        }
    }

    /// Receiver of every block committed from now on.
    pub fn subscribe_blocks(&self) -> broadcast::Receiver<Arc<BlockEvent>> {
        self.blocks.subscribe()
    }

    /// Receiver of every tx hash admitted from now on.
    pub fn subscribe_pending_txs(&self) -> broadcast::Receiver<B256> {
        self.pending_txs.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::TryRecvError;

    fn event(n: u64) -> BlockEvent {
        let mut header: RpcHeader = RpcHeader::default();
        header.inner.number = n;
        BlockEvent {
            header,
            logs: Vec::new(),
            tx_count: 0,
        }
    }

    #[test]
    fn publishes_only_with_subscribers_and_bounds_lag() {
        let events = ChainEvents::new(2, 2);
        assert!(!events.has_block_subscribers());
        events.publish_block(event(0)); // dropped: nobody listens
        let mut rx = events.subscribe_blocks();
        assert!(events.has_block_subscribers());
        for n in 1..=4 {
            events.publish_block(event(n));
        }
        // Capacity 2: the two oldest were overwritten.
        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Lagged(2));
        assert_eq!(rx.try_recv().unwrap().header.inner.number, 3);
        assert_eq!(rx.try_recv().unwrap().header.inner.number, 4);
        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);

        let mut p = events.subscribe_pending_txs();
        events.publish_pending_tx(B256::repeat_byte(1));
        assert_eq!(p.try_recv().unwrap(), B256::repeat_byte(1));
    }
}

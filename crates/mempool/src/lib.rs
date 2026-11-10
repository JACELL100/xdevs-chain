//! Transaction pool (plan §13). **Owner: mempool agent.**
//!
//! This file fixes the shared interface: [`TxPool`], [`TxOrigin`], [`PoolError`],
//! [`PoolConfig`] and [`PoolStatus`]. [`Pool`] is the concrete implementation:
//! entry validation (§13.1, [`validation`]), pending / queued sub-pools per sender,
//! +10% replacement bump, ordering by effective tip then arrival, lowest-fee eviction
//! and a queued-tx TTL measured in block timestamps (no wall clock).
//!
//! XIP-0001 (EOA lock, spec §5 e): admission applies the same predicates as block
//! validity against the head state ("first tx of the next block"); every new head
//! re-checks every sender's lock status and evicts what became invalid; `LOCKED` senders
//! may hold at most 4 txs and [`TxPool::best_txs`] yields at most one of them per block.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod pool;
pub mod validation;

pub use pool::{AccountReader, FromState, Pool};
pub use validation::{intrinsic_gas, PoolParams};

use serde::{Deserialize, Serialize};
use xdevs_primitives::{
    xip0001::Xip0001Error, Address, Block, ReceiptEnvelope, RecoveredTx, TxHash, U256,
};

/// Where a transaction came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TxOrigin {
    /// Submitted to this node over RPC (`eth_sendRawTransaction`).
    Local,
    /// Received from a peer (Phase 2 gossip).
    External,
}

/// Mempool limits (plan §13.1, §33.2 `[mempool]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PoolConfig {
    /// Max number of transactions (pending + queued).
    pub max_txs: usize,
    /// Max total encoded size in bytes.
    pub max_bytes: usize,
    /// Max transactions per sender.
    pub max_per_sender: usize,
    /// Max nonce gap above the account nonce for queued txs.
    pub max_nonce_gap: u64,
    /// Fee bump (percent) required to replace a tx with the same sender + nonce.
    pub price_bump_percent: u64,
    /// Queued (future-nonce) tx lifetime in seconds.
    pub queued_ttl_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_txs: 10_000,
            max_bytes: 64 * 1024 * 1024,
            max_per_sender: 64,
            max_nonce_gap: 64,
            price_bump_percent: 10,
            queued_ttl_secs: 3 * 60 * 60,
        }
    }
}

/// Counts reported by `txpool_status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolStatus {
    /// Executable transactions.
    pub pending: usize,
    /// Future-nonce transactions.
    pub queued: usize,
}

/// Why a transaction was rejected. Messages follow geth's wording where tools rely on it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PoolError {
    /// Encoded size above the 128 KiB limit.
    #[error("oversized data: {size} > {max} bytes")]
    Oversized {
        /// Encoded size.
        size: usize,
        /// Limit.
        max: usize,
    },
    /// Transaction type not supported (e.g. 0x03 blob txs).
    #[error("transaction type not supported: 0x{0:02x}")]
    UnsupportedType(u8),
    /// EIP-7702 set-code transaction with an empty authorization list.
    #[error("EIP-7702 transaction with empty auth list")]
    EmptyAuthorizationList,
    /// Rejected by XIP-0001 (EOA lock): the sender is sealed, or locked and the tx is not
    /// a value-0 self-call within the gas / fee limits. RPC: code -32003 with
    /// `data.reason` = [`Xip0001Error::violation`]`.reason()`.
    #[error(transparent)]
    Xip0001(#[from] Xip0001Error),
    /// Signature could not be recovered.
    #[error("invalid sender: {0}")]
    InvalidSignature(String),
    /// Missing chain id (pre-EIP-155 legacy) or different chain id.
    #[error("invalid chain id: expected {expected}, got {got:?}")]
    WrongChainId {
        /// This chain's id.
        expected: u64,
        /// Id in the tx (`None` = unprotected legacy tx).
        got: Option<u64>,
    },
    /// Nonce below the account nonce.
    #[error("nonce too low: next nonce {state}, tx nonce {tx}")]
    NonceTooLow {
        /// Account nonce.
        state: u64,
        /// Tx nonce.
        tx: u64,
    },
    /// Nonce too far ahead of the account nonce.
    #[error("nonce too high: next nonce {state}, tx nonce {tx}")]
    NonceGapTooLarge {
        /// Account nonce.
        state: u64,
        /// Tx nonce.
        tx: u64,
    },
    /// Balance below `gas_limit * max_fee + value`.
    #[error("insufficient funds for gas * price + value: balance {balance}, tx cost {cost}")]
    InsufficientFunds {
        /// Sender balance.
        balance: U256,
        /// Max cost of the tx.
        cost: U256,
    },
    /// `max_fee_per_gas` below the pool's minimum (≈ 0.9 × current base fee).
    #[error("transaction underpriced: max fee {max_fee} < required {required}")]
    Underpriced {
        /// Tx max fee per gas.
        max_fee: u128,
        /// Required minimum.
        required: u128,
    },
    /// Replacement for same sender+nonce without the required fee bump.
    #[error("replacement transaction underpriced")]
    ReplacementUnderpriced,
    /// Gas limit below intrinsic gas.
    #[error("intrinsic gas too low: have {gas_limit}, want {intrinsic}")]
    IntrinsicGasTooLow {
        /// Tx gas limit.
        gas_limit: u64,
        /// Intrinsic gas.
        intrinsic: u64,
    },
    /// Gas limit above the block gas limit.
    #[error("exceeds block gas limit")]
    GasLimitExceedsBlock,
    /// Gas limit above the per-transaction cap (EIP-7825), if one is configured.
    #[error("transaction gas limit too high (cap: {cap}, tx: {gas_limit})")]
    GasLimitTooHigh {
        /// Tx gas limit.
        gas_limit: u64,
        /// Per-tx cap.
        cap: u64,
    },
    /// `max_priority_fee_per_gas > max_fee_per_gas`.
    #[error("max priority fee per gas higher than max fee per gas: tip {tip}, max fee {max_fee}")]
    TipAboveFeeCap {
        /// Tx max fee per gas.
        max_fee: u128,
        /// Tx max priority fee per gas.
        tip: u128,
    },
    /// Contract creation with init code above the EIP-3860 limit.
    #[error("max initcode size exceeded: code size {size} limit {max}")]
    InitCodeTooLarge {
        /// Init code size.
        size: usize,
        /// Limit (49 152).
        max: usize,
    },
    /// Same tx hash already in the pool.
    #[error("already known")]
    AlreadyKnown,
    /// Sender has too many pending txs.
    #[error("sender {0} has too many transactions in the pool")]
    SenderLimit(Address),
    /// Pool full and the tx does not pay more than the cheapest one.
    #[error("txpool is full")]
    PoolFull,
    /// State lookup failed.
    #[error("state error: {0}")]
    State(String),
}

/// Pending transaction pool (plan §7.3). Implementations use interior mutability
/// and must be safe to share between the RPC server and the block producer.
pub trait TxPool: Send + Sync {
    /// Validate and insert a transaction; returns its hash.
    fn add(&self, tx: RecoveredTx, origin: TxOrigin) -> Result<TxHash, PoolError>;

    /// Executable transactions for the next block, ordered by effective tip at
    /// `base_fee` (then arrival), nonce-ordered per sender, total `gas_limit` ≤ `gas_limit`.
    /// Does not remove them.
    fn best_txs(&self, base_fee: u64, gas_limit: u64) -> Vec<RecoveredTx>;

    /// A block was committed: drop included txs, promote queued txs whose nonce gap
    /// closed, re-check balances / nonces against the new state.
    fn on_block_committed(&self, block: &Block, receipts: &[ReceiptEnvelope]);

    /// Remove transactions by hash (e.g. rejected by the block builder).
    fn remove(&self, hashes: &[TxHash]);

    /// Look up a pooled transaction.
    fn get(&self, hash: &TxHash) -> Option<RecoveredTx>;

    /// `true` if the pool holds `hash`.
    fn contains(&self, hash: &TxHash) -> bool {
        self.get(hash).is_some()
    }

    /// Next nonce for `sender` counting its pending (executable) pool txs, or `None`
    /// if the pool holds none (`eth_getTransactionCount(addr, "pending")`).
    fn pending_nonce(&self, sender: &Address) -> Option<u64>;

    /// All pending txs, nonce-ordered per sender (`pending` block tag, `txpool_content`).
    fn pending(&self) -> Vec<RecoveredTx>;

    /// Pending / queued counts.
    fn status(&self) -> PoolStatus;
}

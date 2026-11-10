//! Block execution for xDEVS Chain (plan §10) on top of `revm`.
//!
//! **Owner: execution agent.** This file fixes the shared interface:
//!
//! * [`BlockExecutor`] — strict execution of a given block ([`BlockExecutor::execute`])
//!   and block building from candidate txs ([`BlockExecutor::build`]).
//! * [`ExecutionOutcome`] — roots, gas, receipts and the [`StateDiff`] of a block.
//! * [`next_base_fee`] — EIP-1559 base fee with the 0.1 gwei floor (§15.1).
//! * [`EvmExecutor::call`] / [`EvmExecutor::estimate_gas`] — single-call execution for
//!   `eth_call` / `eth_estimateGas`.
//!
//! XIP-0001 (EOA lock, `docs/specs/xip-0001-eoa-lock.md`) is enforced by both
//! [`BlockExecutor::execute`] (an invalid tx fails the block) and
//! [`BlockExecutor::build`] (invalid txs are left out), from
//! [`ChainConfig::xip0001_block`] on. The shared classification is
//! [`xdevs_primitives::xip0001`] / [`xdevs_state::StateProvider::eoa_lock_status`].
//!
//! Fee rule (§10.2): per tx, `base_fee * gas_used` is split `burn_bps` burned /
//! rest to `ChainConfig::treasury`; the tip `min(max_priority, max_fee - base) * gas_used`
//! goes to `header.beneficiary`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod base_fee;
mod db;
mod env;
mod error;
mod executor;
#[cfg(test)]
mod tests;
mod xip0001;

pub use base_fee::next_base_fee;
pub use db::StateDbError;
pub use env::{BLOB_TX_TYPE, SPEC};
pub use error::ExecutionError;
use xdevs_primitives::{
    Address, Block, Bloom, Bytes, ChainConfig, Header, Log, ReceiptEnvelope, RecoveredTx, TxHash,
    B256,
};
use xdevs_state::{StateDiff, StateProvider};

/// Result of executing a block on top of its parent state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionOutcome {
    /// State root after the block (parent state + `state_diff`).
    pub state_root: B256,
    /// Ethereum receipts root.
    pub receipts_root: B256,
    /// Union of all receipt blooms.
    pub logs_bloom: Bloom,
    /// Total gas used by the block's transactions.
    pub gas_used: u64,
    /// One receipt per transaction, in order (cumulative gas, status, logs, bloom).
    pub receipts: Vec<ReceiptEnvelope>,
    /// All state changes of the block (accounts, storage, code, fee payouts).
    pub state_diff: StateDiff,
}

/// Inputs for building a new block that are not derived from the parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockAttributes {
    /// Header timestamp (seconds). Must be `> parent.timestamp`.
    pub timestamp: u64,
    /// Fee recipient of the proposer (receives tips).
    pub beneficiary: Address,
    /// Value of `mix_hash` / `PREVRANDAO` (dev mode: any deterministic value).
    pub prev_randao: B256,
    /// Header `extra_data` (≤ 32 bytes).
    pub extra_data: Bytes,
    /// Block gas limit (normally `ChainConfig::block_gas_limit`).
    pub gas_limit: u64,
}

/// A freshly built block.
#[derive(Clone, Debug)]
pub struct BuiltBlock {
    /// The sealed-ready block: header has all roots, gas used, base fee filled in and
    /// [`xdevs_primitives::set_compat_header_fields`] applied.
    pub block: Block,
    /// Execution outcome of `block` (same roots as its header).
    pub outcome: ExecutionOutcome,
    /// Senders of `block.body`, in order (saves re-recovery when indexing).
    pub senders: Vec<Address>,
    /// Candidate transactions that are invalid against the parent state (nonce too low,
    /// insufficient funds, intrinsic gas too low, wrong chain id, blob type, ...) with the
    /// reason. The mempool may drop them.
    ///
    /// Transactions that are merely *not includable now* are skipped silently and are
    /// **not** listed: they do not fit the remaining block gas, their fee cap is below
    /// the base fee, their nonce is ahead of the state, or an earlier tx of the same
    /// sender was skipped (so later nonces stay in the pool).
    pub rejected: Vec<(TxHash, ExecutionError)>,
}

/// Executes blocks with revm (plan §7.3).
pub trait BlockExecutor: Send + Sync {
    /// Execute `block` (strict: any invalid tx fails the block) on `state`, which must
    /// be the state after `parent`. Does **not** compare roots with `block.header`; the
    /// caller does that.
    fn execute(
        &self,
        parent: &Header,
        block: &Block,
        state: &dyn StateProvider,
    ) -> Result<ExecutionOutcome, ExecutionError>;

    /// Build a block on top of `parent` from candidate txs (already ordered by the
    /// mempool). Invalid / non-fitting txs are skipped and reported in `rejected`.
    fn build(
        &self,
        parent: &Header,
        attrs: BlockAttributes,
        candidates: Vec<RecoveredTx>,
        state: &dyn StateProvider,
    ) -> Result<BuiltBlock, ExecutionError>;
}

/// Outcome of a single message call (`eth_call`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallResult {
    /// Call succeeded.
    Success {
        /// Return data.
        output: Bytes,
        /// Gas used (after refunds).
        gas_used: u64,
        /// Logs that would be emitted.
        logs: Vec<Log>,
    },
    /// Call reverted (`REVERT` opcode); `output` is the revert data.
    Revert {
        /// Revert data (ABI-encoded `Error(string)` or custom error).
        output: Bytes,
        /// Gas used.
        gas_used: u64,
    },
    /// Call halted (out of gas, invalid opcode, stack overflow, ...).
    Halt {
        /// Human-readable halt reason.
        reason: String,
        /// Gas used (normally the whole gas limit).
        gas_used: u64,
    },
}

impl CallResult {
    /// Gas used in any outcome.
    pub const fn gas_used(&self) -> u64 {
        match self {
            CallResult::Success { gas_used, .. }
            | CallResult::Revert { gas_used, .. }
            | CallResult::Halt { gas_used, .. } => *gas_used,
        }
    }

    /// `true` for [`CallResult::Success`].
    pub const fn is_success(&self) -> bool {
        matches!(self, CallResult::Success { .. })
    }
}

/// The revm-based executor: implements [`BlockExecutor`] plus call/estimate helpers.
#[derive(Clone, Debug)]
pub struct EvmExecutor {
    cfg: ChainConfig,
}

impl EvmExecutor {
    /// Executor for the given chain.
    pub const fn new(cfg: ChainConfig) -> Self {
        Self { cfg }
    }

    /// Chain configuration.
    pub const fn chain_config(&self) -> &ChainConfig {
        &self.cfg
    }
}

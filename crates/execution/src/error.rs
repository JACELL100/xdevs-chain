//! Execution errors.

use xdevs_primitives::{xip0001::Xip0001Error, Bytes, TxHash, B256};
use xdevs_state::StateError;

/// Errors from block execution, block building and message calls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    /// A transaction is invalid in this block (bad nonce, insufficient funds, fee below
    /// base fee, intrinsic gas too low, wrong chain id, unsupported type, ...).
    #[error("invalid transaction {hash}: {reason}")]
    InvalidTransaction {
        /// Transaction hash.
        hash: TxHash,
        /// Why it is invalid.
        reason: String,
    },
    /// The transaction breaks an XIP-0001 (EOA lock) rule at its position: in a block
    /// (`hash` set) it makes the block invalid; for `eth_estimateGas` (`hash = None`) the
    /// request describes a transaction that can never be included.
    #[error("{}{error}", hash.map(|h| format!("invalid transaction {h}: ")).unwrap_or_default())]
    Xip0001 {
        /// Transaction hash (`None` for call requests).
        hash: Option<TxHash>,
        /// The rule that is broken (boxed: keeps `ExecutionError` small).
        error: Box<Xip0001Error>,
    },
    /// Transaction type not supported by xDEVS Chain (e.g. 0x03 blobs).
    #[error("unsupported transaction type 0x{0:02x}")]
    UnsupportedTxType(u8),
    /// Sum of tx gas limits exceeds the block gas limit.
    #[error("block gas limit exceeded: {used} > {limit}")]
    BlockGasLimitExceeded {
        /// Gas that would be used.
        used: u64,
        /// Block gas limit.
        limit: u64,
    },
    /// Header inconsistent with its parent or the chain config.
    #[error("invalid header: {0}")]
    InvalidHeader(String),
    /// A computed root differs from the one in the header.
    #[error("{field} mismatch: header {expected}, computed {got}")]
    RootMismatch {
        /// Which root (`state_root`, `receipts_root`, `transactions_root`, ...).
        field: &'static str,
        /// Value in the header.
        expected: B256,
        /// Value computed by execution.
        got: B256,
    },
    /// `eth_call` / `eth_estimateGas` reverted.
    #[error("execution reverted")]
    Reverted {
        /// Revert data.
        output: Bytes,
    },
    /// `eth_estimateGas`: the call fails even with the gas cap.
    #[error("gas required exceeds allowance ({cap})")]
    GasCapExceeded {
        /// The cap that was tried.
        cap: u64,
    },
    /// Invalid call request (`eth_call` / `eth_estimateGas` parameters).
    #[error("invalid call request: {0}")]
    InvalidRequest(String),
    /// `eth_call` / `eth_estimateGas` stopped by its [`crate::CallLimits`] (`"timeout"` or
    /// `"cancelled"`). Says nothing about the call itself.
    #[error("execution aborted ({0})")]
    Aborted(&'static str),
    /// State access failed.
    #[error("state error: {0}")]
    State(String),
    /// Unexpected EVM / database error.
    #[error("evm error: {0}")]
    Evm(String),
}

impl From<StateError> for ExecutionError {
    fn from(e: StateError) -> Self {
        ExecutionError::State(e.to_string())
    }
}

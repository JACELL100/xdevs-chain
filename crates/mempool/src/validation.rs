//! Stateless transaction checks (plan §13.1) and intrinsic gas.

use xdevs_primitives::{constants::MAX_TX_SIZE, ChainConfig, RecoveredTx, Transaction, TxEnvelope};

use crate::PoolError;

/// Base cost of every transaction.
pub const TX_BASE_GAS: u64 = 21_000;
/// Extra cost of a contract-creation transaction (EIP-2).
pub const TX_CREATE_GAS: u64 = 32_000;
/// Calldata zero byte (EIP-2028).
pub const TX_DATA_ZERO_GAS: u64 = 4;
/// Calldata non-zero byte (EIP-2028).
pub const TX_DATA_NON_ZERO_GAS: u64 = 16;
/// Per access-list address (EIP-2930).
pub const ACCESS_LIST_ADDRESS_GAS: u64 = 2_400;
/// Per access-list storage key (EIP-2930).
pub const ACCESS_LIST_STORAGE_KEY_GAS: u64 = 1_900;
/// Per 32-byte word of init code (EIP-3860).
pub const INIT_CODE_WORD_GAS: u64 = 2;
/// Per EIP-7702 authorization (`PER_EMPTY_ACCOUNT_COST`).
pub const PER_AUTHORIZATION_GAS: u64 = 25_000;
/// EIP-7623 calldata floor cost per token.
pub const FLOOR_GAS_PER_TOKEN: u64 = 10;
/// Max init code size (EIP-3860: 2 × 24 576).
pub const MAX_INIT_CODE_SIZE: usize = 49_152;

/// Chain-derived validation parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolParams {
    /// Expected EIP-155 chain id.
    pub chain_id: u64,
    /// Block gas limit: no tx may use more.
    pub block_gas_limit: u64,
    /// Optional per-tx gas cap (EIP-7825 / Osaka: 2^24). `None` = only the block limit.
    pub max_tx_gas: Option<u64>,
    /// Max EIP-2718 encoded size (128 KiB).
    pub max_tx_size: usize,
}

impl PoolParams {
    /// Parameters for `cfg` (no per-tx gas cap).
    pub fn from_chain_config(cfg: &ChainConfig) -> Self {
        Self {
            chain_id: cfg.chain_id,
            block_gas_limit: cfg.block_gas_limit,
            max_tx_gas: None,
            max_tx_size: MAX_TX_SIZE,
        }
    }
}

/// Gas a transaction must provide before execution: `max(intrinsic, EIP-7623 floor)`.
///
/// Intrinsic = 21000 + calldata (4 / 16 per byte) + 32000 and 2 per init-code word for
/// creations + access list + 25000 per authorization. Floor = 21000 + 10 × tokens where
/// tokens = zero bytes + 4 × non-zero bytes (Prague).
pub fn intrinsic_gas(tx: &TxEnvelope) -> u64 {
    let input = tx.input();
    let zeros = input.iter().filter(|b| **b == 0).count() as u64;
    let non_zeros = input.len() as u64 - zeros;

    let mut gas = TX_BASE_GAS
        .saturating_add(zeros.saturating_mul(TX_DATA_ZERO_GAS))
        .saturating_add(non_zeros.saturating_mul(TX_DATA_NON_ZERO_GAS));
    if tx.is_create() {
        let words = (input.len() as u64).div_ceil(32);
        gas = gas
            .saturating_add(TX_CREATE_GAS)
            .saturating_add(words.saturating_mul(INIT_CODE_WORD_GAS));
    }
    if let Some(list) = tx.access_list() {
        for item in list.iter() {
            gas = gas.saturating_add(ACCESS_LIST_ADDRESS_GAS).saturating_add(
                (item.storage_keys.len() as u64).saturating_mul(ACCESS_LIST_STORAGE_KEY_GAS),
            );
        }
    }
    if let Some(auths) = tx.authorization_list() {
        gas = gas.saturating_add((auths.len() as u64).saturating_mul(PER_AUTHORIZATION_GAS));
    }

    let tokens = zeros.saturating_add(non_zeros.saturating_mul(4));
    let floor = TX_BASE_GAS.saturating_add(tokens.saturating_mul(FLOOR_GAS_PER_TOKEN));
    gas.max(floor)
}

/// Minimum `max_fee_per_gas` accepted at `base_fee`: `ceil(0.9 × base_fee)`.
pub fn min_max_fee(base_fee: u64) -> u128 {
    (u128::from(base_fee) * 9).div_ceil(10)
}

/// Checks that need neither pool nor account state: size, type, chain id, fee caps,
/// gas limits. Signature recovery happened before (the tx is a [`RecoveredTx`]).
pub fn validate_stateless(
    tx: &RecoveredTx,
    params: &PoolParams,
    base_fee: u64,
) -> Result<(), PoolError> {
    let size = tx.encoded_len();
    if size > params.max_tx_size {
        return Err(PoolError::Oversized {
            size,
            max: params.max_tx_size,
        });
    }
    // Phase 1: legacy, EIP-2930, EIP-1559. Blob (0x03) and EIP-7702 (0x04) are refused.
    let ty = tx.tx_type_byte();
    if ty > 2 {
        return Err(PoolError::UnsupportedType(ty));
    }
    match tx.chain_id() {
        Some(id) if id == params.chain_id => {}
        got => {
            return Err(PoolError::WrongChainId {
                expected: params.chain_id,
                got,
            })
        }
    }
    if let Some(tip) = tx.max_priority_fee_per_gas() {
        if tip > tx.max_fee_per_gas() {
            return Err(PoolError::TipAboveFeeCap {
                max_fee: tx.max_fee_per_gas(),
                tip,
            });
        }
    }
    if tx.is_create() && tx.input().len() > MAX_INIT_CODE_SIZE {
        return Err(PoolError::InitCodeTooLarge {
            size: tx.input().len(),
            max: MAX_INIT_CODE_SIZE,
        });
    }
    let gas_limit = tx.gas_limit();
    if gas_limit > params.block_gas_limit {
        return Err(PoolError::GasLimitExceedsBlock);
    }
    if let Some(cap) = params.max_tx_gas {
        if gas_limit > cap {
            return Err(PoolError::GasLimitTooHigh { gas_limit, cap });
        }
    }
    let intrinsic = intrinsic_gas(tx);
    if gas_limit < intrinsic {
        return Err(PoolError::IntrinsicGasTooLow {
            gas_limit,
            intrinsic,
        });
    }
    let required = min_max_fee(base_fee);
    if tx.max_fee_per_gas() < required {
        return Err(PoolError::Underpriced {
            max_fee: tx.max_fee_per_gas(),
            required,
        });
    }
    Ok(())
}

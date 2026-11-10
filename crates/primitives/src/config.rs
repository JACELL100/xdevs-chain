//! Chain configuration (plan §15.1, §21.1, §33.1).

use crate::{constants::*, serde_helpers::u64_flex};
use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

/// EIP-1559 base-fee parameters plus the xDEVS burn/treasury split.
///
/// JSON shape (genesis `config.baseFee`):
/// `{ "initial": "1000000000", "minimum": "100000000", "elasticity": 2,
///    "maxChangeDenominator": 8, "burnBps": 8000 }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseFeeParams {
    /// Base fee (wei) of the genesis block / first block.
    #[serde(with = "u64_flex", default = "d_initial")]
    pub initial: u64,
    /// Floor: the base fee never drops below this (wei). Default 0.1 gwei.
    #[serde(with = "u64_flex", default = "d_minimum")]
    pub minimum: u64,
    /// Elasticity multiplier: gas target = gas limit / elasticity. Default 2.
    #[serde(with = "u64_flex", default = "d_elasticity")]
    pub elasticity: u64,
    /// Max change denominator: base fee moves by at most `1/d` per block. Default 8.
    #[serde(with = "u64_flex", default = "d_denominator")]
    pub max_change_denominator: u64,
    /// Basis points of the base fee that are burned; the rest goes to the treasury.
    #[serde(default = "d_burn_bps")]
    pub burn_bps: u16,
}

const fn d_initial() -> u64 {
    INITIAL_BASE_FEE
}
const fn d_minimum() -> u64 {
    MIN_BASE_FEE
}
const fn d_elasticity() -> u64 {
    BASE_FEE_ELASTICITY
}
const fn d_denominator() -> u64 {
    BASE_FEE_MAX_CHANGE_DENOMINATOR
}
const fn d_burn_bps() -> u16 {
    BURN_BPS
}
const fn d_block_gas_limit() -> u64 {
    DEFAULT_BLOCK_GAS_LIMIT
}
const fn d_block_time_ms() -> u64 {
    DEFAULT_BLOCK_TIME_MS
}
const fn d_treasury() -> Address {
    TREASURY_ADDRESS
}
const fn d_system() -> Address {
    SYSTEM_ADDRESS
}

impl Default for BaseFeeParams {
    fn default() -> Self {
        Self {
            initial: INITIAL_BASE_FEE,
            minimum: MIN_BASE_FEE,
            elasticity: BASE_FEE_ELASTICITY,
            max_change_denominator: BASE_FEE_MAX_CHANGE_DENOMINATOR,
            burn_bps: BURN_BPS,
        }
    }
}

impl BaseFeeParams {
    /// Gas target for a block with the given gas limit (`gas_limit / elasticity`).
    pub const fn gas_target(&self, gas_limit: u64) -> u64 {
        gas_limit / self.elasticity
    }
}

/// Consensus-relevant chain parameters, shared by execution, mempool, RPC and node.
///
/// Deserializes from the genesis `config` object (§21.1); unknown keys such as
/// `name`, `hardfork`, `epochLength` or `consensus` are ignored in Phase 1.
/// `blockGasLimit` is not part of the genesis `config` object — the genesis loader
/// overrides it with the top-level `gasLimit`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainConfig {
    /// EIP-155 chain id.
    #[serde(with = "u64_flex")]
    pub chain_id: u64,
    /// Block gas limit.
    #[serde(with = "u64_flex", default = "d_block_gas_limit")]
    pub block_gas_limit: u64,
    /// Target block time in milliseconds (dev-mode producer interval).
    #[serde(with = "u64_flex", default = "d_block_time_ms")]
    pub block_time_ms: u64,
    /// EIP-1559 base fee parameters and burn split.
    #[serde(default)]
    pub base_fee: BaseFeeParams,
    /// Receiver of the non-burned part of the base fee (Treasury system contract).
    #[serde(default = "d_treasury")]
    pub treasury: Address,
    /// `msg.sender` of protocol-initiated system calls.
    #[serde(default = "d_system")]
    pub system_address: Address,
}

impl ChainConfig {
    /// Default configuration for a given chain id.
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            block_gas_limit: DEFAULT_BLOCK_GAS_LIMIT,
            block_time_ms: DEFAULT_BLOCK_TIME_MS,
            base_fee: BaseFeeParams::default(),
            treasury: TREASURY_ADDRESS,
            system_address: SYSTEM_ADDRESS,
        }
    }

    /// Configuration of the local single-node dev chain ([`DEV_CHAIN_ID`]).
    pub fn dev() -> Self {
        Self::new(DEV_CHAIN_ID)
    }

    /// Split a base-fee amount (wei) into `(burned, to_treasury)` using `burn_bps`.
    ///
    /// Integer math; the rounding remainder goes to the treasury so no wei is lost.
    pub fn split_base_fee(&self, amount: U256) -> (U256, U256) {
        let burned = amount * U256::from(self.base_fee.burn_bps) / U256::from(BPS_DENOMINATOR);
        (burned, amount - burned)
    }
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self::dev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_genesis_config_object() {
        let json = r#"{
            "chainId": 717171, "name": "xdevs-testnet-1", "hardfork": "xdevs-v1",
            "blockTimeMs": 2000, "epochLength": 3600,
            "baseFee": { "initial": "1000000000", "minimum": "100000000", "elasticity": 2,
                         "maxChangeDenominator": 8, "burnBps": 8000 },
            "consensus": { "engine": "malachite" }
        }"#;
        let cfg: ChainConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.chain_id, 717171);
        assert_eq!(cfg.block_time_ms, 2000);
        assert_eq!(cfg.block_gas_limit, DEFAULT_BLOCK_GAS_LIMIT);
        assert_eq!(cfg.base_fee, BaseFeeParams::default());
        assert_eq!(cfg.treasury, TREASURY_ADDRESS);
        assert_eq!(cfg.system_address, SYSTEM_ADDRESS);
    }

    #[test]
    fn json_roundtrip() {
        let cfg = ChainConfig::dev();
        let s = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<ChainConfig>(&s).unwrap(), cfg);
        assert_eq!(cfg.chain_id, DEV_CHAIN_ID);
    }

    #[test]
    fn burn_split() {
        let cfg = ChainConfig::dev();
        let (burn, treasury) = cfg.split_base_fee(U256::from(1_001u64));
        assert_eq!(burn, U256::from(800u64));
        assert_eq!(treasury, U256::from(201u64));
        assert_eq!(MIN_BASE_FEE, 100_000_000);
        assert_eq!(cfg.base_fee.gas_target(30_000_000), 15_000_000);
    }
}

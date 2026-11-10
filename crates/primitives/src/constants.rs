//! Protocol constants (§15, §18.2, §33.1 of the implementation plan).

use alloy_primitives::{address, Address};

/// Chain id used by local, single-node **dev** chains (`xdevs node --dev`).
///
/// For local development only: it is not registered on chainlist and must never be
/// used for a public network (see plan §8.5).
pub const DEV_CHAIN_ID: u64 = 7117;

/// One gwei in wei.
pub const GWEI: u64 = 1_000_000_000;

/// Default block gas limit (30M, plan §33.1).
pub const DEFAULT_BLOCK_GAS_LIMIT: u64 = 30_000_000;

/// Default target block time in milliseconds (2 s, plan §33.1).
pub const DEFAULT_BLOCK_TIME_MS: u64 = 2_000;

/// Base fee of the first block after genesis (1 gwei).
pub const INITIAL_BASE_FEE: u64 = GWEI;

/// Minimum base fee: 0.1 gwei anti-spam floor (plan §15.1).
pub const MIN_BASE_FEE: u64 = GWEI / 10;

/// EIP-1559 elasticity multiplier: gas target = gas limit / 2.
pub const BASE_FEE_ELASTICITY: u64 = 2;

/// EIP-1559 max change denominator: base fee moves by at most 1/8 = 12.5% per block.
pub const BASE_FEE_MAX_CHANGE_DENOMINATOR: u64 = 8;

/// Share of the base fee that is burned, in basis points (80%). The rest goes to the treasury.
pub const BURN_BPS: u16 = 8_000;

/// Basis-point denominator (100%).
pub const BPS_DENOMINATOR: u16 = 10_000;

/// Maximum encoded transaction size accepted by the mempool (128 KiB, plan §13.1).
pub const MAX_TX_SIZE: usize = 128 * 1024;

/// Maximum `extra_data` length in a header (Ethereum rule).
pub const MAX_EXTRA_DATA_SIZE: usize = 32;

/// Address of the `Staking` system contract (plan §18).
pub const STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");

/// Address of the `Governance` system contract (plan §18).
pub const GOVERNANCE_ADDRESS: Address = address!("0x0000000000000000000000000000000000001001");

/// Address of the `Treasury` system contract that receives 20% of base fees (plan §15.1, §18).
pub const TREASURY_ADDRESS: Address = address!("0x0000000000000000000000000000000000001002");

/// Address of the `FeeConfig` system contract (plan §18).
pub const FEE_CONFIG_ADDRESS: Address = address!("0x0000000000000000000000000000000000001003");

/// Ethereum's system address: `msg.sender` of protocol-initiated calls (plan §18.2).
pub const SYSTEM_ADDRESS: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

/// Client version string reported by `web3_clientVersion` and put in dev-mode `extra_data`.
pub const CLIENT_VERSION: &str = concat!("xdevs/v", env!("CARGO_PKG_VERSION"));

/// EIP-7685 `requests_hash` of a block with no requests: `sha256("")`.
pub const EMPTY_REQUESTS_HASH: alloy_primitives::B256 =
    alloy_primitives::b256!("0xe3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

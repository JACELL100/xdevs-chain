//! Chain configuration (plan §15.1, §21.1, §33.1).

use crate::{
    constants::*,
    serde_helpers::{opt_u64_flex, u256_flex, u64_flex},
};
use alloy_primitives::{Address, Bytes, U256};
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
const fn d_epoch_length() -> u64 {
    DEFAULT_EPOCH_LENGTH
}
fn is_zero_u256(v: &U256) -> bool {
    v.is_zero()
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
/// `name`, `hardfork` or `consensus` are ignored.
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
    /// First block at which XIP-0001 (EOA lock of EIP-7702 smart accounts,
    /// `docs/specs/xip-0001-eoa-lock.md`) is enforced. JSON `xip0001Block`; absent =
    /// never active. Dev chains and the testnet activate it at genesis (`0`).
    #[serde(
        default,
        with = "opt_u64_flex",
        skip_serializing_if = "Option::is_none"
    )]
    pub xip0001_block: Option<u64>,
    /// Blocks per staking epoch (JSON `epochLength`, default 3600, plan §11.5). Block
    /// `B` is an epoch boundary when the staking-driven validator set is active at `B`
    /// and `B % epochLength == 0` (see [`ChainConfig::is_epoch_boundary`]).
    #[serde(with = "u64_flex", default = "d_epoch_length")]
    pub epoch_length: u64,
    /// First block of the staking-driven validator set (Phase 3, plan §11.4–11.6): from
    /// this block on every block makes the Staking system calls (rewards, liveness,
    /// evidence, `onEpoch` at epoch boundaries) and the consensus validator set is read
    /// from the Staking contract at each epoch boundary. JSON `stakingValidatorsBlock`;
    /// absent = never (the genesis validator set stays in force, Phase 2 behaviour).
    /// Activated on a live network by a coordinated upgrade
    /// (`docs/runbooks/activate-staking-validators.md`).
    #[serde(
        default,
        with = "opt_u64_flex",
        skip_serializing_if = "Option::is_none"
    )]
    pub staking_validators_block: Option<u64>,
    /// Block reward in wei credited to the Staking contract in every block from
    /// [`ChainConfig::staking_validators_block`] on and distributed by
    /// `recordParticipation` (proposer bonus + signers pro rata, plan §11.5). JSON
    /// `blockReward` (decimal or hex string); default 0 = no new issuance. A fork
    /// parameter.
    #[serde(default, with = "u256_flex", skip_serializing_if = "is_zero_u256")]
    pub block_reward: U256,
    /// First block of the encrypted mempool (Phase 5 prototype,
    /// `docs/research/encrypted-mempool.md`): from this block on, transactions to
    /// [`ENCRYPTED_INBOX_ADDRESS`] carrying a threshold-encrypted payload are ordered
    /// blind and decrypted and executed at the start of the next block. JSON
    /// `encryptedMempoolBlock`; absent = never. Needs [`ChainConfig::encrypted_mempool`].
    #[serde(
        default,
        with = "opt_u64_flex",
        skip_serializing_if = "Option::is_none"
    )]
    pub encrypted_mempool_block: Option<u64>,
    /// Committee key of the encrypted mempool (JSON `encryptedMempool`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_mempool: Option<EncryptedMempoolParams>,
}

/// Committee key of the encrypted mempool prototype (genesis `config.encryptedMempool`):
/// `{ "threshold": 3, "publicKey": "0x02…", "verificationKeys": ["0x03…", …] }`. Points are
/// 33-byte compressed secp256k1 points; verification key `i - 1` belongs to the holder
/// of share `i`. Produced by a trusted dealer in the prototype (no DKG yet).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedMempoolParams {
    /// Decryption shares needed to open a payload (`t`).
    pub threshold: u32,
    /// Committee public key `Y` (encrypt to this).
    pub public_key: Bytes,
    /// `Y_i` of every share holder, share index `i` at position `i - 1` (`n` entries).
    pub verification_keys: Vec<Bytes>,
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
            xip0001_block: None,
            epoch_length: DEFAULT_EPOCH_LENGTH,
            staking_validators_block: None,
            block_reward: U256::ZERO,
            encrypted_mempool_block: None,
            encrypted_mempool: None,
        }
    }

    /// Configuration of the local single-node dev chain ([`DEV_CHAIN_ID`]): the
    /// [`ChainConfig::new`] defaults with XIP-0001 active from genesis.
    pub fn dev() -> Self {
        Self {
            xip0001_block: Some(0),
            ..Self::new(DEV_CHAIN_ID)
        }
    }

    /// `true` if the XIP-0001 rules apply to block `number` (`number >= xip0001Block`).
    pub fn xip0001_active(&self, number: u64) -> bool {
        self.xip0001_block.is_some_and(|start| number >= start)
    }

    /// `true` if the Staking system calls run in block `number`
    /// (`number >= stakingValidatorsBlock`).
    pub fn staking_active(&self, number: u64) -> bool {
        self.staking_validators_block
            .is_some_and(|start| number >= start)
    }

    /// `true` if block `number` is an epoch boundary: staking is active at `number`,
    /// `number > 0` and `number % epochLength == 0`. The boundary block calls
    /// `Staking.onEpoch()` and fixes the validator set of the blocks after it.
    pub fn is_epoch_boundary(&self, number: u64) -> bool {
        number > 0
            && self.epoch_length > 0
            && self.staking_active(number)
            && number.is_multiple_of(self.epoch_length)
    }

    /// The epoch-boundary block whose validator set is in force at `height`: the
    /// highest boundary `B < height`. `None` = no boundary yet (or staking never
    /// activates): the genesis validator set applies. Sets change only at boundaries,
    /// so the heights `B + 1 ..= B + epochLength` share one set.
    pub fn validator_set_boundary(&self, height: u64) -> Option<u64> {
        let start = self.staking_validators_block?;
        if self.epoch_length == 0 || height < 2 {
            return None;
        }
        let last = (height - 1) / self.epoch_length * self.epoch_length;
        (last > 0 && last >= start).then_some(last)
    }

    /// The first epoch boundary `>= number` (`None` if staking never activates).
    pub fn next_epoch_boundary(&self, number: u64) -> Option<u64> {
        let start = self.staking_validators_block?;
        if self.epoch_length == 0 {
            return None;
        }
        let from = number.max(start).max(1);
        from.div_ceil(self.epoch_length)
            .checked_mul(self.epoch_length)
    }

    /// `true` if block `number` is in the encrypted-mempool era (`number >=
    /// encryptedMempoolBlock` and a committee key is configured): its envelopes are
    /// decrypted by the next block, and it carries the decryption of its parent's.
    pub fn encrypted_mempool_active(&self, number: u64) -> bool {
        self.encrypted_mempool.is_some()
            && self
                .encrypted_mempool_block
                .is_some_and(|start| number >= start)
    }

    /// Inner gas the envelopes of one block may commit to in total (half the block gas
    /// limit): their decrypted transactions always fit at the start of the next block.
    pub const fn encrypted_gas_budget(&self) -> u64 {
        self.block_gas_limit / 2
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
        // F2: absent = never active.
        assert_eq!(cfg.xip0001_block, None);
        assert!(!cfg.xip0001_active(0) && !cfg.xip0001_active(u64::MAX));
    }

    #[test]
    fn xip0001_activation() {
        for v in ["100", "\"100\"", "\"0x64\""] {
            let json = format!(r#"{{"chainId": 1, "xip0001Block": {v}}}"#);
            let cfg: ChainConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(cfg.xip0001_block, Some(100), "{v}");
            // F1: block 99 inactive, block 100 active.
            assert!(!cfg.xip0001_active(99));
            assert!(cfg.xip0001_active(100));
            assert!(cfg.xip0001_active(101));
        }
        let cfg: ChainConfig =
            serde_json::from_str(r#"{"chainId": 1, "xip0001Block": null}"#).unwrap();
        assert_eq!(cfg.xip0001_block, None);
        // Serialized only when set.
        assert!(!serde_json::to_string(&ChainConfig::new(1))
            .unwrap()
            .contains("xip0001"));
        let dev = serde_json::to_string(&ChainConfig::dev()).unwrap();
        assert!(dev.contains(r#""xip0001Block":0"#), "{dev}");
        assert!(ChainConfig::dev().xip0001_active(0));
    }

    #[test]
    fn staking_fork_fields() {
        let cfg: ChainConfig = serde_json::from_str(
            r#"{"chainId": 1, "epochLength": 20, "stakingValidatorsBlock": "0x1e",
                "blockReward": "1000000000000000000"}"#,
        )
        .unwrap();
        assert_eq!(cfg.epoch_length, 20);
        assert_eq!(cfg.staking_validators_block, Some(30));
        assert_eq!(cfg.block_reward, U256::from(10u64).pow(U256::from(18u64)));
        let json = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<ChainConfig>(&json).unwrap(), cfg);
        // Defaults: 3600-block epochs, never active, no reward; unset fields are not
        // serialized (a stored config of an older binary compares equal).
        let d = ChainConfig::new(1);
        assert_eq!(d.epoch_length, 3600);
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("stakingValidatorsBlock") && !json.contains("blockReward"));
        assert!(!d.staking_active(u64::MAX) && !d.is_epoch_boundary(3600));
        assert_eq!(d.validator_set_boundary(1_000_000), None);
        assert_eq!(d.next_epoch_boundary(5), None);
    }

    #[test]
    fn epoch_math() {
        let cfg = ChainConfig {
            epoch_length: 20,
            staking_validators_block: Some(30),
            ..ChainConfig::new(1)
        };
        assert!(!cfg.staking_active(29) && cfg.staking_active(30));
        // 20 is a multiple of the epoch length but before the activation.
        assert!(!cfg.is_epoch_boundary(20));
        assert!(!cfg.is_epoch_boundary(30) && !cfg.is_epoch_boundary(39));
        assert!(cfg.is_epoch_boundary(40) && cfg.is_epoch_boundary(60));
        // The genesis set governs every height up to and including the first boundary.
        for h in [0, 1, 20, 21, 39, 40] {
            assert_eq!(cfg.validator_set_boundary(h), None, "{h}");
        }
        // The set fixed by block 40 governs 41..=60, then the set of block 60.
        assert_eq!(cfg.validator_set_boundary(41), Some(40));
        assert_eq!(cfg.validator_set_boundary(60), Some(40));
        assert_eq!(cfg.validator_set_boundary(61), Some(60));
        assert_eq!(cfg.next_epoch_boundary(0), Some(40));
        assert_eq!(cfg.next_epoch_boundary(40), Some(40));
        assert_eq!(cfg.next_epoch_boundary(41), Some(60));

        // Activation at genesis: block 0 is never a boundary.
        let cfg = ChainConfig {
            epoch_length: 5,
            staking_validators_block: Some(0),
            ..ChainConfig::new(1)
        };
        assert!(!cfg.is_epoch_boundary(0) && cfg.is_epoch_boundary(5));
        assert_eq!(cfg.validator_set_boundary(5), None);
        assert_eq!(cfg.validator_set_boundary(6), Some(5));
        assert_eq!(cfg.next_epoch_boundary(0), Some(5));
    }

    #[test]
    fn encrypted_mempool_activation() {
        let mut cfg = ChainConfig::new(1);
        assert!(!cfg.encrypted_mempool_active(u64::MAX));
        cfg.encrypted_mempool_block = Some(10);
        // A block without a committee key never activates.
        assert!(!cfg.encrypted_mempool_active(10));
        cfg.encrypted_mempool = Some(EncryptedMempoolParams {
            threshold: 1,
            public_key: Bytes::from(vec![2u8; 33]),
            verification_keys: vec![Bytes::from(vec![3u8; 33])],
        });
        assert!(!cfg.encrypted_mempool_active(9) && cfg.encrypted_mempool_active(10));
        assert_eq!(cfg.encrypted_gas_budget(), cfg.block_gas_limit / 2);
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("encryptedMempoolBlock"));
        assert!(json.contains("verificationKeys"));
        let back: ChainConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
        // Absent by default, and not serialized.
        let plain = serde_json::to_string(&ChainConfig::new(1)).unwrap();
        assert!(!plain.contains("encryptedMempool"));
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

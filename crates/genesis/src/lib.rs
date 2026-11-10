//! Genesis configuration (plan §21). **Owner: genesis agent.**
//!
//! [`Genesis`] parses the `genesis.json` shape of §21.1:
//!
//! ```json
//! { "config": { "chainId": 7117, "blockTimeMs": 2000, "baseFee": { ... } },
//!   "timestamp": "0x0", "gasLimit": "0x1c9c380", "extraData": "0x7844455653",
//!   "alloc": { "0x...": { "balance": "1000", "code": "0x..", "storage": { "0x..": "0x.." }, "nonce": "0x1" } } }
//! ```
//!
//! [`genesis_state`] turns the alloc into a [`StateDiff`]; [`genesis_header`] builds the
//! block-0 header for a given state root; [`genesis_block`] does both.
//!
//! [`Genesis::dev`] is the built-in local dev genesis: the 10 well-known Anvil / Hardhat
//! accounts ([`DEV_ACCOUNTS`]) with 10,000 tXDS each plus the treasury. The same chain is
//! described by `deploy/configs/dev-genesis.json`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use xdevs_primitives::{
    address,
    constants::{BPS_DENOMINATOR, DEFAULT_BLOCK_GAS_LIMIT, MAX_EXTRA_DATA_SIZE},
    serde_helpers::{u256_flex, u64_hex_flex},
    set_compat_header_fields, AccountInfo, Address, Block, Bytes, ChainConfig, Header, B256,
    EMPTY_ROOT_HASH, U256,
};

/// BIP-39 mnemonic of the well-known dev accounts (Anvil / Hardhat default).
/// **Public knowledge: never use these keys outside a local dev chain.**
pub const DEV_MNEMONIC: &str = "test test test test test test test test test test test junk";

/// The 10 well-known dev accounts (`m/44'/60'/0'/0/i` of [`DEV_MNEMONIC`]).
pub const DEV_ACCOUNTS: [Address; 10] = [
    address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
    address!("0x70997970C51812dc3a010C7d01b50e0d17dc79C8"),
    address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
    address!("0x90F79bf6EB2c4F870365E785982E1f101E93b906"),
    address!("0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65"),
    address!("0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"),
    address!("0x976EA74026E726554dB657fA54763abd0C3a0aa9"),
    address!("0x14dC79964da2C08b23698B3D3cc7Ca32193d9955"),
    address!("0x23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f"),
    address!("0xa0Ee7A142d267C1f36714E4a8F75612F20a79720"),
];

/// Private keys of [`DEV_ACCOUNTS`], same order (as printed by `anvil`).
/// **Public knowledge: never use these keys outside a local dev chain.**
pub const DEV_PRIVATE_KEYS: [&str; 10] = [
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
    "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a",
    "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a",
    "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba",
    "0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e",
    "0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356",
    "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
    "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6",
];

/// 1 tXDS in wei ("dev"): 10^18.
pub const TXDS: U256 = U256::from_limbs([0x0de0_b6b3_a764_0000, 0, 0, 0]);

/// Balance of each dev account: 10,000 tXDS (10^22 wei).
pub const DEV_ACCOUNT_BALANCE: U256 = U256::from_limbs([0x19e0_c9ba_b240_0000, 0x21e, 0, 0]);

/// Dev treasury balance: 200,000,000 tXDS (2 * 10^26 wei, the 20% treasury bucket of §20.2).
pub const DEV_TREASURY_BALANCE: U256 = U256::from_limbs([0xb990_19a5_c800_0000, 0xa5_6fa5, 0, 0]);

/// Genesis extra data of the dev chain (`"xDEVS"`).
pub const DEV_EXTRA_DATA: &[u8] = b"xDEVS";

/// Smallest block gas limit accepted (Ethereum's `MIN_GAS_LIMIT`).
pub const MIN_GAS_LIMIT: u64 = 5_000;
use xdevs_state::{InMemoryState, StateDiff};

/// Genesis loading errors.
#[derive(Debug, thiserror::Error)]
pub enum GenesisError {
    /// File could not be read.
    #[error("cannot read genesis file: {0}")]
    Io(#[from] std::io::Error),
    /// JSON is malformed or has the wrong shape.
    #[error("invalid genesis json: {0}")]
    Json(#[from] serde_json::Error),
    /// Semantically invalid genesis (e.g. extraData > 32 bytes).
    #[error("invalid genesis: {0}")]
    Invalid(String),
}

/// One pre-funded / pre-deployed account in `alloc`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenesisAccount {
    /// Balance in wei (decimal or hex string, or integer).
    #[serde(with = "u256_flex", default)]
    pub balance: U256,
    /// Runtime bytecode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Bytes>,
    /// Initial storage (`slot => value`, hex). Zero values are ignored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub storage: BTreeMap<B256, U256>,
    /// Account nonce (default 0).
    #[serde(default, with = "u64_hex_flex", skip_serializing_if = "is_zero")]
    pub nonce: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

const fn d_gas_limit() -> u64 {
    DEFAULT_BLOCK_GAS_LIMIT
}

/// Parsed `genesis.json` (plan §21.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Genesis {
    /// Chain parameters (`config` object). Its `block_gas_limit` is overridden by
    /// [`Genesis::gas_limit`] in [`Genesis::chain_config`].
    pub config: ChainConfig,
    /// Genesis timestamp (seconds).
    #[serde(default, with = "u64_hex_flex")]
    pub timestamp: u64,
    /// Block gas limit.
    #[serde(default = "d_gas_limit", with = "u64_hex_flex")]
    pub gas_limit: u64,
    /// Genesis `extra_data` (≤ 32 bytes).
    #[serde(default)]
    pub extra_data: Bytes,
    /// Initial accounts (deterministic order).
    #[serde(default)]
    pub alloc: BTreeMap<Address, GenesisAccount>,
    /// Phase-2+ validator entries; kept verbatim, unused in Phase 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validators: Vec<serde_json::Value>,
}

impl Genesis {
    /// Parse from a JSON string.
    pub fn from_json_str(s: &str) -> Result<Self, GenesisError> {
        let g: Genesis = serde_json::from_str(s)?;
        g.validate()?;
        Ok(g)
    }

    /// Read and parse a `genesis.json` file.
    pub fn from_file(path: &Path) -> Result<Self, GenesisError> {
        Self::from_json_str(&std::fs::read_to_string(path)?)
    }

    /// Sanity checks (run by the parsers).
    pub fn validate(&self) -> Result<(), GenesisError> {
        let invalid = |m: &str| Err(GenesisError::Invalid(m.into()));
        if self.config.chain_id == 0 {
            return invalid("config.chainId must be > 0");
        }
        if self.extra_data.len() > MAX_EXTRA_DATA_SIZE {
            return invalid("extraData longer than 32 bytes");
        }
        if self.gas_limit < MIN_GAS_LIMIT {
            return invalid("gasLimit must be at least 5000");
        }
        let bf = &self.config.base_fee;
        if bf.elasticity == 0 || bf.max_change_denominator == 0 {
            return invalid("baseFee elasticity/denominator must be > 0");
        }
        if bf.burn_bps > BPS_DENOMINATOR {
            return invalid("baseFee.burnBps must be <= 10000");
        }
        if bf.initial < bf.minimum {
            return invalid("baseFee.initial must be >= baseFee.minimum");
        }
        Ok(())
    }

    /// Effective chain config (`block_gas_limit = gas_limit`).
    pub fn chain_config(&self) -> ChainConfig {
        ChainConfig {
            block_gas_limit: self.gas_limit,
            ..self.config.clone()
        }
    }

    /// Built-in local dev genesis for `chain_id` (normally
    /// [`xdevs_primitives::constants::DEV_CHAIN_ID`]): the 10 [`DEV_ACCOUNTS`] with
    /// [`DEV_ACCOUNT_BALANCE`] (10,000 tXDS) each and the treasury
    /// ([`ChainConfig::treasury`]) with [`DEV_TREASURY_BALANCE`]. Other parameters are the
    /// [`ChainConfig::new`] defaults (30M gas, 1 gwei initial base fee, 80% burn).
    pub fn dev(chain_id: u64) -> Self {
        let mut g = Self::dev_with_accounts(chain_id, &DEV_ACCOUNTS, DEV_ACCOUNT_BALANCE);
        let treasury = g.config.treasury;
        let entry = g.alloc.entry(treasury).or_default();
        entry.balance = entry.balance.saturating_add(DEV_TREASURY_BALANCE);
        g
    }

    /// Dev-style genesis for `chain_id` pre-funding only `accounts` with `balance` wei
    /// each (no treasury allocation). Useful for tests.
    /// XIP-0001 (EOA lock) is active from genesis (`xip0001Block = 0`).
    pub fn dev_with_accounts(chain_id: u64, accounts: &[Address], balance: U256) -> Self {
        let cfg = ChainConfig {
            xip0001_block: Some(0),
            ..ChainConfig::new(chain_id)
        };
        Self {
            gas_limit: cfg.block_gas_limit,
            config: cfg,
            timestamp: 0,
            extra_data: Bytes::from_static(DEV_EXTRA_DATA),
            alloc: accounts
                .iter()
                .map(|a| {
                    (
                        *a,
                        GenesisAccount {
                            balance,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            validators: vec![],
        }
    }

    /// Pretty JSON in the §21.1 shape (round-trips through [`Genesis::from_json_str`]).
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("genesis is always serializable")
    }
}

/// Initial state of the chain as a diff on top of an empty state.
pub fn genesis_state(genesis: &Genesis) -> StateDiff {
    let mut diff = StateDiff::new();
    for (address, acc) in &genesis.alloc {
        let mut info = AccountInfo::new(acc.nonce, acc.balance);
        if let Some(code) = acc.code.as_ref().filter(|c| !c.is_empty()) {
            info.code_hash = diff.insert_code(code.clone());
        }
        diff.set_account(*address, info);
        for (slot, value) in &acc.storage {
            if !value.is_zero() {
                diff.set_storage(*address, *slot, *value);
            }
        }
    }
    diff
}

/// Block-0 header for `genesis` with the given state root.
pub fn genesis_header(genesis: &Genesis, state_root: B256) -> Header {
    let mut header = Header {
        number: 0,
        parent_hash: B256::ZERO,
        beneficiary: Address::ZERO,
        state_root,
        transactions_root: EMPTY_ROOT_HASH,
        receipts_root: EMPTY_ROOT_HASH,
        gas_limit: genesis.gas_limit,
        gas_used: 0,
        timestamp: genesis.timestamp,
        extra_data: genesis.extra_data.clone(),
        mix_hash: B256::ZERO,
        base_fee_per_gas: Some(genesis.config.base_fee.initial),
        ..Default::default()
    };
    set_compat_header_fields(&mut header);
    header
}

/// Genesis block (empty body) and its state diff.
pub fn genesis_block(genesis: &Genesis) -> (Block, StateDiff) {
    let diff = genesis_state(genesis);
    let root = InMemoryState::from_diff(&diff).state_root();
    (Block::new(genesis_header(genesis, root), vec![]), diff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::{
        constants::{DEV_CHAIN_ID, GWEI, TREASURY_ADDRESS},
        KECCAK256_EMPTY,
    };

    const SAMPLE: &str = r#"{
      "config": {
        "chainId": 717171, "name": "xdevs-testnet-1", "hardfork": "xdevs-v1",
        "blockTimeMs": 2000, "epochLength": 3600, "maxValidators": 100,
        "minSelfStake": "10000000000000000000000", "unbondingSeconds": 86400,
        "baseFee": { "initial": "1000000000", "minimum": "100000000", "elasticity": 2, "maxChangeDenominator": 8, "burnBps": 8000 },
        "consensus": { "engine": "malachite", "timeoutProposeMs": 1000 }
      },
      "timestamp": "0x0",
      "gasLimit": "0x1c9c380",
      "extraData": "0x7844455653",
      "alloc": {
        "0x00000000000000000000000000000000000000aa": { "balance": "300000000000000000000000000" },
        "0x0000000000000000000000000000000000001002": { "code": "0x6000", "storage": { "0x0000000000000000000000000000000000000000000000000000000000000001": "0x2a" }, "balance": "0x10", "nonce": "0x1" }
      },
      "validators": [ { "operator": "0x01", "power": 1 } ]
    }"#;

    #[test]
    fn parses_plan_shape() {
        let g = Genesis::from_json_str(SAMPLE).unwrap();
        assert_eq!(g.config.chain_id, 717171);
        assert_eq!(g.gas_limit, 30_000_000);
        assert_eq!(g.chain_config().block_gas_limit, 30_000_000);
        assert_eq!(g.extra_data.as_ref(), b"xDEVS");
        assert_eq!(g.alloc.len(), 2);
        let t = &g.alloc[&TREASURY_ADDRESS];
        assert_eq!(t.nonce, 1);
        assert_eq!(t.balance, U256::from(16u64));
        assert_eq!(t.storage[&B256::with_last_byte(1)], U256::from(42u64));
    }

    #[test]
    fn state_and_header() {
        let g = Genesis::from_json_str(SAMPLE).unwrap();
        let (block, diff) = genesis_block(&g);
        let t = diff.accounts[&TREASURY_ADDRESS].unwrap();
        assert_ne!(t.code_hash, KECCAK256_EMPTY);
        assert_eq!(diff.codes.len(), 1);
        assert_eq!(block.header.number, 0);
        assert_eq!(block.header.base_fee_per_gas, Some(1_000_000_000));
        assert_eq!(
            block.header.state_root,
            InMemoryState::from_diff(&diff).state_root()
        );
        // Deterministic: same input, same hash.
        assert_eq!(genesis_block(&g).0.hash(), block.hash());
    }

    #[test]
    fn json_roundtrip() {
        let g = Genesis::from_json_str(SAMPLE).unwrap();
        let s = serde_json::to_string(&g).unwrap();
        assert_eq!(Genesis::from_json_str(&s).unwrap(), g);
    }

    #[test]
    fn dev_genesis() {
        let g = Genesis::dev(DEV_CHAIN_ID);
        g.validate().unwrap();
        assert_eq!(g.config.chain_id, DEV_CHAIN_ID);
        assert_eq!(g.alloc.len(), 11);
        for a in DEV_ACCOUNTS {
            assert_eq!(g.alloc[&a].balance, DEV_ACCOUNT_BALANCE);
        }
        assert_eq!(DEV_ACCOUNT_BALANCE, U256::from(10_000u64) * TXDS);
        assert_eq!(TXDS, U256::from(10u64).pow(U256::from(18u64)));
        assert_eq!(
            g.alloc[&TREASURY_ADDRESS].balance,
            U256::from(200_000_000u64) * TXDS
        );
        assert_eq!(g.extra_data.as_ref(), b"xDEVS");
        assert_eq!(
            g.config.xip0001_block,
            Some(0),
            "XIP-0001 active from genesis"
        );

        let (block, diff) = genesis_block(&g);
        assert_eq!(diff.accounts.len(), 11);
        let h = &block.header;
        assert_eq!(h.number, 0);
        assert_eq!(h.gas_limit, 30_000_000);
        assert_eq!(h.base_fee_per_gas, Some(GWEI));
        assert_eq!(h.transactions_root, EMPTY_ROOT_HASH);
        assert_eq!(h.receipts_root, EMPTY_ROOT_HASH);
        assert_eq!(h.withdrawals_root, Some(EMPTY_ROOT_HASH));
        assert_eq!(h.parent_beacon_block_root, Some(B256::ZERO));
        assert!(h.requests_hash.is_some());
        assert!(block.body.is_empty());
        assert_eq!(h.state_root, InMemoryState::from_diff(&diff).state_root());
        // Another chain id gives another config but the same alloc.
        let other = Genesis::dev(1337);
        assert_eq!(other.config.chain_id, 1337);
        assert_eq!(other.alloc, g.alloc);
    }

    #[test]
    fn dev_keys_match_dev_accounts() {
        use alloy_signer_local::PrivateKeySigner;
        for (key, addr) in DEV_PRIVATE_KEYS.iter().zip(DEV_ACCOUNTS) {
            let s: PrivateKeySigner = key.parse().unwrap();
            assert_eq!(s.address(), addr);
        }
    }

    #[test]
    fn dev_genesis_file_matches_builtin() {
        let file = include_str!("../../../deploy/configs/dev-genesis.json");
        let parsed = Genesis::from_json_str(file).unwrap();
        assert_eq!(parsed, Genesis::dev(DEV_CHAIN_ID));
        assert_eq!(
            genesis_block(&parsed).0.hash(),
            genesis_block(&Genesis::dev(DEV_CHAIN_ID)).0.hash()
        );
    }

    #[test]
    fn dev_with_accounts_has_no_treasury() {
        let a = Address::repeat_byte(7);
        let g = Genesis::dev_with_accounts(42, &[a], U256::from(1u64));
        assert_eq!(g.config.chain_id, 42);
        assert_eq!(g.alloc.len(), 1);
        assert_eq!(
            genesis_state(&g).accounts[&a].unwrap().balance,
            U256::from(1u64)
        );
    }

    #[test]
    fn balances_hex_decimal_or_integer() {
        let json = |bal: &str| {
            format!(
                r#"{{"config":{{"chainId":1}},"alloc":{{"0x00000000000000000000000000000000000000aa":{{"balance":{bal}}}}}}}"#
            )
        };
        let a = Address::with_last_byte(0xaa);
        for bal in [r#""0x3e8""#, r#""1000""#, "1000", r#""0X3E8""#] {
            let g = Genesis::from_json_str(&json(bal)).unwrap();
            assert_eq!(g.alloc[&a].balance, U256::from(1000u64), "{bal}");
            assert_eq!(g.gas_limit, 30_000_000);
        }
        assert!(Genesis::from_json_str(&json(r#""lots""#)).is_err());
    }

    #[test]
    fn to_json_roundtrip() {
        let g = Genesis::dev(DEV_CHAIN_ID);
        assert_eq!(Genesis::from_json_str(&g.to_json_string()).unwrap(), g);
    }

    #[test]
    fn rejects_invalid_genesis() {
        let base = Genesis::dev(DEV_CHAIN_ID);
        let check = |f: &dyn Fn(&mut Genesis)| {
            let mut g = base.clone();
            f(&mut g);
            assert!(matches!(g.validate(), Err(GenesisError::Invalid(_))));
        };
        check(&|g| g.config.chain_id = 0);
        check(&|g| g.extra_data = Bytes::from(vec![0u8; 33]));
        check(&|g| g.gas_limit = 0);
        check(&|g| g.config.base_fee.elasticity = 0);
        check(&|g| g.config.base_fee.burn_bps = 10_001);
        check(&|g| g.config.base_fee.initial = g.config.base_fee.minimum - 1);
        assert!(Genesis::from_json_str("{").is_err());
        assert!(Genesis::from_json_str(r#"{"alloc":{}}"#).is_err()); // config required
    }

    #[test]
    fn zero_storage_and_empty_code_are_ignored() {
        let json = r#"{"config":{"chainId":1},"alloc":{"0x00000000000000000000000000000000000000bb":{
            "balance":"1","code":"0x","storage":{"0x0000000000000000000000000000000000000000000000000000000000000001":"0x0"}}}}"#;
        let g = Genesis::from_json_str(json).unwrap();
        let diff = genesis_state(&g);
        let acc = diff.accounts[&Address::with_last_byte(0xbb)].unwrap();
        assert_eq!(acc.code_hash, KECCAK256_EMPTY);
        assert!(diff.storage.is_empty());
        assert!(diff.codes.is_empty());
    }
}

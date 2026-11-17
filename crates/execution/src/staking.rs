//! Staking system calls (Phase 3, plan §10.2 and §11.4–11.6,
//! `docs/specs/predeploys-system.md` §1.3) and ABI helpers for the Staking predeploy
//! (`0x…1000`).
//!
//! From [`ChainConfig::staking_validators_block`](xdevs_primitives::ChainConfig) on, every
//! block runs, **before** its transactions, as `msg.sender = systemAddress` (no signature,
//! no gas charge, not counted in `gasUsed`):
//!
//! 1. the block reward (`ChainConfig::block_reward`) is credited to the Staking balance
//!    (direct balance credit, spec §1.4);
//! 2. `recordParticipation(proposer, signedBitmap)`: `proposer` = operator of the block's
//!    proposer, bit `i` = member `i` of `signingSet()` has a signature in the parent's
//!    commit certificate (rewards, liveness counters, downtime jailing);
//! 3. `slash(abi.encode(1, operator, height, ref))` for each double-sign evidence item;
//! 4. at an epoch boundary ([`ChainConfig::is_epoch_boundary`](xdevs_primitives::ChainConfig)),
//!    `onEpoch()`, after which `consensusValidatorSet()` is read: the validator set of the
//!    next epoch ([`crate::ExecutionOutcome::next_validators`]).
//!
//! The node hands over Ed25519 consensus keys ([`StakingInputs`]); they are mapped to
//! operator addresses with `consensusKeyOwner(bytes32)` in the same state, so execution
//! never depends on anything outside the block and its parent state. A system call that
//! reverts makes the block invalid ([`ExecutionError::SystemCall`]): the contract is built
//! so that none of these calls reverts when the node calls them correctly.

use crate::{EvmExecutor, ExecutionError};
use std::collections::{BTreeMap, BTreeSet};
use xdevs_primitives::{
    constants::STAKING_ADDRESS, keccak256, AccountInfo, Address, Bytes, Header, B256, U256,
};
use xdevs_state::InMemoryState;

/// `slash` evidence kind of a double sign (duplicate vote).
pub const EVIDENCE_DOUBLE_SIGN: u64 = 1;
/// Width of the `recordParticipation` signer bitmap (the contract's hard cap of the
/// active set).
pub const MAX_SIGNING_SET: usize = 256;

/// Inputs of the Staking system calls of one block, taken from the block's consensus
/// data and verified by the node before execution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StakingInputs {
    /// Ed25519 consensus key of the block's proposer (`None` = no proposer bonus).
    pub proposer: Option<B256>,
    /// Consensus keys with a precommit in the parent block's commit certificate.
    pub signers: Vec<B256>,
    /// Verified double-sign evidence to slash, in block order.
    pub evidence: Vec<SlashEvidence>,
}

/// One verified double-sign offence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashEvidence {
    /// Consensus key of the offender.
    pub consensus_key: B256,
    /// Height of the offence.
    pub height: u64,
    /// Hash of the evidence object (traceability, `Tombstoned` event `ref`).
    pub reference: B256,
}

/// One member of the Staking contract's consensus set (`consensusValidatorSet()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakingValidator {
    /// Operator (validator id).
    pub operator: Address,
    /// Ed25519 consensus public key.
    pub consensus_key: B256,
    /// Bonded stake in wei at selection time (the voting power source).
    pub stake: U256,
}

/// `IStaking.ValidatorStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ValidatorStatus {
    /// Never registered.
    None = 0,
    /// Registered, not in the active set.
    Candidate = 1,
    /// In the active set.
    Active = 2,
    /// Jailed for downtime.
    Jailed = 3,
    /// Slashed for double signing (permanent).
    Tombstoned = 4,
    /// Withdrew its whole self-stake.
    Exited = 5,
}

impl ValidatorStatus {
    /// The status with ABI value `v`.
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::None,
            1 => Self::Candidate,
            2 => Self::Active,
            3 => Self::Jailed,
            4 => Self::Tombstoned,
            5 => Self::Exited,
            _ => return None,
        })
    }
}

/// The fields of `getValidator(address)` the node and the tests use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatorInfo {
    /// Consensus key.
    pub consensus_key: B256,
    /// Lifecycle status.
    pub status: ValidatorStatus,
    /// Commission in basis points.
    pub commission_bps: u16,
    /// Jail end (unix seconds; `u64::MAX` when tombstoned).
    pub jailed_until: u64,
    /// Bonded stake (wei).
    pub total_stake: U256,
    /// Operator self-stake (wei).
    pub self_stake: U256,
    /// Unbonding pool (wei).
    pub unbonding_stake: U256,
    /// Commission accrued for the operator (wei).
    pub commission_accrued: U256,
}

/// `IStaking.Params` (governance-tunable). [`Default`] = the testnet values of the spec
/// (§1.7) and of `contracts/script/system/SystemGenesis.s.sol`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StakingParams {
    /// Minimum operator self-stake (wei).
    pub min_self_stake: U256,
    /// Smallest `delegate()` amount (wei).
    pub min_delegation: U256,
    /// Unbonding period (seconds).
    pub unbonding_seconds: u64,
    /// Seconds between commission changes.
    pub commission_change_interval: u64,
    /// Blocks per liveness window.
    pub downtime_window: u64,
    /// Jail duration after downtime (seconds).
    pub downtime_jail_seconds: u64,
    /// Active-set size.
    pub max_validators: u16,
    /// Bound on registered validators.
    pub max_candidates: u16,
    /// Commission cap (bps).
    pub max_commission_bps: u16,
    /// Max commission change per interval (bps).
    pub max_commission_change_bps: u16,
    /// Proposer bonus (bps of the block reward).
    pub proposer_bonus_bps: u16,
    /// Double-sign slash (bps of the stake).
    pub double_sign_slash_bps: u16,
    /// Downtime slash (bps of the stake).
    pub downtime_slash_bps: u16,
    /// Minimum signed share of a liveness window (bps).
    pub min_signed_bps: u16,
}

impl Default for StakingParams {
    fn default() -> Self {
        let ether = U256::from(10u64).pow(U256::from(18u64));
        Self {
            min_self_stake: U256::from(10_000u64) * ether,
            min_delegation: ether,
            unbonding_seconds: 86_400,
            commission_change_interval: 86_400,
            downtime_window: 10_000,
            downtime_jail_seconds: 600,
            max_validators: 100,
            max_candidates: 1_000,
            max_commission_bps: 2_000,
            max_commission_change_bps: 100,
            proposer_bonus_bps: 500,
            double_sign_slash_bps: 500,
            downtime_slash_bps: 1,
            min_signed_bps: 5_000,
        }
    }
}

/// A validator registered by `initialize` (genesis).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenesisStake {
    /// Operator address.
    pub operator: Address,
    /// Ed25519 consensus key.
    pub consensus_key: B256,
    /// Commission (bps).
    pub commission_bps: u16,
    /// Self-stake (wei).
    pub stake: U256,
}

/// Minimal Solidity ABI encoding / decoding for the Staking calls (static arguments and
/// dynamic arrays of 32-byte words only).
pub mod abi {
    use super::*;

    /// The 4-byte selector of a function signature, e.g. `"onEpoch()"`.
    pub fn selector(signature: &str) -> [u8; 4] {
        let h = keccak256(signature.as_bytes());
        [h[0], h[1], h[2], h[3]]
    }

    /// Calldata: selector followed by the 32-byte argument words.
    pub fn call(signature: &str, words: &[[u8; 32]]) -> Bytes {
        let mut out = Vec::with_capacity(4 + 32 * words.len());
        out.extend_from_slice(&selector(signature));
        for w in words {
            out.extend_from_slice(w);
        }
        out.into()
    }

    /// An address as an ABI word.
    pub fn address(a: Address) -> [u8; 32] {
        a.into_word().0
    }

    /// A `uint` as an ABI word.
    pub fn uint(v: U256) -> [u8; 32] {
        v.to_be_bytes::<32>()
    }

    /// A `uint64` as an ABI word.
    pub fn uint64(v: u64) -> [u8; 32] {
        uint(U256::from(v))
    }

    fn short(what: &str) -> ExecutionError {
        ExecutionError::SystemCall(format!("Staking returned malformed data ({what})"))
    }

    /// Word `i` of `data`.
    pub fn word(data: &[u8], i: usize) -> Result<[u8; 32], ExecutionError> {
        let start = i.checked_mul(32).ok_or_else(|| short("offset"))?;
        let end = start.checked_add(32).ok_or_else(|| short("offset"))?;
        let w = data.get(start..end).ok_or_else(|| short("short output"))?;
        let mut out = [0u8; 32];
        out.copy_from_slice(w);
        Ok(out)
    }

    /// Word `i` of `data` as a `uint`.
    pub fn uint_at(data: &[u8], i: usize) -> Result<U256, ExecutionError> {
        Ok(U256::from_be_bytes(word(data, i)?))
    }

    /// Word `i` of `data` as an address (upper 12 bytes must be zero).
    pub fn address_at(data: &[u8], i: usize) -> Result<Address, ExecutionError> {
        let w = word(data, i)?;
        if w[..12].iter().any(|b| *b != 0) {
            return Err(short("address"));
        }
        Ok(Address::from_slice(&w[12..]))
    }

    /// Word `i` of `data` as a small integer (`u64`).
    pub fn u64_at(data: &[u8], i: usize) -> Result<u64, ExecutionError> {
        u64::try_from(uint_at(data, i)?).map_err(|_| short("integer"))
    }

    /// The dynamic array of words whose offset is head word `head`.
    pub fn words_array(data: &[u8], head: usize) -> Result<Vec<[u8; 32]>, ExecutionError> {
        let offset = usize::try_from(u64_at(data, head)?).map_err(|_| short("offset"))?;
        if !offset.is_multiple_of(32) {
            return Err(short("unaligned offset"));
        }
        let first = offset / 32;
        let len = usize::try_from(u64_at(data, first)?).map_err(|_| short("length"))?;
        if len > data.len() / 32 {
            return Err(short("array length"));
        }
        (0..len).map(|k| word(data, first + 1 + k)).collect()
    }
}

/// Calldata of the Staking functions the node and the tests call.
pub mod calls {
    use super::{abi, GenesisStake, StakingParams, EVIDENCE_DOUBLE_SIGN};
    use xdevs_primitives::{Address, Bytes, B256, U256};

    /// `recordParticipation(address proposer, uint256 signedBitmap)`.
    pub fn record_participation(proposer: Address, bitmap: U256) -> Bytes {
        abi::call(
            "recordParticipation(address,uint256)",
            &[abi::address(proposer), abi::uint(bitmap)],
        )
    }

    /// `slash(bytes evidence)` with `evidence = abi.encode(1, validator, height, ref)`.
    pub fn slash(validator: Address, height: u64, reference: B256) -> Bytes {
        abi::call(
            "slash(bytes)",
            &[
                abi::uint64(0x20),
                abi::uint64(128),
                abi::uint64(EVIDENCE_DOUBLE_SIGN),
                abi::address(validator),
                abi::uint64(height),
                reference.0,
            ],
        )
    }

    /// `onEpoch()`.
    pub fn on_epoch() -> Bytes {
        abi::call("onEpoch()", &[])
    }

    /// `signingSet()`.
    pub fn signing_set() -> Bytes {
        abi::call("signingSet()", &[])
    }

    /// `consensusValidatorSet()`.
    pub fn consensus_validator_set() -> Bytes {
        abi::call("consensusValidatorSet()", &[])
    }

    /// `consensusKeyOwner(bytes32)`.
    pub fn consensus_key_owner(key: B256) -> Bytes {
        abi::call("consensusKeyOwner(bytes32)", &[key.0])
    }

    /// `register(bytes32 consensusKey, uint16 commissionBps)` (send the self-stake as
    /// value).
    pub fn register(key: B256, commission_bps: u16) -> Bytes {
        abi::call(
            "register(bytes32,uint16)",
            &[key.0, abi::uint64(commission_bps.into())],
        )
    }

    /// `unjail()`.
    pub fn unjail() -> Bytes {
        abi::call("unjail()", &[])
    }

    /// `getValidator(address)`.
    pub fn get_validator(operator: Address) -> Bytes {
        abi::call("getValidator(address)", &[abi::address(operator)])
    }

    /// `pendingRewards(address validator, address delegator)`.
    pub fn pending_rewards(validator: Address, delegator: Address) -> Bytes {
        abi::call(
            "pendingRewards(address,address)",
            &[abi::address(validator), abi::address(delegator)],
        )
    }

    /// `currentEpoch()`.
    pub fn current_epoch() -> Bytes {
        abi::call("currentEpoch()", &[])
    }

    /// The genesis `initialize(address governance, Params params, GenesisValidator[] vals)`
    /// (run once by the genesis builder as `SYSTEM_ADDRESS`).
    pub fn initialize(governance: Address, p: &StakingParams, vals: &[GenesisStake]) -> Bytes {
        let mut words = vec![
            abi::address(governance),
            abi::uint(p.min_self_stake),
            abi::uint(p.min_delegation),
            abi::uint64(p.unbonding_seconds),
            abi::uint64(p.commission_change_interval),
            abi::uint64(p.downtime_window),
            abi::uint64(p.downtime_jail_seconds),
            abi::uint64(p.max_validators.into()),
            abi::uint64(p.max_candidates.into()),
            abi::uint64(p.max_commission_bps.into()),
            abi::uint64(p.max_commission_change_bps.into()),
            abi::uint64(p.proposer_bonus_bps.into()),
            abi::uint64(p.double_sign_slash_bps.into()),
            abi::uint64(p.downtime_slash_bps.into()),
            abi::uint64(p.min_signed_bps.into()),
        ];
        // Offset of the dynamic array = size of the head (this word included).
        let head = (words.len() as u64 + 1) * 32;
        words.push(abi::uint64(head));
        words.push(abi::uint64(vals.len() as u64));
        for v in vals {
            words.push(abi::address(v.operator));
            words.push(v.consensus_key.0);
            words.push(abi::uint64(v.commission_bps.into()));
            words.push(abi::uint(v.stake));
        }
        abi::call(
            "initialize(address,(uint256,uint256,uint64,uint64,uint64,uint64,uint16,uint16,\
             uint16,uint16,uint16,uint16,uint16,uint16),(address,bytes32,uint16,uint256)[])",
            &words,
        )
    }
}

/// Decode `consensusValidatorSet()`: `(address[] operators, bytes32[] keys, uint256[] powers)`.
pub fn decode_validator_set(output: &[u8]) -> Result<Vec<StakingValidator>, ExecutionError> {
    let ops = abi::words_array(output, 0)?;
    let keys = abi::words_array(output, 1)?;
    let powers = abi::words_array(output, 2)?;
    if ops.len() != keys.len() || ops.len() != powers.len() {
        return Err(ExecutionError::SystemCall(
            "consensusValidatorSet arrays differ in length".into(),
        ));
    }
    ops.iter()
        .zip(&keys)
        .zip(&powers)
        .map(|((op, key), power)| {
            Ok(StakingValidator {
                operator: abi::address_at(op, 0)?,
                consensus_key: B256::from(*key),
                stake: U256::from_be_bytes(*power),
            })
        })
        .collect()
}

/// Decode the operators of `signingSet()` / `activeValidators()`
/// (`(address[] operators, uint256[] powers)`).
pub fn decode_operators(output: &[u8]) -> Result<Vec<Address>, ExecutionError> {
    abi::words_array(output, 0)?
        .iter()
        .map(|w| abi::address_at(w, 0))
        .collect()
}

/// Decode `getValidator(address)` (a static `ValidatorInfo` tuple of 11 words).
pub fn decode_validator_info(output: &[u8]) -> Result<ValidatorInfo, ExecutionError> {
    let status = abi::u64_at(output, 1)?;
    let status = u8::try_from(status)
        .ok()
        .and_then(ValidatorStatus::from_u8)
        .ok_or_else(|| ExecutionError::SystemCall(format!("unknown validator status {status}")))?;
    Ok(ValidatorInfo {
        consensus_key: B256::from(abi::word(output, 0)?),
        status,
        commission_bps: u16::try_from(abi::u64_at(output, 2)?)
            .map_err(|_| ExecutionError::SystemCall("commission".into()))?,
        jailed_until: abi::u64_at(output, 3)?,
        total_stake: abi::uint_at(output, 5)?,
        self_stake: abi::uint_at(output, 7)?,
        unbonding_stake: abi::uint_at(output, 8)?,
        commission_accrued: abi::uint_at(output, 10)?,
    })
}

/// The `recordParticipation` bitmap: bit `i` set iff `signing_set[i]` is in `signed`
/// (members beyond [`MAX_SIGNING_SET`] cannot be represented and count as absent).
pub fn signed_bitmap(signing_set: &[Address], signed: &BTreeSet<Address>) -> U256 {
    let mut bitmap = U256::ZERO;
    for (i, op) in signing_set.iter().enumerate().take(MAX_SIGNING_SET) {
        if signed.contains(op) {
            bitmap |= U256::from(1u64) << i;
        }
    }
    bitmap
}

/// Storage and balance of an initialised Staking predeploy for a genesis: runs
/// `initialize(governance, params, validators)` as the system address on an otherwise
/// empty state holding only `code` at `0x…1000` with balance = Σ stakes (the procedure of
/// the TypeScript genesis builder, `docs/specs/predeploys-system.md` §0). Returns the
/// balance and the non-zero storage slots to put into `alloc`.
pub fn genesis_storage(
    executor: &EvmExecutor,
    code: Bytes,
    governance: Address,
    params: &StakingParams,
    validators: &[GenesisStake],
) -> Result<(U256, BTreeMap<B256, U256>), ExecutionError> {
    let balance = validators
        .iter()
        .fold(U256::ZERO, |acc, v| acc.saturating_add(v.stake));
    let mut state = InMemoryState::new();
    let code_hash = state.insert_code(code);
    let mut info = AccountInfo::new(0, balance);
    info.code_hash = code_hash;
    state.insert_account(STAKING_ADDRESS, info);
    let header = Header {
        gas_limit: executor.chain_config().block_gas_limit,
        ..Default::default()
    };
    let (_, diff) = executor.system_call_at(
        &state,
        &header,
        STAKING_ADDRESS,
        calls::initialize(governance, params, validators),
    )?;
    let slots = diff
        .storage
        .get(&STAKING_ADDRESS)
        .map(|s| {
            s.slots
                .iter()
                .filter(|(_, v)| !v.is_zero())
                .map(|(k, v)| (*k, *v))
                .collect()
        })
        .unwrap_or_default();
    Ok((balance, slots))
}

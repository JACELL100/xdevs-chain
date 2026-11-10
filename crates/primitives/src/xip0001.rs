//! XIP-0001: EOA lock of EIP-7702 smart accounts (`docs/specs/xip-0001-eoa-lock.md`).
//!
//! An EOA delegated to the xDEVS Smart Account (code `0xef0100 ‖ 0x…1005`) can write a
//! lock word into [`LOCK_SLOT`]. While it is non-zero the node restricts what the EOA key
//! can do at the protocol level:
//!
//! * **(a)** `SEALED` (word 2): no transaction from the address is valid. `LOCKED` (word 1
//!   or any other non-zero value): only value-0 calls to itself are valid.
//! * **(b)** EIP-7702 authorizations whose authority is locked or sealed are skipped.
//! * **(c)** `LOCKED` senders: at most one tx per block, gas limit ≤ 2,000,000, max fee
//!   per gas ≤ 2 × base fee.
//!
//! Everything here is pure: the callers (block executor, block builder, mempool, RPC)
//! read the account and the slot from the state *before* the transaction (rule d) and
//! use [`eoa_lock_status`] + [`check_tx`], so all of them apply the exact same rules.

use crate::{address, b256, Address, RecoveredTx, Transaction, TxKind, B256, U256};
use serde::{Deserialize, Serialize};

/// `ACCOUNT_IMPL`: the xDEVS Smart Account implementation (`Predeploys.SMART_ACCOUNT_IMPL`).
pub const ACCOUNT_IMPL: Address = address!("0x0000000000000000000000000000000000001005");

/// `DESIGNATOR`: the 23-byte EIP-7702 delegation code `0xef0100 ‖ ACCOUNT_IMPL`.
pub const DESIGNATOR: [u8; 23] = {
    let mut code = [0u8; 23];
    code[0] = 0xef;
    code[1] = 0x01;
    code[2] = 0x00;
    let impl_bytes = ACCOUNT_IMPL.0 .0;
    let mut i = 0;
    while i < 20 {
        code[3 + i] = impl_bytes[i];
        i += 1;
    }
    code
};

/// `DESIGNATOR_HASH = keccak256(DESIGNATOR)` (`AccountConstants.EIP7702_DESIGNATOR_HASH`).
pub const DESIGNATOR_HASH: B256 =
    b256!("0x4879b5ba5d3e15615c1dc61bbc61a36231da1b95582299c433c2edbee1ad7333");

/// `LOCK_SLOT = keccak256("xdevs.account.eoaLock") - 1` (`AccountConstants.EOA_LOCK_SLOT`).
pub const LOCK_SLOT: B256 =
    b256!("0xfd81d7eef3094fecca95e5e53fe72709c7315b39f65e414992ead9c855e841ce");

/// Lock word value `LOCKED`.
pub const LOCKED: u64 = 1;
/// Lock word value `SEALED`.
pub const SEALED: u64 = 2;
/// `XIP0001_MAX_GAS`: gas limit cap of a `LOCKED` sender's transaction (rule c2).
pub const MAX_GAS: u64 = 2_000_000;
/// `XIP0001_FEE_MULTIPLIER`: a `LOCKED` sender's max fee is capped at this × base fee (c3).
pub const FEE_MULTIPLIER: u128 = 2;
/// `XIP0001_MAX_TXS_PER_BLOCK`: transactions per block of a `LOCKED` sender (c1).
pub const MAX_TXS_PER_BLOCK: usize = 1;
/// Mempool: max pooled transactions (pending + queued) of a `LOCKED` sender (§5 e).
pub const MAX_POOL_TXS_PER_LOCKED_SENDER: usize = 4;

/// Lock status of an address (`status(A, S)` of XIP-0001 §3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EoaLockStatus {
    /// Not delegated to `ACCOUNT_IMPL`, or lock word 0: no restriction.
    #[default]
    Unlocked,
    /// Delegated and word 1 (or any non-zero value other than 2): value-0 self-calls only.
    Locked,
    /// Delegated and word 2: no transaction at all.
    Sealed,
}

impl EoaLockStatus {
    /// Classify from the account's code hash (`None` = account does not exist) and the
    /// lock word. `delegated` ⇔ `code_hash == DESIGNATOR_HASH` (byte-exact designator).
    pub fn classify(code_hash: Option<B256>, word: U256) -> Self {
        if code_hash != Some(DESIGNATOR_HASH) || word.is_zero() {
            Self::Unlocked
        } else if word == U256::from(SEALED) {
            Self::Sealed
        } else {
            Self::Locked
        }
    }

    /// Classify from the raw account code (byte-exact comparison with [`DESIGNATOR`]).
    pub fn from_code(code: &[u8], word: U256) -> Self {
        let hash = (code == DESIGNATOR.as_slice()).then_some(DESIGNATOR_HASH);
        Self::classify(hash, word)
    }

    /// `true` for `Locked` and `Sealed` ("locked sender" / "locked authority").
    pub const fn is_restricted(self) -> bool {
        !matches!(self, Self::Unlocked)
    }

    /// `"unlocked"`, `"locked"` or `"sealed"` (the `xdevs_getEoaLock` `status` field).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unlocked => "unlocked",
            Self::Locked => "locked",
            Self::Sealed => "sealed",
        }
    }
}

/// The one shared way to read an account's lock status from any state.
///
/// `code_hash` is the account's code hash (`None` if the account does not exist);
/// `read_word` reads `storage[account][LOCK_SLOT]` and is only called when the account
/// is delegated to [`ACCOUNT_IMPL`] (one code-hash compare per tx, one slot read only for
/// xDEVS 7702 accounts — §9 "Cost").
pub fn eoa_lock_status<E>(
    code_hash: Option<B256>,
    read_word: impl FnOnce() -> Result<U256, E>,
) -> Result<EoaLockStatus, E> {
    if code_hash != Some(DESIGNATOR_HASH) {
        return Ok(EoaLockStatus::Unlocked);
    }
    Ok(EoaLockStatus::classify(code_hash, read_word()?))
}

/// Which XIP-0001 rule a transaction breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xip0001Violation {
    /// (a.1) the sender is `SEALED`.
    Sealed,
    /// (a.2) `LOCKED` sender, but the tx is not a call to the sender itself.
    NotSelfCall,
    /// (a.2) `LOCKED` sender calling itself with a non-zero value.
    ValueNotZero,
    /// (c1) `LOCKED` sender already has an earlier transaction in this block.
    OnePerBlock,
    /// (c2) `LOCKED` sender, gas limit above [`MAX_GAS`].
    GasLimit {
        /// The transaction's gas limit.
        gas_limit: u64,
    },
    /// (c3) `LOCKED` sender, max fee per gas above `FEE_MULTIPLIER × base fee`.
    FeeCap {
        /// The transaction's max fee per gas (gas price for legacy / EIP-2930).
        max_fee: u128,
        /// The allowed maximum (`2 × base fee`).
        cap: u128,
    },
}

impl Xip0001Violation {
    /// Machine-readable reason (JSON-RPC `error.data.reason`, spec §5 f).
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Sealed => "XIP0001_SEALED",
            Self::NotSelfCall => "XIP0001_NOT_SELF_CALL",
            Self::ValueNotZero => "XIP0001_VALUE_NOT_ZERO",
            Self::OnePerBlock => "XIP0001_ONE_PER_BLOCK",
            Self::GasLimit { .. } => "XIP0001_GAS_LIMIT",
            Self::FeeCap { .. } => "XIP0001_FEE_CAP",
        }
    }

    /// `true` for violations that may disappear on their own in a later block without
    /// any state change of the sender (c1: next block; c3: the base fee moved). The
    /// mempool keeps such txs and the block builder skips them without dropping them.
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::OnePerBlock | Self::FeeCap { .. })
    }
}

/// A transaction rejected by XIP-0001.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub struct Xip0001Error {
    /// The transaction sender.
    pub sender: Address,
    /// The broken rule.
    pub violation: Xip0001Violation,
}

impl core::fmt::Display for Xip0001Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.sender;
        match self.violation {
            Xip0001Violation::Sealed => write!(
                f,
                "xip-0001: sender {s} is EOA-sealed: no transaction from this address is valid"
            ),
            Xip0001Violation::NotSelfCall => write!(
                f,
                "xip-0001: sender {s} is EOA-locked: only value-0 calls to itself are allowed"
            ),
            Xip0001Violation::ValueNotZero => write!(
                f,
                "xip-0001: sender {s} is EOA-locked: only value-0 calls to itself are allowed \
                 (value must be 0)"
            ),
            Xip0001Violation::OnePerBlock => write!(
                f,
                "xip-0001: sender {s} is EOA-locked: at most {MAX_TXS_PER_BLOCK} transaction \
                 per block"
            ),
            Xip0001Violation::GasLimit { gas_limit } => write!(
                f,
                "xip-0001: sender {s} is EOA-locked: gas limit {gas_limit} above {MAX_GAS}"
            ),
            Xip0001Violation::FeeCap { max_fee, cap } => write!(
                f,
                "xip-0001: sender {s} is EOA-locked: max fee per gas {max_fee} above \
                 {FEE_MULTIPLIER} x base fee ({cap})"
            ),
        }
    }
}

/// The transaction fields XIP-0001 looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxShape {
    /// Sender.
    pub sender: Address,
    /// Call target or create.
    pub to: TxKind,
    /// Value.
    pub value: U256,
    /// Gas limit.
    pub gas_limit: u64,
    /// `max_fee_per_gas` (EIP-1559 / 7702) or `gas_price` (legacy / EIP-2930).
    pub max_fee_per_gas: u128,
}

impl From<&RecoveredTx> for TxShape {
    fn from(tx: &RecoveredTx) -> Self {
        Self {
            sender: tx.sender,
            to: tx.kind(),
            value: tx.value(),
            gas_limit: tx.gas_limit(),
            max_fee_per_gas: tx.max_fee_per_gas(),
        }
    }
}

/// Rule (a) only: the shape a locked sender may send (no per-block / fee limits).
/// Used where no block context exists (`eth_estimateGas`, pool re-checks).
pub fn check_shape(status: EoaLockStatus, tx: &TxShape) -> Result<(), Xip0001Error> {
    let err = |violation| {
        Err(Xip0001Error {
            sender: tx.sender,
            violation,
        })
    };
    match status {
        EoaLockStatus::Unlocked => Ok(()),
        EoaLockStatus::Sealed => err(Xip0001Violation::Sealed),
        EoaLockStatus::Locked => {
            if tx.to != TxKind::Call(tx.sender) {
                err(Xip0001Violation::NotSelfCall)
            } else if !tx.value.is_zero() {
                err(Xip0001Violation::ValueNotZero)
            } else {
                Ok(())
            }
        }
    }
}

/// Rules (a) and (c) for a transaction whose sender has `status` at `S₀(tx)`, in a block
/// with `base_fee`. `sender_already_in_block` = an earlier transaction of the block has the
/// same sender (c1; the mempool passes `false`: "first tx of the next block").
///
/// Checks run in the order sealed → self-call → value → c1 → c2 → c3.
pub fn check_tx(
    status: EoaLockStatus,
    tx: &TxShape,
    base_fee: u64,
    sender_already_in_block: bool,
) -> Result<(), Xip0001Error> {
    check_shape(status, tx)?;
    if status != EoaLockStatus::Locked {
        return Ok(());
    }
    let err = |violation| {
        Err(Xip0001Error {
            sender: tx.sender,
            violation,
        })
    };
    if sender_already_in_block {
        return err(Xip0001Violation::OnePerBlock);
    }
    if tx.gas_limit > MAX_GAS {
        return err(Xip0001Violation::GasLimit {
            gas_limit: tx.gas_limit,
        });
    }
    let cap = FEE_MULTIPLIER.saturating_mul(u128::from(base_fee));
    if tx.max_fee_per_gas > cap {
        return err(Xip0001Violation::FeeCap {
            max_fee: tx.max_fee_per_gas,
            cap,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{constants::GWEI, keccak256};

    const A: Address = Address::repeat_byte(0xa1);
    const F: Address = Address::repeat_byte(0xf1);
    const B: u64 = GWEI;

    fn shape(to: TxKind, value: u64, gas_limit: u64, max_fee: u128) -> TxShape {
        TxShape {
            sender: A,
            to,
            value: U256::from(value),
            gas_limit,
            max_fee_per_gas: max_fee,
        }
    }

    /// V2 shape: `to=A, value=0, gas=300000, maxFee=2 gwei`.
    fn v2() -> TxShape {
        shape(TxKind::Call(A), 0, 300_000, 2 * u128::from(GWEI))
    }

    fn reason(status: EoaLockStatus, tx: &TxShape) -> Option<&'static str> {
        check_tx(status, tx, B, false)
            .err()
            .map(|e| e.violation.reason())
    }

    fn word(v: u64) -> U256 {
        U256::from(v)
    }

    #[test]
    fn constants_match_the_spec_formulas() {
        assert_eq!(
            LOCK_SLOT,
            B256::from(U256::from_be_bytes(keccak256("xdevs.account.eoaLock").0) - U256::from(1))
        );
        assert_eq!(keccak256(DESIGNATOR), DESIGNATOR_HASH);
        assert_eq!(
            DESIGNATOR.as_slice(),
            alloy_primitives::hex!("ef01000000000000000000000000000000000000001005").as_slice()
        );
    }

    #[test]
    fn classification() {
        use EoaLockStatus::*;
        let d = Some(DESIGNATOR_HASH);
        assert_eq!(EoaLockStatus::classify(d, word(0)), Unlocked);
        assert_eq!(EoaLockStatus::classify(d, word(1)), Locked);
        assert_eq!(EoaLockStatus::classify(d, word(2)), Sealed);
        // V14: garbage counts as LOCKED, never SEALED.
        assert_eq!(EoaLockStatus::classify(d, word(0xdead)), Locked);
        assert_eq!(EoaLockStatus::classify(d, U256::MAX), Locked);
        // V10: delegated to another implementation.
        let mut other = DESIGNATOR;
        other[22] = 0x06;
        assert_eq!(EoaLockStatus::from_code(&other, word(1)), Unlocked);
        assert_eq!(
            EoaLockStatus::classify(Some(keccak256(other)), word(2)),
            Unlocked
        );
        // V11: a real contract, an EOA without code, a missing account.
        assert_eq!(
            EoaLockStatus::from_code(&[0x60, 0x00, 0x00], word(1)),
            Unlocked
        );
        assert_eq!(EoaLockStatus::from_code(&[], word(1)), Unlocked);
        assert_eq!(EoaLockStatus::classify(None, word(2)), Unlocked);
        // Byte-exact: a longer code starting with the designator is not delegated.
        let mut longer = DESIGNATOR.to_vec();
        longer.push(0);
        assert_eq!(EoaLockStatus::from_code(&longer, word(1)), Unlocked);
        assert_eq!(EoaLockStatus::from_code(&DESIGNATOR, word(1)), Locked);

        assert_eq!(Locked.as_str(), "locked");
        assert_eq!(serde_json::to_string(&Sealed).unwrap(), "\"sealed\"");
        assert!(Locked.is_restricted() && Sealed.is_restricted() && !Unlocked.is_restricted());
    }

    #[test]
    fn slot_is_read_only_for_delegated_accounts() {
        let mut reads = 0;
        let st = eoa_lock_status::<()>(None, || {
            reads += 1;
            Ok(word(1))
        });
        assert_eq!(st, Ok(EoaLockStatus::Unlocked));
        assert_eq!(reads, 0);
        let st = eoa_lock_status::<()>(Some(DESIGNATOR_HASH), || {
            reads += 1;
            Ok(word(2))
        });
        assert_eq!(st, Ok(EoaLockStatus::Sealed));
        assert_eq!(reads, 1);
        assert_eq!(
            eoa_lock_status(Some(DESIGNATOR_HASH), || Err("db")),
            Err("db")
        );
    }

    /// Spec §10 "Transaction validity (rule a / c), single transaction in the block".
    #[test]
    fn spec_vectors_tx_validity() {
        use EoaLockStatus::*;
        let gwei = u128::from(GWEI);
        // V1: unlocked, plain transfer at a huge fee.
        let v1 = shape(TxKind::Call(F), 10u64.pow(18), 21_000, 100 * gwei);
        assert_eq!(reason(Unlocked, &v1), None);
        // V2: locked self-call.
        assert_eq!(reason(Locked, &v2()), None);
        // V3: value 1 wei to itself.
        let v3 = TxShape {
            value: U256::from(1),
            ..v2()
        };
        assert_eq!(reason(Locked, &v3), Some("XIP0001_VALUE_NOT_ZERO"));
        // V4: value 0 to another address.
        let v4 = TxShape {
            to: TxKind::Call(F),
            ..v2()
        };
        assert_eq!(reason(Locked, &v4), Some("XIP0001_NOT_SELF_CALL"));
        // V5: contract creation.
        let v5 = TxShape {
            to: TxKind::Create,
            ..v2()
        };
        assert_eq!(reason(Locked, &v5), Some("XIP0001_NOT_SELF_CALL"));
        // Both wrong target and value: the target is reported (M3).
        let both = TxShape {
            to: TxKind::Call(F),
            value: U256::from(1),
            ..v2()
        };
        assert_eq!(reason(Locked, &both), Some("XIP0001_NOT_SELF_CALL"));
        // V7a / V7b: gas cap.
        let v7a = TxShape {
            gas_limit: 2_000_000,
            ..v2()
        };
        assert_eq!(reason(Locked, &v7a), None);
        let v7b = TxShape {
            gas_limit: 2_000_001,
            ..v2()
        };
        assert_eq!(reason(Locked, &v7b), Some("XIP0001_GAS_LIMIT"));
        // V8a / V8b: fee cap = 2 x base.
        let v8a = TxShape {
            max_fee_per_gas: 2_000_000_000,
            ..v2()
        };
        assert_eq!(reason(Locked, &v8a), None);
        let v8b = TxShape {
            max_fee_per_gas: 2_000_000_001,
            ..v2()
        };
        assert_eq!(reason(Locked, &v8b), Some("XIP0001_FEE_CAP"));
        // V8c: legacy gas price is the max fee: 2 gwei valid, 3 gwei invalid.
        let legacy = |gp| TxShape {
            max_fee_per_gas: gp,
            ..v2()
        };
        assert_eq!(reason(Locked, &legacy(2 * gwei)), None);
        assert_eq!(reason(Locked, &legacy(3 * gwei)), Some("XIP0001_FEE_CAP"));
        // V13: sealed rejects even the V2 shape.
        assert_eq!(reason(Sealed, &v2()), Some("XIP0001_SEALED"));
        assert_eq!(reason(Sealed, &v1), Some("XIP0001_SEALED"));
        // V14: garbage word = LOCKED: V2 valid, V4 invalid.
        let garbage = EoaLockStatus::classify(Some(DESIGNATOR_HASH), word(0xdead));
        assert_eq!(reason(garbage, &v2()), None);
        assert_eq!(reason(garbage, &v4), Some("XIP0001_NOT_SELF_CALL"));
        // V10: word 1 but delegated elsewhere: unlocked, transfer valid.
        let v10 = EoaLockStatus::from_code(
            &alloy_primitives::hex!("ef01000000000000000000000000000000000000001006"),
            word(1),
        );
        assert_eq!(reason(v10, &v1), None);
    }

    #[test]
    fn one_per_block_and_messages() {
        let e = check_tx(EoaLockStatus::Locked, &v2(), B, true).unwrap_err();
        assert_eq!(e.violation, Xip0001Violation::OnePerBlock);
        assert!(e.violation.is_transient());
        // Unlocked senders are never limited, whatever the position.
        assert!(check_tx(EoaLockStatus::Unlocked, &v2(), B, true).is_ok());
        // Sealed is reported before c1.
        let e = check_tx(EoaLockStatus::Sealed, &v2(), B, true).unwrap_err();
        assert_eq!(e.violation, Xip0001Violation::Sealed);
        assert!(!e.violation.is_transient());

        let msg = check_tx(
            EoaLockStatus::Locked,
            &TxShape {
                to: TxKind::Call(F),
                ..v2()
            },
            B,
            false,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            msg,
            format!("xip-0001: sender {A} is EOA-locked: only value-0 calls to itself are allowed")
        );
        // check_shape ignores c1-c3.
        let heavy = TxShape {
            gas_limit: 30_000_000,
            max_fee_per_gas: u128::MAX,
            ..v2()
        };
        assert!(check_shape(EoaLockStatus::Locked, &heavy).is_ok());
        assert!(check_tx(EoaLockStatus::Locked, &heavy, B, false).is_err());
    }
}

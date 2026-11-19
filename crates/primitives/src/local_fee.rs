//! Local fee markets (Phase 5, plan §15.2, `docs/research/local-fee-markets.md`).
//!
//! A spike of demand for one contract raises the price of transactions **to that
//! contract** instead of the global EIP-1559 base fee that everyone pays:
//!
//! * The protocol tracks the gas used per **target** (top-level `to` of a transaction;
//!   contract creations have no target) in every block.
//! * A target that used more than [`LOCAL_FEE_TARGET_BPS`] (25%) of the block gas limit
//!   gets a **local base fee** that rises like EIP-1559 (up to +37.5% per block at a
//!   full block, starting at [`LOCAL_FEE_MIN`]) and decays by 1/8 per block below that
//!   share (also in blocks where it is not used at all), dropping to zero below
//!   [`LOCAL_FEE_MIN`]. It is capped at [`LOCAL_FEE_MAX`].
//! * A transaction to a target pays `base_fee + local_fee(to)` as its base fee: it must
//!   have `max_fee_per_gas >= base_fee + local_fee`, its tip is `min(max_priority,
//!   max_fee - base_fee - local_fee)` and the whole base part (global + local) is split
//!   burn / treasury like the global base fee (plan §10.2). Everyone else pays only the
//!   global base fee, which is unchanged.
//!
//! State: the table lives in the storage of [`LOCAL_FEE_MARKET_ADDRESS`] (an account
//! with nonce 1 and no code, created at the first update), Solidity layout
//! `mapping(address => uint256)` at slot 0, value `fee << 64 | last_block`: the local
//! fee that applies in block `last_block + 1`. Later blocks without gas to the target
//! decay it lazily ([`fee_at`]). Only targets with gas in a block are rewritten at the
//! end of that block (after its transactions), so the fees of a block are fixed by its
//! parent state: execution order inside the block cannot change them.

use crate::{address, keccak256, Address, B256, U256};

/// Account whose storage holds the local fee table (no code).
pub const LOCAL_FEE_MARKET_ADDRESS: Address =
    address!("0x0000000000000000000000000000000000001101");

/// Share of the block gas limit a target may use without a local fee (25%).
pub const LOCAL_FEE_TARGET_BPS: u64 = 2_500;
/// Max change per block is `1 / LOCAL_FEE_CHANGE_DENOMINATOR` of the fee per target of
/// excess (like EIP-1559's 1/8).
pub const LOCAL_FEE_CHANGE_DENOMINATOR: u64 = 8;
/// First local fee of a newly congested target, and the level below which a decaying
/// fee is dropped (0.1 gwei, the global base fee floor).
pub const LOCAL_FEE_MIN: u64 = 100_000_000;
/// Cap of a local fee (10,000 gwei).
pub const LOCAL_FEE_MAX: u64 = 10_000_000_000_000;
/// After this many idle blocks a local fee is zero whatever its level (bounds [`decay`]).
pub const LOCAL_FEE_MAX_DECAY_BLOCKS: u64 = 256;

/// Storage slot of `target` in the table (`keccak256(abi.encode(target, 0))`).
pub fn slot(target: Address) -> B256 {
    let mut buf = [0u8; 64];
    buf[12..32].copy_from_slice(target.as_slice());
    keccak256(buf)
}

/// Storage word for `fee` (applies in block `last_block + 1`).
pub fn pack(fee: u64, last_block: u64) -> U256 {
    if fee == 0 {
        return U256::ZERO;
    }
    (U256::from(fee) << 64) | U256::from(last_block)
}

/// `(fee, last_block)` of a storage word (saturating on out-of-range garbage).
pub fn unpack(word: U256) -> (u64, u64) {
    let fee = word >> 64;
    let fee = u64::try_from(fee)
        .unwrap_or(LOCAL_FEE_MAX)
        .min(LOCAL_FEE_MAX);
    let last = u64::try_from(word & U256::from(u64::MAX)).unwrap_or(0);
    (fee, last)
}

/// The local fee after one block in which the target used `gas_used` of `gas_limit`.
pub fn next_local_fee(current: u64, gas_used: u64, gas_limit: u64) -> u64 {
    let target = u128::from(gas_limit) * u128::from(LOCAL_FEE_TARGET_BPS) / 10_000;
    if target == 0 {
        return 0;
    }
    let cur = u128::from(current.min(LOCAL_FEE_MAX));
    let used = u128::from(gas_used);
    let denom = u128::from(LOCAL_FEE_CHANGE_DENOMINATOR);
    if used > target {
        if cur == 0 {
            return LOCAL_FEE_MIN;
        }
        let delta = (cur * (used - target) / target / denom).max(1);
        u64::try_from(cur + delta)
            .unwrap_or(LOCAL_FEE_MAX)
            .min(LOCAL_FEE_MAX)
    } else {
        let delta = cur * (target - used) / target / denom;
        let next = (cur - delta) as u64;
        if next < LOCAL_FEE_MIN {
            0
        } else {
            next
        }
    }
}

/// `fee` after `blocks` blocks without gas to its target.
pub fn decay(mut fee: u64, blocks: u64) -> u64 {
    if blocks >= LOCAL_FEE_MAX_DECAY_BLOCKS {
        return 0;
    }
    for _ in 0..blocks {
        if fee == 0 {
            break;
        }
        fee = next_local_fee(fee, 0, 10_000);
    }
    fee
}

/// The local fee of the target whose table word is `word`, for transactions in block
/// `number` (the word was written at the end of block `last_block < number`).
pub fn fee_at(word: U256, number: u64) -> u64 {
    let (fee, last) = unpack(word);
    if fee == 0 {
        return 0;
    }
    decay(fee, number.saturating_sub(last).saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: u64 = 30_000_000;

    #[test]
    fn rises_above_a_quarter_of_the_block_and_decays_below() {
        // Not congested: stays zero.
        assert_eq!(next_local_fee(0, LIMIT / 4, LIMIT), 0);
        // First congested block: the minimum.
        assert_eq!(next_local_fee(0, LIMIT / 4 + 1, LIMIT), LOCAL_FEE_MIN);
        // Full block: +3/8 (excess of 3 targets, 1/8 each).
        assert_eq!(next_local_fee(8_000, LIMIT, LIMIT), 8_000 + 3_000);
        let f = 8 * LOCAL_FEE_MIN;
        assert_eq!(next_local_fee(f, LIMIT, LIMIT), f + 3 * LOCAL_FEE_MIN);
        // Half a block: +1/8.
        assert_eq!(next_local_fee(f, LIMIT / 2, LIMIT), f + LOCAL_FEE_MIN);
        // Exactly at the target: unchanged.
        assert_eq!(next_local_fee(f, LIMIT / 4, LIMIT), f);
        // Idle: -1/8, then dropped below the minimum.
        assert_eq!(next_local_fee(f, 0, LIMIT), f - LOCAL_FEE_MIN);
        assert_eq!(next_local_fee(LOCAL_FEE_MIN, 0, LIMIT), 0);
        // Capped.
        assert_eq!(next_local_fee(LOCAL_FEE_MAX, LIMIT, LIMIT), LOCAL_FEE_MAX);
        // Degenerate gas limit.
        assert_eq!(next_local_fee(f, 10, 0), 0);
    }

    #[test]
    fn lazy_decay_matches_idle_blocks() {
        let mut fee = 50 * LOCAL_FEE_MIN;
        for idle in 0..80u64 {
            assert_eq!(
                decay(50 * LOCAL_FEE_MIN, idle),
                fee,
                "after {idle} idle blocks"
            );
            fee = next_local_fee(fee, 0, LIMIT);
        }
        assert_eq!(fee, 0);
        assert_eq!(decay(LOCAL_FEE_MAX, LOCAL_FEE_MAX_DECAY_BLOCKS), 0);
    }

    #[test]
    fn table_words() {
        let w = pack(12_345, 77);
        assert_eq!(unpack(w), (12_345, 77));
        assert_eq!(pack(0, 77), U256::ZERO);
        assert_eq!(unpack(U256::ZERO), (0, 0));
        // Written at the end of block 77: applies unchanged in block 78, decays after.
        assert_eq!(fee_at(pack(8 * LOCAL_FEE_MIN, 77), 78), 8 * LOCAL_FEE_MIN);
        assert_eq!(fee_at(pack(8 * LOCAL_FEE_MIN, 77), 79), 7 * LOCAL_FEE_MIN);
        assert_eq!(fee_at(U256::MAX, 5), LOCAL_FEE_MAX);
        // Solidity mapping slot of address(0x01) at slot 0.
        assert_eq!(
            slot(Address::with_last_byte(1)),
            "0xada5013122d395ba3c54772283fb069b10426056ef8ca54750cb9bb552a59e7d"
                .parse::<B256>()
                .unwrap()
        );
    }
}

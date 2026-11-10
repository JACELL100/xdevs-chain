//! EIP-1559 base fee (plan §15.1). Integer math only (§10.6).

use xdevs_primitives::{ChainConfig, Header};

/// Base fee of the child of `parent`.
///
/// Standard EIP-1559 update (`target = gas_limit / elasticity`, max change
/// `1 / max_change_denominator`), floored at `cfg.base_fee.minimum`. A parent without
/// a base fee (should not happen: genesis carries `cfg.base_fee.initial`) is treated as
/// having `initial`.
pub fn next_base_fee(parent: &Header, cfg: &ChainConfig) -> u64 {
    let p = &cfg.base_fee;
    let parent_base = u128::from(parent.base_fee_per_gas.unwrap_or(p.initial));
    let target = u128::from(p.gas_target(parent.gas_limit));
    let used = u128::from(parent.gas_used);
    let denom = u128::from(p.max_change_denominator.max(1));

    let next = if target == 0 || used == target {
        parent_base
    } else if used > target {
        let delta = (parent_base * (used - target) / target / denom).max(1);
        parent_base + delta
    } else {
        let delta = parent_base * (target - used) / target / denom;
        parent_base - delta
    };
    u64::try_from(next).unwrap_or(u64::MAX).max(p.minimum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdevs_primitives::constants::{GWEI, MIN_BASE_FEE};

    fn parent(base: u64, used: u64) -> Header {
        Header {
            base_fee_per_gas: Some(base),
            gas_limit: 30_000_000,
            gas_used: used,
            ..Default::default()
        }
    }

    #[test]
    fn eip1559_updates() {
        let cfg = ChainConfig::dev();
        assert_eq!(next_base_fee(&parent(GWEI, 15_000_000), &cfg), GWEI);
        assert_eq!(
            next_base_fee(&parent(GWEI, 30_000_000), &cfg),
            GWEI + GWEI / 8
        );
        assert_eq!(next_base_fee(&parent(GWEI, 0), &cfg), GWEI - GWEI / 8);
    }

    #[test]
    fn floor_and_min_increase() {
        let cfg = ChainConfig::dev();
        assert_eq!(next_base_fee(&parent(MIN_BASE_FEE, 0), &cfg), MIN_BASE_FEE);
        // Increase is at least 1 wei even for tiny deltas.
        let mut c = cfg.clone();
        c.base_fee.minimum = 0;
        assert_eq!(next_base_fee(&parent(1, 15_000_001), &c), 2);
    }

    #[test]
    fn missing_parent_base_fee_uses_initial() {
        let cfg = ChainConfig::dev();
        let mut h = parent(0, 15_000_000);
        h.base_fee_per_gas = None;
        assert_eq!(next_base_fee(&h, &cfg), cfg.base_fee.initial);
    }
}

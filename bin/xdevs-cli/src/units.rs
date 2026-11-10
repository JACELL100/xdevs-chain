//! Exact decimal amount parsing (no floats): `"1.5"`, `"10 gwei"`, `"21000wei"`.

use anyhow::{bail, Context};
use xdevs_primitives::{reexports::alloy_primitives::utils::format_ether, U256};

/// Parse `amount` with an optional unit suffix (`wei`, `gwei`, `ether` / `xds` / `txds`);
/// `default_decimals` applies when there is no suffix (18 = whole tXDS, 9 = gwei).
/// More fractional digits than the unit allows is an error (never silently truncated).
pub fn parse_amount(amount: &str, default_decimals: u32) -> anyhow::Result<U256> {
    let s = amount.trim().to_ascii_lowercase();
    let (num, decimals) = if let Some(n) = s.strip_suffix("gwei") {
        (n, 9)
    } else if let Some(n) = s.strip_suffix("wei") {
        (n, 0)
    } else if let Some(n) = s
        .strip_suffix("txds")
        .or_else(|| s.strip_suffix("xds"))
        .or_else(|| s.strip_suffix("ether"))
    {
        (n, 18)
    } else {
        (s.as_str(), default_decimals)
    };
    let num = num.trim().replace('_', "");
    let (int, frac) = num.split_once('.').unwrap_or((num.as_str(), ""));
    if int.is_empty() && frac.is_empty() {
        bail!("invalid amount {amount:?}");
    }
    if !int.chars().chain(frac.chars()).all(|c| c.is_ascii_digit()) {
        bail!("invalid amount {amount:?}: only digits and one '.' allowed");
    }
    if frac.len() > decimals as usize {
        bail!("invalid amount {amount:?}: more than {decimals} decimal places");
    }
    let digits = format!("{int}{frac:0<width$}", width = decimals as usize);
    let digits = if digits.is_empty() {
        "0"
    } else {
        digits.as_str()
    };
    U256::from_str_radix(digits, 10).with_context(|| format!("amount {amount:?} too large"))
}

/// Wei as a human-readable tXDS amount without trailing zeros (`1.5`, `0`, `0.000000000000000001`).
pub fn format_xds(wei: U256) -> String {
    let s = format_ether(wei);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> U256 {
        parse_amount(s, 18).unwrap()
    }

    #[test]
    fn parses_xds_amounts() {
        assert_eq!(p("1"), U256::from(10u128.pow(18)));
        assert_eq!(p("10"), U256::from(10u128.pow(19)));
        assert_eq!(p("0.5"), U256::from(5 * 10u128.pow(17)));
        assert_eq!(p(".5"), U256::from(5 * 10u128.pow(17)));
        assert_eq!(p("2."), U256::from(2 * 10u128.pow(18)));
        assert_eq!(p("0.000000000000000001"), U256::from(1));
        assert_eq!(p("1_000"), U256::from(10u128.pow(21)));
        assert_eq!(p(" 3 tXDS "), U256::from(3 * 10u128.pow(18)));
        assert_eq!(p("0"), U256::ZERO);
    }

    #[test]
    fn parses_units() {
        assert_eq!(p("21000wei"), U256::from(21_000));
        assert_eq!(p("1.5 gwei"), U256::from(1_500_000_000u64));
        assert_eq!(parse_amount("2", 9).unwrap(), U256::from(2_000_000_000u64));
        assert_eq!(p("1ether"), U256::from(10u128.pow(18)));
    }

    #[test]
    fn rejects_bad_amounts() {
        for bad in [
            "",
            ".",
            "-1",
            "1e18",
            "1.2.3",
            "abc",
            "0x10",
            "1.0000000000000000001",
            "1.5wei",
        ] {
            assert!(parse_amount(bad, 18).is_err(), "{bad:?} should fail");
        }
        assert!(parse_amount(&"9".repeat(100), 18).is_err());
    }

    #[test]
    fn formats() {
        assert_eq!(format_xds(U256::ZERO), "0");
        assert_eq!(format_xds(p("1.5")), "1.5");
        assert_eq!(format_xds(p("1000")), "1000");
        assert_eq!(format_xds(U256::from(1)), "0.000000000000000001");
    }
}

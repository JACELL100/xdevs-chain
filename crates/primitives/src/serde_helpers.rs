//! Lenient serde helpers for human-written config files (genesis.json, config.toml).
//!
//! Numbers may be given as JSON/TOML integers, decimal strings (`"1000000000"`) or
//! `0x`-prefixed hex strings (`"0x3b9aca00"`). Values are always *serialized* in the
//! canonical form noted on each module.

use alloy_primitives::U256;
use core::{fmt, str::FromStr};
use serde::{de, Deserializer, Serializer};

fn parse_u64_str<E: de::Error>(s: &str) -> Result<u64, E> {
    let s = s.trim();
    let res = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some("") => Ok(0),
        Some(hex) => u64::from_str_radix(hex, 16),
        None => s.replace('_', "").parse::<u64>(),
    };
    res.map_err(|e| E::custom(format!("invalid integer {s:?}: {e}")))
}

fn parse_u256_str<E: de::Error>(s: &str) -> Result<U256, E> {
    let s = s.trim();
    if s == "0x" || s == "0X" {
        return Ok(U256::ZERO);
    }
    // `U256::from_str` understands both decimal and `0x`-prefixed hex.
    U256::from_str(&s.replace('_', ""))
        .map_err(|e| E::custom(format!("invalid uint256 {s:?}: {e}")))
}

/// `u64` from integer / decimal string / hex string. Serializes as a plain integer.
pub mod u64_flex {
    use super::*;

    /// Serialize as a plain integer.
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(*v)
    }

    /// Deserialize from integer, decimal string or hex string.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        struct V;
        impl de::Visitor<'_> for V {
            type Value = u64;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an unsigned integer, decimal string or 0x-hex string")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
                Ok(v)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
                u64::try_from(v).map_err(|_| E::custom("negative integer"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
                parse_u64_str(v)
            }
        }
        d.deserialize_any(V)
    }
}

/// `u64` as in [`u64_flex`] but serialized as a `0x` hex quantity (Ethereum JSON style).
pub mod u64_hex_flex {
    use super::*;

    /// Serialize as `0x`-prefixed hex.
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{v:#x}"))
    }

    /// Deserialize from integer, decimal string or hex string.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        u64_flex::deserialize(d)
    }
}

/// [`U256`] from integer / decimal string / hex string. Serializes as a decimal string
/// (JSON numbers cannot hold 256-bit values).
pub mod u256_flex {
    use super::*;

    /// Serialize as a decimal string.
    pub fn serialize<S: Serializer>(v: &U256, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    /// Deserialize from integer, decimal string or hex string.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        struct V;
        impl de::Visitor<'_> for V {
            type Value = U256;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an unsigned integer, decimal string or 0x-hex string")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<U256, E> {
                Ok(U256::from(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<U256, E> {
                u64::try_from(v)
                    .map(U256::from)
                    .map_err(|_| E::custom("negative integer"))
            }
            fn visit_u128<E: de::Error>(self, v: u128) -> Result<U256, E> {
                Ok(U256::from(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<U256, E> {
                parse_u256_str(v)
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct T {
        #[serde(with = "u64_flex")]
        a: u64,
        #[serde(with = "u256_flex")]
        b: U256,
    }

    #[test]
    fn parses_all_forms() {
        for (json, a, b) in [
            (r#"{"a":5,"b":7}"#, 5u64, 7u64),
            (r#"{"a":"5","b":"7"}"#, 5, 7),
            (r#"{"a":"0x5","b":"0x7"}"#, 5, 7),
            (r#"{"a":"0x","b":"0x0"}"#, 0, 0),
        ] {
            let t: T = serde_json::from_str(json).unwrap();
            assert_eq!(
                t,
                T {
                    a,
                    b: U256::from(b)
                }
            );
        }
        let big: T = serde_json::from_str(r#"{"a":1,"b":"300000000000000000000000000"}"#).unwrap();
        assert_eq!(
            big.b,
            U256::from(300_000_000u64) * U256::from(10u64).pow(U256::from(18u64))
        );
    }

    #[test]
    fn roundtrip() {
        let t = T {
            a: 9,
            b: U256::from(10u64),
        };
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, r#"{"a":9,"b":"10"}"#);
        assert_eq!(serde_json::from_str::<T>(&s).unwrap(), t);
    }

    #[test]
    fn rejects_garbage() {
        assert!(serde_json::from_str::<T>(r#"{"a":"abc","b":1}"#).is_err());
        assert!(serde_json::from_str::<T>(r#"{"a":-1,"b":1}"#).is_err());
    }
}

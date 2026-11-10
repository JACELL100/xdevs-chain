//! secp256k1 account keys.

use alloy_primitives::hex;
use alloy_signer_local::PrivateKeySigner;
use zeroize::Zeroizing;

/// A private key could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// Not valid hex.
    #[error("private key is not valid hex")]
    InvalidHex,
    /// Wrong length (must be 32 bytes).
    #[error("private key must be 32 bytes, got {0}")]
    InvalidLength(usize),
    /// Zero or not below the curve order.
    #[error("private key is not a valid secp256k1 scalar")]
    InvalidScalar,
}

/// Generate a fresh random account key (OS-seeded CSPRNG).
pub fn generate_signer() -> PrivateKeySigner {
    PrivateKeySigner::random()
}

/// Parse a hex private key (with or without `0x`, surrounding whitespace ignored).
/// The decoded bytes are zeroized after use.
pub fn signer_from_hex(key: &str) -> Result<PrivateKeySigner, KeyError> {
    let bytes = Zeroizing::new(hex::decode(key.trim()).map_err(|_| KeyError::InvalidHex)?);
    signer_from_bytes(&bytes)
}

/// Build a signer from a raw 32-byte private key.
pub fn signer_from_bytes(bytes: &[u8]) -> Result<PrivateKeySigner, KeyError> {
    if bytes.len() != 32 {
        return Err(KeyError::InvalidLength(bytes.len()));
    }
    PrivateKeySigner::from_slice(bytes).map_err(|_| KeyError::InvalidScalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn parses_known_key() {
        // Anvil / Hardhat default account #0.
        let s =
            signer_from_hex("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        assert_eq!(
            s.address(),
            address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")
        );
        // without prefix + whitespace
        let s2 =
            signer_from_hex(" ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80\n")
                .unwrap();
        assert_eq!(s.address(), s2.address());
    }

    #[test]
    fn rejects_bad_keys() {
        assert_eq!(signer_from_hex("zz").unwrap_err(), KeyError::InvalidHex);
        assert_eq!(
            signer_from_hex("0x01").unwrap_err(),
            KeyError::InvalidLength(1)
        );
        assert_eq!(
            signer_from_hex(&"00".repeat(32)).unwrap_err(),
            KeyError::InvalidScalar
        );
        assert_eq!(
            signer_from_hex(&"ff".repeat(32)).unwrap_err(),
            KeyError::InvalidScalar
        );
    }

    #[test]
    fn generated_keys_are_distinct_and_roundtrip() {
        let a = generate_signer();
        let b = generate_signer();
        assert_ne!(a.address(), b.address());
        let again = signer_from_bytes(a.to_bytes().as_slice()).unwrap();
        assert_eq!(again.address(), a.address());
    }
}

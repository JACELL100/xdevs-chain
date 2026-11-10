//! Web3 Secret Storage v3 keystores (the JSON key files of geth, Foundry and MetaMask).
//!
//! Format: `derived_key = KDF(password, salt)` (scrypt or pbkdf2-hmac-sha256, 32 bytes),
//! `ciphertext = AES-128-CTR(key = derived_key[0..16], iv, private_key)`,
//! `mac = keccak256(derived_key[16..32] ++ ciphertext)`. Hex fields are written without a
//! `0x` prefix (geth rejects prefixed hex) and accepted with or without it.

use aes::cipher::{KeyIvInit, StreamCipher};
use alloy_primitives::{hex, keccak256, Address};
use alloy_signer_local::PrivateKeySigner;
use rand::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::keys::signer_from_bytes;

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// Keystore format version handled here.
pub const KEYSTORE_VERSION: u64 = 3;
/// Cipher name in the JSON.
pub const CIPHER: &str = "aes-128-ctr";
/// Largest accepted scrypt `log2(n)` when decrypting (2^22 = 4 GiB at r=8 would be absurd;
/// this bounds memory for untrusted files while accepting every real-world keystore).
pub const MAX_SCRYPT_LOG_N: u8 = 20;
/// Largest accepted pbkdf2 iteration count when decrypting.
pub const MAX_PBKDF2_ROUNDS: u32 = 10_000_000;

/// Why a keystore could not be created or opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeystoreError {
    /// Malformed JSON / hex.
    #[error("invalid keystore json: {0}")]
    Json(String),
    /// `version` other than 3.
    #[error("unsupported keystore version {0}")]
    UnsupportedVersion(u64),
    /// Cipher other than `aes-128-ctr`.
    #[error("unsupported cipher {0:?}")]
    UnsupportedCipher(String),
    /// KDF other than `scrypt` / `pbkdf2` (hmac-sha256).
    #[error("unsupported kdf {0:?}")]
    UnsupportedKdf(String),
    /// KDF / cipher parameters out of range.
    #[error("invalid keystore parameters: {0}")]
    InvalidParams(String),
    /// MAC check failed: wrong password or corrupted file (geth wording).
    #[error("could not decrypt key with given password")]
    MacMismatch,
    /// Decrypted bytes are not a valid secp256k1 key.
    #[error("keystore contains an invalid private key")]
    InvalidKey,
    /// `address` field does not match the decrypted key.
    #[error("keystore address {stored} does not match key address {derived}")]
    AddressMismatch {
        /// Address written in the file.
        stored: Address,
        /// Address of the decrypted key.
        derived: Address,
    },
}

/// scrypt cost parameters for new keystores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScryptParams {
    /// `log2(n)`.
    pub log_n: u8,
    /// Block size.
    pub r: u32,
    /// Parallelism.
    pub p: u32,
}

impl ScryptParams {
    /// geth / Foundry "standard" parameters (n = 2^18, r = 8, p = 1; 256 MiB, ~1 s).
    pub const STANDARD: Self = Self {
        log_n: 18,
        r: 8,
        p: 1,
    };
    /// geth `--lightkdf` parameters (n = 2^12, r = 8, p = 6; 4 MiB).
    pub const LIGHT: Self = Self {
        log_n: 12,
        r: 8,
        p: 6,
    };
}

impl Default for ScryptParams {
    fn default() -> Self {
        Self::STANDARD
    }
}

/// Hex bytes serialized without `0x`, deserialized with or without it.
mod hex_bytes {
    use super::hex;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(s.trim()).map_err(D::Error::custom)
    }
}

/// `crypto.cipherparams`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CipherParams {
    /// 16-byte AES-CTR initial counter.
    #[serde(with = "hex_bytes")]
    pub iv: Vec<u8>,
}

/// `crypto.kdf` + `crypto.kdfparams`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kdf", content = "kdfparams", rename_all = "lowercase")]
pub enum Kdf {
    /// scrypt.
    Scrypt {
        /// Output length (≥ 32).
        dklen: u32,
        /// CPU/memory cost (power of two).
        n: u64,
        /// Parallelism.
        p: u32,
        /// Block size.
        r: u32,
        /// Salt.
        #[serde(with = "hex_bytes")]
        salt: Vec<u8>,
    },
    /// PBKDF2 (only `hmac-sha256` is defined by the spec).
    Pbkdf2 {
        /// Iterations.
        c: u32,
        /// Output length (≥ 32).
        dklen: u32,
        /// Pseudo-random function, `hmac-sha256`.
        prf: String,
        /// Salt.
        #[serde(with = "hex_bytes")]
        salt: Vec<u8>,
    },
}

/// The `crypto` object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptoSection {
    /// Always `aes-128-ctr`.
    pub cipher: String,
    /// Cipher parameters.
    pub cipherparams: CipherParams,
    /// Encrypted private key.
    #[serde(with = "hex_bytes")]
    pub ciphertext: Vec<u8>,
    /// Key derivation function and its parameters.
    #[serde(flatten)]
    pub kdf: Kdf,
    /// `keccak256(dk[16..32] ++ ciphertext)`.
    #[serde(with = "hex_bytes")]
    pub mac: Vec<u8>,
}

/// A Web3 Secret Storage v3 keystore file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keystore {
    /// Account address, lowercase hex without `0x` (optional in the spec, written by geth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Encryption data (`Crypto` accepted for old MyEtherWallet files).
    #[serde(alias = "Crypto")]
    pub crypto: CryptoSection,
    /// Random UUID v4.
    pub id: String,
    /// Must be 3.
    pub version: u64,
}

impl Keystore {
    /// Parse a keystore from JSON.
    pub fn from_json_str(json: &str) -> Result<Self, KeystoreError> {
        serde_json::from_str(json).map_err(|e| KeystoreError::Json(e.to_string()))
    }

    /// Pretty JSON (what `xdevs-cli keygen` writes).
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("keystore serialization cannot fail")
    }

    /// Address stored in the file, if present and well formed.
    pub fn stored_address(&self) -> Option<Address> {
        self.address.as_deref().and_then(|a| a.trim().parse().ok())
    }

    /// Decrypt into a signer (see [`decrypt_signer`]).
    pub fn decrypt(&self, password: impl AsRef<[u8]>) -> Result<PrivateKeySigner, KeystoreError> {
        decrypt_signer(self, password)
    }
}

/// Run the KDF described by `kdf`; the result is zeroized on drop.
fn derive_key(kdf: &Kdf, password: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
    match kdf {
        Kdf::Scrypt {
            dklen,
            n,
            p,
            r,
            salt,
        } => {
            if *dklen < 32 {
                return Err(KeystoreError::InvalidParams(format!("dklen {dklen} < 32")));
            }
            if !n.is_power_of_two() || *n < 2 {
                return Err(KeystoreError::InvalidParams(format!(
                    "scrypt n {n} is not a power of two"
                )));
            }
            let log_n = n.trailing_zeros() as u8;
            if log_n > MAX_SCRYPT_LOG_N {
                return Err(KeystoreError::InvalidParams(format!(
                    "scrypt n 2^{log_n} too large"
                )));
            }
            let params = scrypt::Params::new(log_n, *r, *p, *dklen as usize)
                .map_err(|e| KeystoreError::InvalidParams(e.to_string()))?;
            let mut out = Zeroizing::new(vec![0u8; *dklen as usize]);
            scrypt::scrypt(password, salt, &params, &mut out)
                .map_err(|e| KeystoreError::InvalidParams(e.to_string()))?;
            Ok(out)
        }
        Kdf::Pbkdf2 {
            c,
            dklen,
            prf,
            salt,
        } => {
            if prf != "hmac-sha256" {
                return Err(KeystoreError::UnsupportedKdf(format!("pbkdf2/{prf}")));
            }
            if *dklen < 32 {
                return Err(KeystoreError::InvalidParams(format!("dklen {dklen} < 32")));
            }
            if *c == 0 || *c > MAX_PBKDF2_ROUNDS {
                return Err(KeystoreError::InvalidParams(format!(
                    "pbkdf2 c {c} out of range"
                )));
            }
            let mut out = Zeroizing::new(vec![0u8; *dklen as usize]);
            pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, *c, &mut out);
            Ok(out)
        }
    }
}

/// `keccak256(dk[16..32] ++ ciphertext)`.
fn compute_mac(derived: &[u8], ciphertext: &[u8]) -> [u8; 32] {
    let mut buf = Zeroizing::new(Vec::with_capacity(16 + ciphertext.len()));
    buf.extend_from_slice(&derived[16..32]);
    buf.extend_from_slice(ciphertext);
    keccak256(&buf[..]).0
}

/// AES-128-CTR in place (encryption == decryption).
fn aes_ctr(derived: &[u8], iv: &[u8], data: &mut [u8]) -> Result<(), KeystoreError> {
    if iv.len() != 16 {
        return Err(KeystoreError::InvalidParams(format!(
            "iv must be 16 bytes, got {}",
            iv.len()
        )));
    }
    let mut cipher = Aes128Ctr::new(derived[..16].into(), iv.into());
    cipher.apply_keystream(data);
    Ok(())
}

/// Random UUID v4 string.
fn uuid_v4<R: RngCore>(rng: &mut R) -> String {
    let mut b = [0u8; 16];
    rng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Encrypt a 32-byte private key with scrypt, using the OS-seeded thread RNG for the
/// salt, IV and id.
pub fn encrypt_key(
    secret: &[u8],
    password: impl AsRef<[u8]>,
    params: ScryptParams,
) -> Result<Keystore, KeystoreError> {
    encrypt_key_with_rng(&mut rand::thread_rng(), secret, password, params)
}

/// Encrypt a signer's private key (the temporary key copy is zeroized).
pub fn encrypt_signer(
    signer: &PrivateKeySigner,
    password: impl AsRef<[u8]>,
    params: ScryptParams,
) -> Result<Keystore, KeystoreError> {
    let secret = Zeroizing::new(signer.to_bytes().0);
    encrypt_key(&secret[..], password, params)
}

/// [`encrypt_key`] with an explicit RNG (deterministic tests).
pub fn encrypt_key_with_rng<R: RngCore + CryptoRng>(
    rng: &mut R,
    secret: &[u8],
    password: impl AsRef<[u8]>,
    params: ScryptParams,
) -> Result<Keystore, KeystoreError> {
    let signer = signer_from_bytes(secret).map_err(|_| KeystoreError::InvalidKey)?;
    let (crypto, id) = encrypt_secret(rng, secret, password.as_ref(), params)?;
    Ok(Keystore {
        address: Some(hex::encode(signer.address())),
        crypto,
        id,
        version: KEYSTORE_VERSION,
    })
}

/// Encrypt arbitrary secret bytes into a v3 `crypto` section (scrypt + AES-128-CTR +
/// keccak MAC) and a fresh UUID v4. Shared by the account and the Ed25519 keystores.
pub(crate) fn encrypt_secret<R: RngCore + CryptoRng>(
    rng: &mut R,
    secret: &[u8],
    password: &[u8],
    params: ScryptParams,
) -> Result<(CryptoSection, String), KeystoreError> {
    let mut salt = vec![0u8; 32];
    rng.fill_bytes(&mut salt);
    let mut iv = vec![0u8; 16];
    rng.fill_bytes(&mut iv);

    let kdf = Kdf::Scrypt {
        dklen: 32,
        n: 1u64 << params.log_n,
        p: params.p,
        r: params.r,
        salt,
    };
    let derived = derive_key(&kdf, password)?;
    let mut ciphertext = secret.to_vec();
    aes_ctr(&derived, &iv, &mut ciphertext)?;
    let mac = compute_mac(&derived, &ciphertext).to_vec();
    let crypto = CryptoSection {
        cipher: CIPHER.to_string(),
        cipherparams: CipherParams { iv },
        ciphertext,
        kdf,
        mac,
    };
    Ok((crypto, uuid_v4(rng)))
}

/// Decrypt a v3 `crypto` section (version / cipher / KDF bounds / constant-time MAC
/// checked). The plaintext is zeroized on drop.
pub(crate) fn decrypt_secret(
    version: u64,
    c: &CryptoSection,
    password: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
    if version != KEYSTORE_VERSION {
        return Err(KeystoreError::UnsupportedVersion(version));
    }
    if c.cipher != CIPHER {
        return Err(KeystoreError::UnsupportedCipher(c.cipher.clone()));
    }
    let derived = derive_key(&c.kdf, password)?;
    let mac = compute_mac(&derived, &c.ciphertext);
    if !bool::from(mac.as_slice().ct_eq(&c.mac)) {
        return Err(KeystoreError::MacMismatch);
    }
    let mut plain = Zeroizing::new(c.ciphertext.clone());
    aes_ctr(&derived, &c.cipherparams.iv, &mut plain)?;
    Ok(plain)
}

/// Decrypt the raw private key bytes (zeroized on drop). Verifies the MAC in constant time.
pub fn decrypt_key(
    keystore: &Keystore,
    password: impl AsRef<[u8]>,
) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
    decrypt_secret(keystore.version, &keystore.crypto, password.as_ref())
}

/// Decrypt a keystore into a signer, checking the optional `address` field.
pub fn decrypt_signer(
    keystore: &Keystore,
    password: impl AsRef<[u8]>,
) -> Result<PrivateKeySigner, KeystoreError> {
    let secret = decrypt_key(keystore, password)?;
    let signer = signer_from_bytes(&secret).map_err(|_| KeystoreError::InvalidKey)?;
    if let Some(stored) = keystore.stored_address() {
        if stored != signer.address() {
            return Err(KeystoreError::AddressMismatch {
                stored,
                derived: signer.address(),
            });
        }
    }
    Ok(signer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    /// Private key of both Web3 Secret Storage spec test vectors (password "testpassword").
    const SPEC_KEY: &str = "7a28b5ba57c53603b0b07b56bba752f7784bf506fa95edc395f5cf6c7514fe9d";

    /// Spec vector #1: PBKDF2-SHA-256.
    const SPEC_PBKDF2: &str = r#"{
        "crypto" : {
            "cipher" : "aes-128-ctr",
            "cipherparams" : { "iv" : "6087dab2f9fdbbfaddc31a909735c1e6" },
            "ciphertext" : "5318b4d5bcd28de64ee5559e671353e16f075ecae9f99c7a79a38af5f869aa46",
            "kdf" : "pbkdf2",
            "kdfparams" : {
                "c" : 262144,
                "dklen" : 32,
                "prf" : "hmac-sha256",
                "salt" : "ae3cd4e7013836a3df6bd7241b12db061dbe2c6785853cce422d148a624ce0bd"
            },
            "mac" : "517ead924a9d0dc3124507e3393d175ce3ff7c1e96529c6c555ce9e51205e9b2"
        },
        "id" : "3198bc9c-6672-5ab3-d995-4942343ae5b6",
        "version" : 3
    }"#;

    /// Spec vector #2: scrypt (n = 2^18, r = 1, p = 8).
    const SPEC_SCRYPT: &str = r#"{
        "crypto" : {
            "cipher" : "aes-128-ctr",
            "cipherparams" : { "iv" : "83dbcc02d8ccb40e466191a123791e0e" },
            "ciphertext" : "d172bf743a674da9cdad04534d56926ef8358534d458fffccd4e6ad2fbde479c",
            "kdf" : "scrypt",
            "kdfparams" : {
                "dklen" : 32,
                "n" : 262144,
                "p" : 8,
                "r" : 1,
                "salt" : "ab0c7876052600dd703518d6fc3fe8984592145b591fc8fb5c6d43190334ba19"
            },
            "mac" : "2103ac29920d71da29f15d75b4a16dbe95cfd7ff8faea1056c33131d846e3097"
        },
        "id" : "3198bc9c-6672-5ab3-d995-4942343ae5b6",
        "version" : 3
    }"#;

    #[test]
    fn spec_vector_pbkdf2() {
        let ks = Keystore::from_json_str(SPEC_PBKDF2).unwrap();
        let key = decrypt_key(&ks, "testpassword").unwrap();
        assert_eq!(hex::encode(&key[..]), SPEC_KEY);
        assert_eq!(
            decrypt_key(&ks, "wrong").unwrap_err(),
            KeystoreError::MacMismatch
        );
    }

    /// The spec's scrypt vector uses r = 1 with n = 2^18, which violates RFC 7914's
    /// `n < 2^(16 r)` bound. The RustCrypto `scrypt` crate enforces the RFC, so this file
    /// is refused with a clear error (geth / Foundry always write r = 8, see below).
    #[test]
    fn spec_vector_scrypt_r1_is_refused_cleanly() {
        let ks = Keystore::from_json_str(SPEC_SCRYPT).unwrap();
        assert!(matches!(
            decrypt_key(&ks, "testpassword"),
            Err(KeystoreError::InvalidParams(_))
        ));
    }

    /// Standard-strength scrypt keystore (n = 2^18, r = 8, p = 1) from the web3.js
    /// `accounts.encrypt` documentation.
    const WEB3JS_SCRYPT: &str = r#"{
        "version": 3,
        "id": "04e9bcbb-96fa-497b-94d1-14df4cd20af6",
        "address": "2c7536e3605d9c16a7a3d7b1898e529396a65c23",
        "crypto": {
            "ciphertext": "a1c25da3ecde4e6a24f3697251dd15d6208520efc84ad97397e906e6df24d251",
            "cipherparams": { "iv": "2885df2b63f7ef247d753c82fa20038a" },
            "cipher": "aes-128-ctr",
            "kdf": "scrypt",
            "kdfparams": {
                "dklen": 32,
                "salt": "4531b3c174cc3ff32a6a7a85d6761b410db674807b2d216d022318ceee50be10",
                "n": 262144,
                "r": 8,
                "p": 1
            },
            "mac": "b8b010fff37f9ae5559a352a185e86f9b9c1d7f7a9f1bd4e82a5dd35468fc7f6"
        }
    }"#;

    #[test]
    fn web3js_vector_scrypt() {
        let ks = Keystore::from_json_str(WEB3JS_SCRYPT).unwrap();
        let signer = decrypt_signer(&ks, "test!").unwrap();
        assert_eq!(
            hex::encode(signer.to_bytes()),
            "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
        );
        assert_eq!(ks.stored_address(), Some(signer.address()));
    }

    fn light_keystore(password: &str) -> (Keystore, PrivateKeySigner) {
        let signer = crate::keys::generate_signer();
        let ks = encrypt_signer(&signer, password, ScryptParams::LIGHT).unwrap();
        (ks, signer)
    }

    #[test]
    fn encrypt_decrypt_roundtrip_through_json() {
        let (ks, signer) = light_keystore("hunter2");
        let json = ks.to_json_string();
        // geth-style: unprefixed hex, lowercase kdf names, version 3.
        assert!(!json.contains("0x"));
        assert!(json.contains("\"kdf\": \"scrypt\""));
        assert!(json.contains("\"kdfparams\""));
        let parsed = Keystore::from_json_str(&json).unwrap();
        assert_eq!(parsed, ks);
        assert_eq!(parsed.stored_address(), Some(signer.address()));
        let back = parsed.decrypt("hunter2").unwrap();
        assert_eq!(back.address(), signer.address());
        assert_eq!(
            parsed.decrypt("hunter3").unwrap_err(),
            KeystoreError::MacMismatch
        );
    }

    #[test]
    fn deterministic_with_seeded_rng_and_uuid_v4() {
        let key = hex::decode(SPEC_KEY).unwrap();
        let mut r1 = rand::rngs::StdRng::seed_from_u64(7);
        let mut r2 = rand::rngs::StdRng::seed_from_u64(7);
        let a = encrypt_key_with_rng(&mut r1, &key, "pw", ScryptParams::LIGHT).unwrap();
        let b = encrypt_key_with_rng(&mut r2, &key, "pw", ScryptParams::LIGHT).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.id.len(), 36);
        assert_eq!(a.id.as_bytes()[14], b'4');
        assert!(matches!(a.id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn accepts_prefixed_hex_and_capital_crypto() {
        let (ks, signer) = light_keystore("pw");
        let mut v: serde_json::Value = serde_json::from_str(&ks.to_json_string()).unwrap();
        let crypto = v.as_object_mut().unwrap().remove("crypto").unwrap();
        v["Crypto"] = crypto;
        let ct = v["Crypto"]["ciphertext"].as_str().unwrap().to_string();
        v["Crypto"]["ciphertext"] = format!("0x{ct}").into();
        v["address"] = format!("0x{}", v["address"].as_str().unwrap()).into();
        let parsed = Keystore::from_json_str(&v.to_string()).unwrap();
        assert_eq!(parsed.decrypt("pw").unwrap().address(), signer.address());
    }

    #[test]
    fn rejects_tampering_and_bad_params() {
        let (ks, _) = light_keystore("pw");

        let mut t = ks.clone();
        t.crypto.ciphertext[0] ^= 1;
        assert_eq!(
            decrypt_key(&t, "pw").unwrap_err(),
            KeystoreError::MacMismatch
        );

        let mut t = ks.clone();
        t.version = 1;
        assert_eq!(
            decrypt_key(&t, "pw").unwrap_err(),
            KeystoreError::UnsupportedVersion(1)
        );

        let mut t = ks.clone();
        t.crypto.cipher = "aes-128-cbc".into();
        assert!(matches!(
            decrypt_key(&t, "pw"),
            Err(KeystoreError::UnsupportedCipher(_))
        ));

        let mut t = ks.clone();
        if let Kdf::Scrypt { n, .. } = &mut t.crypto.kdf {
            *n = 1000;
        }
        assert!(matches!(
            decrypt_key(&t, "pw"),
            Err(KeystoreError::InvalidParams(_))
        ));

        let mut t = ks.clone();
        if let Kdf::Scrypt { n, .. } = &mut t.crypto.kdf {
            *n = 1 << 30;
        }
        assert!(matches!(
            decrypt_key(&t, "pw"),
            Err(KeystoreError::InvalidParams(_))
        ));

        let mut t = ks.clone();
        t.address = Some(hex::encode([0x11u8; 20]));
        assert!(matches!(
            decrypt_signer(&t, "pw"),
            Err(KeystoreError::AddressMismatch { .. })
        ));

        let json = ks.to_json_string().replace("\"scrypt\"", "\"argon2\"");
        assert!(matches!(
            Keystore::from_json_str(&json),
            Err(KeystoreError::Json(_))
        ));
    }

    #[test]
    fn rejects_invalid_secret() {
        assert_eq!(
            encrypt_key(&[0u8; 32], "pw", ScryptParams::LIGHT).unwrap_err(),
            KeystoreError::InvalidKey
        );
        assert_eq!(
            encrypt_key(&[1u8; 31], "pw", ScryptParams::LIGHT).unwrap_err(),
            KeystoreError::InvalidKey
        );
    }
}

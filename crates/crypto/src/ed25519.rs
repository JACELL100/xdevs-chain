//! Ed25519 key files (Phase 2, research doc §6.7, §6.10; plan §9).
//!
//! Two different keys (plan §9: consensus, p2p and fee-recipient keys are separate):
//!
//! * **Consensus key** ([`ConsensusKeystore`]): the validator's vote-signing key. The
//!   32-byte Ed25519 seed is encrypted exactly like a Web3 Secret Storage v3 keystore
//!   (scrypt + AES-128-CTR + keccak MAC); only the envelope fields differ:
//!
//!   ```json
//!   { "type": "ed25519", "publicKey": "ed25519:0x<64 hex>", "validatorAddress": "0x<40 hex>",
//!     "crypto": { "cipher": "aes-128-ctr", "kdf": "scrypt", ... }, "id": "<uuid>", "version": 3 }
//!   ```
//!
//!   `publicKey` uses the genesis `consensusKey` format, so it can be pasted into
//!   `genesis.json`. `validatorAddress` is `sha256(publicKey)[..20]` (the consensus
//!   `ValidatorAddress`, not an EVM address).
//! * **Node key** ([`NodeKey`]): the libp2p identity. Low value, stored unencrypted with
//!   owner-only permissions (like CometBFT's `node_key.json`):
//!
//!   ```json
//!   { "type": "ed25519", "privateKey": "0x<64 hex seed>", "publicKey": "0x<64 hex>",
//!     "peerId": "12D3KooW..." }
//!   ```
//!
//! A seed gives the same public key in `ed25519-dalek` (here, libp2p) and in
//! `ed25519-consensus` (Malachite). Secrets live in [`Ed25519Seed`] (zeroized on drop,
//! never printed by `Debug`).

use std::{fmt, io::Write as _, path::Path};

use alloy_primitives::hex;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::{rngs::OsRng, CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::keystore::{
    decrypt_secret, encrypt_secret, CryptoSection, KeystoreError, ScryptParams, KEYSTORE_VERSION,
};

/// `type` field of both key files.
pub const ED25519_KEY_TYPE: &str = "ed25519";
/// Prefix of a formatted Ed25519 public key (same as the genesis `consensusKey`).
pub const ED25519_PUBKEY_PREFIX: &str = "ed25519:";

/// Errors of the Ed25519 key files.
#[derive(Debug, thiserror::Error)]
pub enum KeyFileError {
    /// Reading or writing the file failed.
    #[error("{path}: {source}")]
    Io {
        /// File path.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// Malformed JSON or field.
    #[error("invalid key file: {0}")]
    Format(String),
    /// Decryption failed (wrong password, bad parameters...).
    #[error(transparent)]
    Keystore(#[from] KeystoreError),
    /// The stored public key / address / peer id does not match the secret.
    #[error("key file mismatch: {0}")]
    Mismatch(String),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> KeyFileError + '_ {
    move |source| KeyFileError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// A 32-byte Ed25519 secret seed (RFC 8032 secret key), zeroized on drop.
#[derive(Clone)]
pub struct Ed25519Seed(Zeroizing<[u8; 32]>);

impl Ed25519Seed {
    /// A fresh random seed from the OS CSPRNG.
    pub fn generate() -> Self {
        Self::generate_with_rng(&mut OsRng)
    }

    /// A fresh seed from `rng` (deterministic tests).
    pub fn generate_with_rng<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut seed = Zeroizing::new([0u8; 32]);
        rng.fill_bytes(&mut seed[..]);
        Self(seed)
    }

    /// Copy a seed (every 32-byte string is a valid Ed25519 seed).
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(Zeroizing::new(*bytes))
    }

    /// Seed from a slice that must be exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyFileError> {
        let arr: &[u8; 32] = bytes.try_into().map_err(|_| {
            KeyFileError::Format(format!(
                "ed25519 seed must be 32 bytes, got {}",
                bytes.len()
            ))
        })?;
        Ok(Self::from_bytes(arr))
    }

    /// Seed from hex (`0x` optional, surrounding whitespace ignored).
    pub fn from_hex(s: &str) -> Result<Self, KeyFileError> {
        let bytes = Zeroizing::new(
            hex::decode(s.trim()).map_err(|_| KeyFileError::Format("seed is not hex".into()))?,
        );
        Self::from_slice(&bytes)
    }

    /// The secret bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// A zeroizing copy (the form `xdevs_consensus::EngineKeys` takes).
    pub fn to_zeroizing(&self) -> Zeroizing<[u8; 32]> {
        self.0.clone()
    }

    /// The Ed25519 public key.
    pub fn public_key(&self) -> [u8; 32] {
        ed25519_public_key(&self.0)
    }
}

impl fmt::Debug for Ed25519Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Ed25519Seed(public: 0x{})",
            hex::encode(self.public_key())
        )
    }
}

impl PartialEq for Ed25519Seed {
    /// Constant-time comparison.
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        self.0[..].ct_eq(&other.0[..]).into()
    }
}

impl Eq for Ed25519Seed {}

/// Public key of an Ed25519 seed (the signing-key copy is zeroized on drop).
pub fn ed25519_public_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// Whether `key` decodes to an Ed25519 curve point.
pub fn is_valid_ed25519_public_key(key: &[u8; 32]) -> bool {
    VerifyingKey::from_bytes(key).is_ok()
}

/// Format a public key as `"ed25519:0x<64 hex>"` (genesis `consensusKey` format).
pub fn format_ed25519_public_key(key: &[u8; 32]) -> String {
    format!("{ED25519_PUBKEY_PREFIX}0x{}", hex::encode(key))
}

/// Parse `"ed25519:0x<64 hex>"` (the `0x` is optional).
pub fn parse_ed25519_public_key(s: &str) -> Result<[u8; 32], KeyFileError> {
    let bad = || KeyFileError::Format(format!("invalid ed25519 public key {s:?}"));
    let body = s
        .trim()
        .strip_prefix(ED25519_PUBKEY_PREFIX)
        .ok_or_else(bad)?;
    hex::decode(body)
        .map_err(|_| bad())?
        .try_into()
        .map_err(|_| bad())
}

/// Consensus validator address: `sha256(public_key)[..20]` (CometBFT convention; equals
/// `xdevs_consensus::ValidatorAddress::from_public_key`).
pub fn validator_address(public_key: &[u8; 32]) -> [u8; 20] {
    let digest = Sha256::digest(public_key);
    let mut out = [0u8; 20];
    out.copy_from_slice(&digest[..20]);
    out
}

/// libp2p `PeerId` (base58btc, `12D3KooW...`) of an Ed25519 public key: the identity
/// multihash of the protobuf `PublicKey { Type: Ed25519, Data: key }`, as libp2p 0.56
/// computes it.
pub fn peer_id(public_key: &[u8; 32]) -> String {
    // multihash(identity = 0x00, len = 36) ‖ protobuf: field 1 varint 1 (Ed25519),
    // field 2 bytes (len 32).
    let mut buf = Vec::with_capacity(38);
    buf.extend_from_slice(&[0x00, 0x24, 0x08, 0x01, 0x12, 0x20]);
    buf.extend_from_slice(public_key);
    bs58::encode(buf).into_string()
}

// ---------------------------------------------------------------------------
// Consensus key file
// ---------------------------------------------------------------------------

/// An encrypted validator consensus key file (see the module docs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusKeystore {
    /// Always [`ED25519_KEY_TYPE`].
    #[serde(rename = "type")]
    pub key_type: String,
    /// `"ed25519:0x<64 hex>"`.
    pub public_key: String,
    /// `0x` + 20-byte [`validator_address`] (informational; checked on decrypt if present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator_address: Option<String>,
    /// Web3 Secret Storage v3 encryption of the 32-byte seed.
    pub crypto: CryptoSection,
    /// Random UUID v4.
    pub id: String,
    /// Must be 3.
    pub version: u64,
}

impl ConsensusKeystore {
    /// Encrypt `seed` with scrypt `params` (OS-seeded RNG for salt, IV and id).
    pub fn encrypt(
        seed: &Ed25519Seed,
        password: impl AsRef<[u8]>,
        params: ScryptParams,
    ) -> Result<Self, KeyFileError> {
        Self::encrypt_with_rng(&mut rand::thread_rng(), seed, password, params)
    }

    /// [`Self::encrypt`] with an explicit RNG (deterministic tests).
    pub fn encrypt_with_rng<R: RngCore + CryptoRng>(
        rng: &mut R,
        seed: &Ed25519Seed,
        password: impl AsRef<[u8]>,
        params: ScryptParams,
    ) -> Result<Self, KeyFileError> {
        let public = seed.public_key();
        let (crypto, id) = encrypt_secret(rng, seed.as_bytes(), password.as_ref(), params)?;
        Ok(Self {
            key_type: ED25519_KEY_TYPE.into(),
            public_key: format_ed25519_public_key(&public),
            validator_address: Some(hex::encode_prefixed(validator_address(&public))),
            crypto,
            id,
            version: KEYSTORE_VERSION,
        })
    }

    /// Parse from JSON (checks the type and the public key format).
    pub fn from_json_str(json: &str) -> Result<Self, KeyFileError> {
        let ks: Self =
            serde_json::from_str(json).map_err(|e| KeyFileError::Format(e.to_string()))?;
        if ks.key_type != ED25519_KEY_TYPE {
            return Err(KeyFileError::Format(format!(
                "consensus key type {:?}, expected {ED25519_KEY_TYPE:?}",
                ks.key_type
            )));
        }
        ks.public_key()?;
        Ok(ks)
    }

    /// Pretty JSON.
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("keystore serialization cannot fail")
    }

    /// Read and parse a key file.
    pub fn load(path: &Path) -> Result<Self, KeyFileError> {
        Self::from_json_str(&std::fs::read_to_string(path).map_err(io_err(path))?)
    }

    /// The public key stored in the file (no password needed).
    pub fn public_key(&self) -> Result<[u8; 32], KeyFileError> {
        parse_ed25519_public_key(&self.public_key)
    }

    /// Decrypt the seed and check it against `publicKey` and `validatorAddress`.
    pub fn decrypt(&self, password: impl AsRef<[u8]>) -> Result<Ed25519Seed, KeyFileError> {
        let plain = decrypt_secret(self.version, &self.crypto, password.as_ref())?;
        let seed = Ed25519Seed::from_slice(&plain)?;
        let public = seed.public_key();
        if public != self.public_key()? {
            return Err(KeyFileError::Mismatch(format!(
                "publicKey {} does not match the decrypted key {}",
                self.public_key,
                format_ed25519_public_key(&public)
            )));
        }
        if let Some(stored) = &self.validator_address {
            let derived = validator_address(&public);
            let ok = hex::decode(stored.trim()).is_ok_and(|b| b == derived);
            if !ok {
                return Err(KeyFileError::Mismatch(format!(
                    "validatorAddress {stored} does not match {}",
                    hex::encode_prefixed(derived)
                )));
            }
        }
        Ok(seed)
    }
}

/// Load and decrypt a consensus key file (what the node does at start).
pub fn read_consensus_key(
    path: &Path,
    password: impl AsRef<[u8]>,
) -> Result<Ed25519Seed, KeyFileError> {
    ConsensusKeystore::load(path)?.decrypt(password)
}

// ---------------------------------------------------------------------------
// Node (libp2p) key file
// ---------------------------------------------------------------------------

/// JSON shape of a node key file; the secret string is wiped on drop.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeKeyJson {
    #[serde(rename = "type")]
    key_type: String,
    private_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peer_id: Option<String>,
}

impl Drop for NodeKeyJson {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

/// A libp2p Ed25519 node identity (see the module docs). The node builds its libp2p
/// keypair with `Keypair::ed25519_from_bytes(seed)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeKey {
    seed: Ed25519Seed,
}

impl NodeKey {
    /// A fresh random identity.
    pub fn generate() -> Self {
        Self::from_seed(Ed25519Seed::generate())
    }

    /// Identity from a seed.
    pub fn from_seed(seed: Ed25519Seed) -> Self {
        Self { seed }
    }

    /// The secret seed.
    pub fn seed(&self) -> &Ed25519Seed {
        &self.seed
    }

    /// Ed25519 public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.seed.public_key()
    }

    /// libp2p peer id (`12D3KooW...`).
    pub fn peer_id(&self) -> String {
        peer_id(&self.public_key())
    }

    /// Pretty JSON, **containing the secret** (zeroized on drop).
    pub fn to_json_string(&self) -> Zeroizing<String> {
        let json = NodeKeyJson {
            key_type: ED25519_KEY_TYPE.into(),
            private_key: hex::encode_prefixed(self.seed.as_bytes()),
            public_key: Some(hex::encode_prefixed(self.public_key())),
            peer_id: Some(self.peer_id()),
        };
        Zeroizing::new(serde_json::to_string_pretty(&json).expect("node key serializes"))
    }

    /// Parse from JSON; `publicKey` and `peerId`, when present, must match the secret.
    pub fn from_json_str(json: &str) -> Result<Self, KeyFileError> {
        let parsed: NodeKeyJson =
            serde_json::from_str(json).map_err(|e| KeyFileError::Format(e.to_string()))?;
        if parsed.key_type != ED25519_KEY_TYPE {
            return Err(KeyFileError::Format(format!(
                "node key type {:?}, expected {ED25519_KEY_TYPE:?}",
                parsed.key_type
            )));
        }
        let key = Self::from_seed(Ed25519Seed::from_hex(&parsed.private_key)?);
        if let Some(pk) = &parsed.public_key {
            if !hex::decode(pk.trim()).is_ok_and(|b| b == key.public_key()) {
                return Err(KeyFileError::Mismatch(format!(
                    "publicKey {pk} does not match the private key"
                )));
            }
        }
        if let Some(id) = &parsed.peer_id {
            if id.trim() != key.peer_id() {
                return Err(KeyFileError::Mismatch(format!(
                    "peerId {id} does not match the private key ({})",
                    key.peer_id()
                )));
            }
        }
        Ok(key)
    }

    /// Read a node key file.
    pub fn load(path: &Path) -> Result<Self, KeyFileError> {
        let json = Zeroizing::new(std::fs::read_to_string(path).map_err(io_err(path))?);
        Self::from_json_str(&json)
    }

    /// Write the key file (owner-only permissions on Unix). Refuses to replace an
    /// existing file unless `overwrite`.
    pub fn save(&self, path: &Path, overwrite: bool) -> Result<(), KeyFileError> {
        write_secret_file(path, self.to_json_string().as_bytes(), overwrite)
    }

    /// Load `path`, or create it with a fresh identity if it does not exist.
    pub fn load_or_generate(path: &Path) -> Result<Self, KeyFileError> {
        if path.exists() {
            return Self::load(path);
        }
        let key = Self::generate();
        key.save(path, false)?;
        Ok(key)
    }
}

/// Write `content` (+ newline) to `path`, creating parent directories. Owner-only
/// permissions (0600) on Unix; `overwrite = false` fails if the file exists.
pub fn write_secret_file(path: &Path, content: &[u8], overwrite: bool) -> Result<(), KeyFileError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if overwrite {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path).map_err(io_err(path))?;
    file.write_all(content).map_err(io_err(path))?;
    file.write_all(b"\n").map_err(io_err(path))?;
    file.sync_all().map_err(io_err(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    /// RFC 8032 §7.1 test 1.
    const RFC_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const RFC_PUB: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn rfc_seed() -> Ed25519Seed {
        Ed25519Seed::from_hex(RFC_SEED).unwrap()
    }

    #[test]
    fn rfc8032_public_key_and_formats() {
        let seed = rfc_seed();
        assert_eq!(hex::encode(seed.public_key()), RFC_PUB);
        let s = format_ed25519_public_key(&seed.public_key());
        assert_eq!(s, format!("ed25519:0x{RFC_PUB}"));
        assert_eq!(parse_ed25519_public_key(&s).unwrap(), seed.public_key());
        assert_eq!(
            parse_ed25519_public_key(&format!("ed25519:{RFC_PUB}")).unwrap(),
            seed.public_key()
        );
        for bad in [RFC_PUB, "ed25519:0x12", "secp256k1:0x00", "ed25519:zz"] {
            assert!(parse_ed25519_public_key(bad).is_err(), "{bad}");
        }
        assert!(is_valid_ed25519_public_key(&seed.public_key()));
        // y = 2 is not on the curve.
        let mut not_a_point = [0u8; 32];
        not_a_point[0] = 2;
        assert!(!is_valid_ed25519_public_key(&not_a_point));
        let digest = Sha256::digest(seed.public_key());
        assert_eq!(validator_address(&seed.public_key())[..], digest[..20]);
    }

    #[test]
    fn seed_debug_hides_secret_and_rejects_bad_input() {
        let seed = rfc_seed();
        let dbg = format!("{seed:?}");
        assert!(dbg.contains(RFC_PUB) && !dbg.contains(RFC_SEED));
        assert_eq!(*seed.to_zeroizing(), *seed.as_bytes());
        assert!(Ed25519Seed::from_hex("zz").is_err());
        assert!(Ed25519Seed::from_slice(&[1u8; 31]).is_err());
        assert_ne!(Ed25519Seed::generate(), Ed25519Seed::generate());
        assert_eq!(rfc_seed(), seed);
    }

    /// The hand-built peer id equals libp2p's (`libp2p-identity` 0.2.14, Malachite's).
    #[test]
    fn peer_id_matches_libp2p() {
        for seed in [rfc_seed(), Ed25519Seed::generate(), Ed25519Seed::generate()] {
            let mut bytes = *seed.as_bytes();
            let kp = libp2p_identity::Keypair::ed25519_from_bytes(&mut bytes).unwrap();
            let expected = kp.public().to_peer_id().to_base58();
            assert_eq!(peer_id(&seed.public_key()), expected);
            assert!(expected.starts_with("12D3KooW"));
        }
    }

    #[test]
    fn consensus_keystore_roundtrip() {
        let seed = Ed25519Seed::generate();
        let ks = ConsensusKeystore::encrypt(&seed, "pw", ScryptParams::LIGHT).unwrap();
        let json = ks.to_json_string();
        assert!(json.contains("\"type\": \"ed25519\""));
        assert!(json.contains("\"kdf\": \"scrypt\""));
        assert!(!json.contains(&hex::encode(seed.as_bytes())));
        let parsed = ConsensusKeystore::from_json_str(&json).unwrap();
        assert_eq!(parsed, ks);
        assert_eq!(parsed.public_key().unwrap(), seed.public_key());
        assert_eq!(parsed.decrypt("pw").unwrap(), seed);
        assert!(matches!(
            parsed.decrypt("nope"),
            Err(KeyFileError::Keystore(KeystoreError::MacMismatch))
        ));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k").join("consensus.json");
        write_secret_file(&path, json.as_bytes(), false).unwrap();
        assert!(write_secret_file(&path, b"x", false).is_err());
        assert_eq!(read_consensus_key(&path, "pw").unwrap(), seed);
        assert!(matches!(
            read_consensus_key(&dir.path().join("missing.json"), "pw"),
            Err(KeyFileError::Io { .. })
        ));
    }

    #[test]
    fn consensus_keystore_is_deterministic_with_seeded_rng() {
        let mut r1 = rand::rngs::StdRng::seed_from_u64(1);
        let mut r2 = rand::rngs::StdRng::seed_from_u64(1);
        let a = ConsensusKeystore::encrypt_with_rng(&mut r1, &rfc_seed(), "p", ScryptParams::LIGHT)
            .unwrap();
        let b = ConsensusKeystore::encrypt_with_rng(&mut r2, &rfc_seed(), "p", ScryptParams::LIGHT)
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn consensus_keystore_rejects_tampering() {
        let seed = rfc_seed();
        let ks = ConsensusKeystore::encrypt(&seed, "pw", ScryptParams::LIGHT).unwrap();

        let mut other_pub = ks.clone();
        other_pub.public_key = format_ed25519_public_key(&Ed25519Seed::generate().public_key());
        assert!(matches!(
            other_pub.decrypt("pw"),
            Err(KeyFileError::Mismatch(_))
        ));

        let mut other_addr = ks.clone();
        other_addr.validator_address = Some(hex::encode_prefixed([7u8; 20]));
        assert!(matches!(
            other_addr.decrypt("pw"),
            Err(KeyFileError::Mismatch(_))
        ));
        let mut no_addr = ks.clone();
        no_addr.validator_address = None;
        assert_eq!(no_addr.decrypt("pw").unwrap(), seed);

        let mut flipped = ks.clone();
        flipped.crypto.ciphertext[0] ^= 1;
        assert!(matches!(
            flipped.decrypt("pw"),
            Err(KeyFileError::Keystore(KeystoreError::MacMismatch))
        ));

        let wrong_type = ks.to_json_string().replace("\"ed25519\"", "\"secp256k1\"");
        assert!(ConsensusKeystore::from_json_str(&wrong_type).is_err());
        let bad_pub = ks.to_json_string().replace(&ks.public_key, "ed25519:0x00");
        assert!(ConsensusKeystore::from_json_str(&bad_pub).is_err());
        // An account keystore is not a consensus key file.
        let account =
            crate::encrypt_signer(&crate::generate_signer(), "pw", ScryptParams::LIGHT).unwrap();
        assert!(ConsensusKeystore::from_json_str(&account.to_json_string()).is_err());
    }

    #[test]
    fn node_key_roundtrip_and_checks() {
        let key = NodeKey::from_seed(rfc_seed());
        let json = key.to_json_string();
        assert!(json.contains(&key.peer_id()));
        assert_eq!(NodeKey::from_json_str(&json).unwrap(), key);
        assert!(format!("{key:?}").contains(RFC_PUB));
        assert!(!format!("{key:?}").contains(RFC_SEED));

        // Optional fields may be omitted.
        let minimal = format!(r#"{{"type":"ed25519","privateKey":"0x{RFC_SEED}"}}"#);
        assert_eq!(NodeKey::from_json_str(&minimal).unwrap(), key);
        // Mismatches and bad shapes are refused.
        let other = NodeKey::generate();
        let wrong_peer = json.replace(&key.peer_id(), &other.peer_id());
        assert!(matches!(
            NodeKey::from_json_str(&wrong_peer),
            Err(KeyFileError::Mismatch(_))
        ));
        let wrong_pub = json.replace(RFC_PUB, &hex::encode(other.public_key()));
        assert!(matches!(
            NodeKey::from_json_str(&wrong_pub),
            Err(KeyFileError::Mismatch(_))
        ));
        assert!(NodeKey::from_json_str(&json.replace("\"ed25519\"", "\"rsa\"")).is_err());
        assert!(NodeKey::from_json_str(r#"{"type":"ed25519","privateKey":"0x01"}"#).is_err());
    }

    #[test]
    fn node_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node_key.json");
        let created = NodeKey::load_or_generate(&path).unwrap();
        assert_eq!(NodeKey::load_or_generate(&path).unwrap(), created);
        assert_eq!(NodeKey::load(&path).unwrap(), created);
        let other = NodeKey::generate();
        assert!(other.save(&path, false).is_err());
        other.save(&path, true).unwrap();
        assert_eq!(NodeKey::load(&path).unwrap(), other);
    }
}

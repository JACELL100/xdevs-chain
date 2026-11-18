//! Threshold public-key encryption for the encrypted mempool prototype (Phase 5,
//! `docs/research/encrypted-mempool.md`).
//!
//! **Research-grade. Not reviewed. Do not use on mainnet.** Every primitive comes from an
//! audited crate; this module only composes them:
//!
//! * group: secp256k1 (`k256`, already used for transaction signatures);
//! * KEM: hashed ElGamal with a committee key `Y = s·G` whose secret `s` is Shamir-shared
//!   t-of-n (`s_i = f(i)`, verification keys `Y_i = s_i·G`) by a trusted dealer
//!   ([`deal`]);
//! * DEM: HKDF-SHA256 → ChaCha20-Poly1305 (the label is the AEAD associated data);
//! * non-malleability: a Schnorr proof of knowledge of the encryption randomness `r`
//!   (`U = r·G`) bound to the label and the body (the TDH1 construction of Shoup and
//!   Gennaro), so a ciphertext cannot be copied into another envelope / sender and
//!   decrypted early;
//! * decryption shares `D_i = s_i·U` with a Chaum-Pedersen DLEQ proof
//!   (`log_G Y_i = log_U D_i`), publicly verifiable against `Y_i`;
//! * combination by Lagrange interpolation in the exponent: `K = Σ λ_i·D_i = s·U = r·Y`.
//!
//! Fiat-Shamir challenges are `sha256(domain ‖ len ‖ part ‖ …)` reduced modulo the group
//! order (length-prefixed, so part boundaries are unambiguous).

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use hkdf::Hkdf;
use k256::{
    elliptic_curve::{
        ff::{Field, PrimeField},
        group::{Group, GroupEncoding},
        ops::Reduce,
    },
    AffinePoint, CompressedPoint, FieldBytes, ProjectivePoint, Scalar,
};
use rand::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Length of a compressed secp256k1 point.
pub const POINT_LEN: usize = 33;
/// Length of a scalar.
pub const SCALAR_LEN: usize = 32;
/// Poly1305 tag length.
pub const TAG_LEN: usize = 16;
/// Bytes a [`Ciphertext`] adds to its plaintext (`U ‖ c ‖ z ‖ tag`).
pub const CIPHERTEXT_OVERHEAD: usize = POINT_LEN + 2 * SCALAR_LEN + TAG_LEN;
/// Largest committee [`deal`] accepts (share indices are `u32`, but larger committees
/// make no sense for a prototype and would only slow verification).
pub const MAX_COMMITTEE: u32 = 1024;

const DOMAIN_KDF: &[u8] = b"xdevs/tpke/kdf/v1";
const DOMAIN_POK: &[u8] = b"xdevs/tpke/pok/v1";
const DOMAIN_DLEQ: &[u8] = b"xdevs/tpke/dleq/v1";

/// Errors of the threshold scheme.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThresholdError {
    /// Not a valid compressed secp256k1 point (or the identity).
    #[error("invalid curve point")]
    InvalidPoint,
    /// Not a canonical non-zero scalar.
    #[error("invalid scalar")]
    InvalidScalar,
    /// Bad threshold / committee size.
    #[error("invalid parameters: {0}")]
    InvalidParameters(String),
    /// Malformed encoding.
    #[error("malformed: {0}")]
    Malformed(String),
    /// The ciphertext's proof of knowledge does not verify for this label.
    #[error("ciphertext proof does not verify")]
    InvalidCiphertext,
    /// A decryption share's DLEQ proof does not verify.
    #[error("decryption share {0} does not verify")]
    InvalidShare(u32),
    /// Share index 0, out of range or repeated.
    #[error("bad share index {0}")]
    BadShareIndex(u32),
    /// Fewer shares than the threshold.
    #[error("{have} decryption shares, {need} needed")]
    NotEnoughShares {
        /// Shares given.
        have: usize,
        /// Threshold.
        need: usize,
    },
    /// The AEAD rejected the body (wrong key, tampered body or label).
    #[error("decryption failed")]
    Decryption,
}

type Result<T> = std::result::Result<T, ThresholdError>;

fn hash_to_scalar(domain: &[u8], parts: &[&[u8]]) -> Scalar {
    let mut h = Sha256::new();
    h.update((domain.len() as u64).to_be_bytes());
    h.update(domain);
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p);
    }
    let digest = h.finalize();
    let mut bytes = FieldBytes::default();
    bytes.copy_from_slice(&digest);
    <Scalar as Reduce<k256::U256>>::reduce_bytes(&bytes)
}

fn encode_point(p: &ProjectivePoint) -> [u8; POINT_LEN] {
    let repr = p.to_affine().to_bytes();
    let mut out = [0u8; POINT_LEN];
    out.copy_from_slice(&repr);
    out
}

/// Decode a compressed point; the identity is refused.
fn decode_point(bytes: &[u8]) -> Result<ProjectivePoint> {
    if bytes.len() != POINT_LEN {
        return Err(ThresholdError::InvalidPoint);
    }
    let mut repr = CompressedPoint::default();
    repr.copy_from_slice(bytes);
    let affine: Option<AffinePoint> = AffinePoint::from_bytes(&repr).into();
    let point = ProjectivePoint::from(affine.ok_or(ThresholdError::InvalidPoint)?);
    if bool::from(point.is_identity()) {
        return Err(ThresholdError::InvalidPoint);
    }
    Ok(point)
}

fn decode_scalar(bytes: &[u8; SCALAR_LEN]) -> Result<Scalar> {
    let mut repr = FieldBytes::default();
    repr.copy_from_slice(bytes);
    Option::<Scalar>::from(Scalar::from_repr(repr)).ok_or(ThresholdError::InvalidScalar)
}

fn scalar_bytes(s: &Scalar) -> [u8; SCALAR_LEN] {
    let mut out = [0u8; SCALAR_LEN];
    out.copy_from_slice(&s.to_bytes());
    out
}

fn random_nonzero<R: RngCore + CryptoRng>(rng: &mut R) -> Scalar {
    loop {
        let s = Scalar::random(&mut *rng);
        if !bool::from(s.is_zero()) {
            return s;
        }
    }
}

/// A validated public point (committee key or a verification key).
#[derive(Clone, Copy, Debug)]
pub struct PublicPoint {
    bytes: [u8; POINT_LEN],
    point: ProjectivePoint,
}

impl PartialEq for PublicPoint {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Eq for PublicPoint {}

impl PublicPoint {
    /// Parse a 33-byte compressed point (the identity is refused).
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let point = decode_point(bytes)?;
        let mut b = [0u8; POINT_LEN];
        b.copy_from_slice(bytes);
        Ok(Self { bytes: b, point })
    }

    fn from_point(point: ProjectivePoint) -> Self {
        Self {
            bytes: encode_point(&point),
            point,
        }
    }

    /// The compressed encoding.
    pub fn as_bytes(&self) -> &[u8; POINT_LEN] {
        &self.bytes
    }
}

/// One committee member's secret share `(i, s_i)`. Zeroized on drop; `Debug` hides it.
#[derive(Clone)]
pub struct KeyShare {
    index: u32,
    secret: Zeroizing<Scalar>,
}

impl std::fmt::Debug for KeyShare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyShare")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl KeyShare {
    /// Share index `i` (1-based: verification key `i - 1` of the committee).
    pub fn index(&self) -> u32 {
        self.index
    }

    /// `Y_i = s_i·G`.
    pub fn verification_key(&self) -> PublicPoint {
        PublicPoint::from_point(ProjectivePoint::GENERATOR * *self.secret)
    }

    /// Text form `<index>:<64 hex digits>` (the validator's share file).
    pub fn to_text(&self) -> Zeroizing<String> {
        let mut hex = String::with_capacity(2 * SCALAR_LEN);
        for b in scalar_bytes(&self.secret) {
            hex.push_str(&format!("{b:02x}"));
        }
        Zeroizing::new(format!("{}:{hex}", self.index))
    }

    /// Inverse of [`KeyShare::to_text`] (surrounding whitespace and a `0x` prefix on the
    /// scalar are accepted).
    pub fn from_text(text: &str) -> Result<Self> {
        let (index, hex) = text
            .trim()
            .split_once(':')
            .ok_or_else(|| ThresholdError::Malformed("expected <index>:<hex>".into()))?;
        let index: u32 = index
            .trim()
            .parse()
            .map_err(|_| ThresholdError::Malformed("share index".into()))?;
        if index == 0 {
            return Err(ThresholdError::BadShareIndex(0));
        }
        let hex = hex.trim().trim_start_matches("0x");
        if hex.len() != 2 * SCALAR_LEN {
            return Err(ThresholdError::Malformed("share scalar length".into()));
        }
        let mut bytes = Zeroizing::new([0u8; SCALAR_LEN]);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
                .map_err(|_| ThresholdError::Malformed("share scalar hex".into()))?;
        }
        let secret = decode_scalar(&bytes)?;
        if bool::from(secret.is_zero()) {
            return Err(ThresholdError::InvalidScalar);
        }
        Ok(Self {
            index,
            secret: Zeroizing::new(secret),
        })
    }

    /// This member's decryption share of `ct` with its DLEQ proof. The caller must have
    /// checked the ciphertext ([`Ciphertext::verify`]) first: releasing shares for an
    /// unchecked ciphertext is what the proof of knowledge exists to prevent.
    pub fn decryption_share<R: RngCore + CryptoRng>(
        &self,
        ct: &Ciphertext,
        rng: &mut R,
    ) -> Result<DecryptionShare> {
        let u = decode_point(&ct.u)?;
        let s = *self.secret;
        let d = u * s;
        let y_i = ProjectivePoint::GENERATOR * s;
        let w = Zeroizing::new(random_nonzero(rng));
        let a1 = ProjectivePoint::GENERATOR * *w;
        let a2 = u * *w;
        let d_bytes = encode_point(&d);
        let c = dleq_challenge(&encode_point(&y_i), &ct.u, &d_bytes, &a1, &a2);
        let z = *w + c * s;
        Ok(DecryptionShare {
            index: self.index,
            d: d_bytes,
            c: scalar_bytes(&c),
            z: scalar_bytes(&z),
        })
    }

    /// [`KeyShare::decryption_share`] with the OS RNG.
    pub fn decryption_share_random(&self, ct: &Ciphertext) -> Result<DecryptionShare> {
        self.decryption_share(ct, &mut rand::rngs::OsRng)
    }
}

fn dleq_challenge(
    y_i: &[u8],
    u: &[u8],
    d: &[u8],
    a1: &ProjectivePoint,
    a2: &ProjectivePoint,
) -> Scalar {
    hash_to_scalar(
        DOMAIN_DLEQ,
        &[y_i, u, d, &encode_point(a1), &encode_point(a2)],
    )
}

/// Output of the trusted dealer ([`deal`]).
#[derive(Clone, Debug)]
pub struct Dealt {
    /// Committee public key `Y = s·G`.
    pub public_key: PublicPoint,
    /// `Y_i`, index `i` at position `i - 1`.
    pub verification_keys: Vec<PublicPoint>,
    /// `t`: shares needed to decrypt.
    pub threshold: u32,
    /// The `n` secret shares (index `i` at position `i - 1`).
    pub shares: Vec<KeyShare>,
}

/// Trusted-dealer key generation: a random degree `t - 1` polynomial `f`, `s = f(0)`,
/// share `i` = `f(i)` for `i = 1..=n`. The dealer learns `s`; a DKG replaces this before
/// any real use (see the research doc).
pub fn deal<R: RngCore + CryptoRng>(threshold: u32, n: u32, rng: &mut R) -> Result<Dealt> {
    if threshold == 0 || threshold > n || n > MAX_COMMITTEE {
        return Err(ThresholdError::InvalidParameters(format!(
            "threshold {threshold} of {n} (need 1 <= t <= n <= {MAX_COMMITTEE})"
        )));
    }
    let coeffs: Vec<Zeroizing<Scalar>> = (0..threshold)
        .map(|_| Zeroizing::new(random_nonzero(rng)))
        .collect();
    let eval = |x: u32| -> Scalar {
        // Horner: f(x) = c0 + x(c1 + x(c2 + …)).
        let x = Scalar::from(u64::from(x));
        let mut acc = Scalar::ZERO;
        for c in coeffs.iter().rev() {
            acc = acc * x + **c;
        }
        acc
    };
    let mut shares = Vec::with_capacity(n as usize);
    for i in 1..=n {
        let s_i = eval(i);
        if bool::from(s_i.is_zero()) {
            // Probability ~2^-256; retry with fresh coefficients.
            return deal(threshold, n, rng);
        }
        shares.push(KeyShare {
            index: i,
            secret: Zeroizing::new(s_i),
        });
    }
    let public_key = PublicPoint::from_point(ProjectivePoint::GENERATOR * *coeffs[0]);
    let verification_keys = shares.iter().map(KeyShare::verification_key).collect();
    Ok(Dealt {
        public_key,
        verification_keys,
        threshold,
        shares,
    })
}

/// [`deal`] with the OS RNG.
pub fn deal_random(threshold: u32, n: u32) -> Result<Dealt> {
    deal(threshold, n, &mut rand::rngs::OsRng)
}

/// A threshold ciphertext: `U = r·G`, the proof of knowledge `(c, z)` of `r` and the
/// AEAD body (plaintext + 16-byte tag).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ciphertext {
    /// `U = r·G` (compressed).
    pub u: [u8; POINT_LEN],
    /// Proof-of-knowledge challenge.
    pub c: [u8; SCALAR_LEN],
    /// Proof-of-knowledge response.
    pub z: [u8; SCALAR_LEN],
    /// ChaCha20-Poly1305 output.
    pub body: Vec<u8>,
}

impl Ciphertext {
    /// `U ‖ c ‖ z ‖ body`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(POINT_LEN + 2 * SCALAR_LEN + self.body.len());
        out.extend_from_slice(&self.u);
        out.extend_from_slice(&self.c);
        out.extend_from_slice(&self.z);
        out.extend_from_slice(&self.body);
        out
    }

    /// Inverse of [`Ciphertext::to_bytes`] (structure only; see [`Ciphertext::verify`]).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < CIPHERTEXT_OVERHEAD {
            return Err(ThresholdError::Malformed(format!(
                "ciphertext of {} bytes, at least {CIPHERTEXT_OVERHEAD}",
                bytes.len()
            )));
        }
        let mut u = [0u8; POINT_LEN];
        let mut c = [0u8; SCALAR_LEN];
        let mut z = [0u8; SCALAR_LEN];
        u.copy_from_slice(&bytes[..POINT_LEN]);
        c.copy_from_slice(&bytes[POINT_LEN..POINT_LEN + SCALAR_LEN]);
        z.copy_from_slice(&bytes[POINT_LEN + SCALAR_LEN..POINT_LEN + 2 * SCALAR_LEN]);
        Ok(Self {
            u,
            c,
            z,
            body: bytes[POINT_LEN + 2 * SCALAR_LEN..].to_vec(),
        })
    }

    /// Check the proof of knowledge of `r` for `label`: `A = z·G - c·U`,
    /// `c == H(U, A, label, body)`. Committee members release decryption shares only for
    /// ciphertexts that pass.
    pub fn verify(&self, label: &[u8]) -> Result<()> {
        let u = decode_point(&self.u).map_err(|_| ThresholdError::InvalidCiphertext)?;
        let c = decode_scalar(&self.c).map_err(|_| ThresholdError::InvalidCiphertext)?;
        let z = decode_scalar(&self.z).map_err(|_| ThresholdError::InvalidCiphertext)?;
        let a = ProjectivePoint::GENERATOR * z - u * c;
        let expected = hash_to_scalar(DOMAIN_POK, &[&self.u, &encode_point(&a), label, &self.body]);
        if expected == c {
            Ok(())
        } else {
            Err(ThresholdError::InvalidCiphertext)
        }
    }
}

/// HKDF-SHA256(salt = domain, ikm = K, info = U ‖ label) → (key, nonce).
fn derive_key(k: &ProjectivePoint, u: &[u8], label: &[u8]) -> Zeroizing<[u8; 44]> {
    let ikm = Zeroizing::new(encode_point(k));
    let hk = Hkdf::<Sha256>::new(Some(DOMAIN_KDF), &ikm[..]);
    let mut info = Vec::with_capacity(u.len() + label.len());
    info.extend_from_slice(u);
    info.extend_from_slice(label);
    let mut okm = Zeroizing::new([0u8; 44]);
    hk.expand(&info, &mut okm[..])
        .expect("44 bytes is a valid HKDF-SHA256 output length");
    okm
}

fn aead_parts(okm: &[u8; 44]) -> (ChaCha20Poly1305, Nonce) {
    let cipher =
        ChaCha20Poly1305::new_from_slice(&okm[..32]).expect("a 32-byte ChaCha20-Poly1305 key");
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&okm[32..]);
    (cipher, Nonce::from(nonce))
}

fn aead_seal(okm: &[u8; 44], label: &[u8], msg: &[u8]) -> Vec<u8> {
    let (cipher, nonce) = aead_parts(okm);
    cipher
        .encrypt(&nonce, Payload { msg, aad: label })
        .expect("ChaCha20-Poly1305 encryption of an in-memory buffer cannot fail")
}

fn aead_open(okm: &[u8; 44], label: &[u8], body: &[u8]) -> Result<Vec<u8>> {
    let (cipher, nonce) = aead_parts(okm);
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: body,
                aad: label,
            },
        )
        .map_err(|_| ThresholdError::Decryption)
}

/// Encrypt `msg` to the committee key `pk` under `label` (context bound into both the
/// AEAD and the proof of knowledge).
pub fn encrypt<R: RngCore + CryptoRng>(
    pk: &PublicPoint,
    label: &[u8],
    msg: &[u8],
    rng: &mut R,
) -> Ciphertext {
    let r = Zeroizing::new(random_nonzero(rng));
    let u_point = ProjectivePoint::GENERATOR * *r;
    let u = encode_point(&u_point);
    let k = pk.point * *r;
    let okm = derive_key(&k, &u, label);
    let body = aead_seal(&okm, label, msg);
    let w = Zeroizing::new(random_nonzero(rng));
    let a = ProjectivePoint::GENERATOR * *w;
    let c = hash_to_scalar(DOMAIN_POK, &[&u, &encode_point(&a), label, &body]);
    let z = *w + c * *r;
    Ciphertext {
        u,
        c: scalar_bytes(&c),
        z: scalar_bytes(&z),
        body,
    }
}

/// [`encrypt`] with the OS RNG.
pub fn encrypt_random(pk: &PublicPoint, label: &[u8], msg: &[u8]) -> Ciphertext {
    encrypt(pk, label, msg, &mut rand::rngs::OsRng)
}

/// A decryption share `D_i = s_i·U` with its DLEQ proof `(c, z)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecryptionShare {
    /// Share index `i` (1-based).
    pub index: u32,
    /// `D_i` (compressed).
    pub d: [u8; POINT_LEN],
    /// DLEQ challenge.
    pub c: [u8; SCALAR_LEN],
    /// DLEQ response.
    pub z: [u8; SCALAR_LEN],
}

impl DecryptionShare {
    /// Verify the DLEQ proof against member `index`'s verification key `vk` and `ct`:
    /// `A1 = z·G - c·Y_i`, `A2 = z·U - c·D_i`, `c == H(Y_i, U, D_i, A1, A2)`.
    pub fn verify(&self, vk: &PublicPoint, ct: &Ciphertext) -> Result<()> {
        let bad = ThresholdError::InvalidShare(self.index);
        let u = decode_point(&ct.u).map_err(|_| bad.clone())?;
        let d = decode_point(&self.d).map_err(|_| bad.clone())?;
        let c = decode_scalar(&self.c).map_err(|_| bad.clone())?;
        let z = decode_scalar(&self.z).map_err(|_| bad.clone())?;
        let a1 = ProjectivePoint::GENERATOR * z - vk.point * c;
        let a2 = u * z - d * c;
        if dleq_challenge(&vk.bytes, &ct.u, &self.d, &a1, &a2) == c {
            Ok(())
        } else {
            Err(bad)
        }
    }
}

/// Lagrange coefficient at 0 of index `i` over `indices`.
fn lagrange_at_zero(i: u32, indices: &[u32]) -> Result<Scalar> {
    let xi = Scalar::from(u64::from(i));
    let mut num = Scalar::ONE;
    let mut den = Scalar::ONE;
    for &j in indices {
        if j == i {
            continue;
        }
        let xj = Scalar::from(u64::from(j));
        num *= xj;
        den *= xj - xi;
    }
    let inv: Option<Scalar> = den.invert().into();
    Ok(num * inv.ok_or(ThresholdError::BadShareIndex(i))?)
}

/// Combine the first `threshold` of `shares` (distinct non-zero indices; the caller has
/// verified each one with [`DecryptionShare::verify`]) and open `ct` under `label`.
pub fn combine(
    threshold: u32,
    shares: &[DecryptionShare],
    ct: &Ciphertext,
    label: &[u8],
) -> Result<Vec<u8>> {
    let t = threshold as usize;
    if t == 0 {
        return Err(ThresholdError::InvalidParameters("threshold 0".into()));
    }
    if shares.len() < t {
        return Err(ThresholdError::NotEnoughShares {
            have: shares.len(),
            need: t,
        });
    }
    let used = &shares[..t];
    let indices: Vec<u32> = used.iter().map(|s| s.index).collect();
    for (k, &i) in indices.iter().enumerate() {
        if i == 0 || indices[..k].contains(&i) {
            return Err(ThresholdError::BadShareIndex(i));
        }
    }
    let mut k = ProjectivePoint::IDENTITY;
    for s in used {
        let d = decode_point(&s.d).map_err(|_| ThresholdError::InvalidShare(s.index))?;
        k += d * lagrange_at_zero(s.index, &indices)?;
    }
    let okm = derive_key(&k, &ct.u, label);
    aead_open(&okm, label, &ct.body)
}

/// Decrypt with the full secret (tests and tooling only: nobody holds `s` in a real
/// deployment).
#[cfg(test)]
fn decrypt_with_secret(s: &Scalar, ct: &Ciphertext, label: &[u8]) -> Result<Vec<u8>> {
    let u = decode_point(&ct.u)?;
    let okm = derive_key(&(u * *s), &ct.u, label);
    aead_open(&okm, label, &ct.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> rand::rngs::OsRng {
        rand::rngs::OsRng
    }

    fn secret_from(d: &Dealt) -> Scalar {
        // Reconstruct s from the first t shares (test only).
        let idx: Vec<u32> = d.shares[..d.threshold as usize]
            .iter()
            .map(|s| s.index)
            .collect();
        d.shares[..d.threshold as usize]
            .iter()
            .map(|s| *s.secret * lagrange_at_zero(s.index, &idx).unwrap())
            .fold(Scalar::ZERO, |a, b| a + b)
    }

    #[test]
    fn encrypt_and_threshold_decrypt_round_trip() {
        let d = deal(3, 5, &mut rng()).unwrap();
        assert_eq!(d.verification_keys.len(), 5);
        let label = b"label";
        let msg = b"swap 100 XDS for TOKEN".to_vec();
        let ct = encrypt(&d.public_key, label, &msg, &mut rng());
        assert_eq!(ct.body.len(), msg.len() + TAG_LEN);
        ct.verify(label).unwrap();
        assert_eq!(Ciphertext::from_bytes(&ct.to_bytes()).unwrap(), ct);

        // Every t-subset decrypts; shares verify against their own key only.
        let shares: Vec<DecryptionShare> = d
            .shares
            .iter()
            .map(|s| s.decryption_share(&ct, &mut rng()).unwrap())
            .collect();
        for (i, s) in shares.iter().enumerate() {
            s.verify(&d.verification_keys[i], &ct).unwrap();
            let other = &d.verification_keys[(i + 1) % 5];
            assert_eq!(
                s.verify(other, &ct),
                Err(ThresholdError::InvalidShare(s.index))
            );
        }
        for subset in [[0, 1, 2], [2, 3, 4], [4, 0, 2], [1, 3, 4]] {
            let pick: Vec<_> = subset.iter().map(|&k| shares[k]).collect();
            assert_eq!(combine(3, &pick, &ct, label).unwrap(), msg);
        }
        // The dealer's secret matches the public key.
        let s = secret_from(&d);
        assert_eq!(
            PublicPoint::from_point(ProjectivePoint::GENERATOR * s),
            d.public_key
        );
        assert_eq!(decrypt_with_secret(&s, &ct, label).unwrap(), msg);
    }

    #[test]
    fn fewer_than_threshold_or_wrong_label_fails() {
        let d = deal(3, 4, &mut rng()).unwrap();
        let ct = encrypt(&d.public_key, b"l", b"secret", &mut rng());
        let shares: Vec<_> = d.shares[..2]
            .iter()
            .map(|s| s.decryption_share(&ct, &mut rng()).unwrap())
            .collect();
        assert_eq!(
            combine(3, &shares, &ct, b"l"),
            Err(ThresholdError::NotEnoughShares { have: 2, need: 3 })
        );
        // Two shares with a forged third index interpolate to the wrong key.
        let mut forged = shares.clone();
        let mut third = shares[0];
        third.index = 3;
        forged.push(third);
        assert_eq!(
            combine(3, &forged, &ct, b"l"),
            Err(ThresholdError::Decryption)
        );
        // Duplicate indices are refused.
        let dup = vec![shares[0], shares[1], shares[0]];
        assert_eq!(
            combine(3, &dup, &ct, b"l"),
            Err(ThresholdError::BadShareIndex(shares[0].index))
        );
        // Right shares, wrong label: the AEAD (and the proof) refuse.
        let all: Vec<_> = d.shares[..3]
            .iter()
            .map(|s| s.decryption_share(&ct, &mut rng()).unwrap())
            .collect();
        assert_eq!(
            combine(3, &all, &ct, b"other"),
            Err(ThresholdError::Decryption)
        );
        assert_eq!(ct.verify(b"other"), Err(ThresholdError::InvalidCiphertext));
    }

    #[test]
    fn tampering_is_detected() {
        let d = deal(2, 3, &mut rng()).unwrap();
        let ct = encrypt(&d.public_key, b"l", b"payload", &mut rng());
        // Body bit flip: proof fails, and the AEAD would too.
        let mut t = ct.clone();
        t.body[0] ^= 1;
        assert_eq!(t.verify(b"l"), Err(ThresholdError::InvalidCiphertext));
        let shares: Vec<_> = d.shares[..2]
            .iter()
            .map(|s| s.decryption_share(&t, &mut rng()).unwrap())
            .collect();
        assert_eq!(
            combine(2, &shares, &t, b"l"),
            Err(ThresholdError::Decryption)
        );
        // Copying U (and the proof) into a new ciphertext under another label fails.
        let mut copy = ct.clone();
        copy.body = encrypt(&d.public_key, b"m", b"payload", &mut rng()).body;
        assert_eq!(copy.verify(b"m"), Err(ThresholdError::InvalidCiphertext));
        // A share with a tampered D or response does not verify.
        let mut s = d.shares[0].decryption_share(&ct, &mut rng()).unwrap();
        s.verify(&d.verification_keys[0], &ct).unwrap();
        s.z[31] ^= 1;
        assert!(s.verify(&d.verification_keys[0], &ct).is_err());
        let mut s2 = d.shares[0].decryption_share(&ct, &mut rng()).unwrap();
        s2.d = d.shares[1].decryption_share(&ct, &mut rng()).unwrap().d;
        assert!(s2.verify(&d.verification_keys[0], &ct).is_err());
        // Garbage points / short buffers.
        assert!(Ciphertext::from_bytes(&[0u8; 10]).is_err());
        let mut g = ct.clone();
        g.u = [0u8; POINT_LEN];
        assert_eq!(g.verify(b"l"), Err(ThresholdError::InvalidCiphertext));
        assert!(PublicPoint::from_slice(&[0u8; POINT_LEN]).is_err());
        assert!(PublicPoint::from_slice(&[2u8; 5]).is_err());
    }

    #[test]
    fn share_text_round_trip_and_params() {
        let d = deal(1, 1, &mut rng()).unwrap();
        let text = d.shares[0].to_text();
        let back = KeyShare::from_text(&text).unwrap();
        assert_eq!(back.index(), 1);
        assert_eq!(back.verification_key(), d.verification_keys[0]);
        assert!(format!("{back:?}").contains("index"));
        assert!(!format!("{back:?}").contains(&text[2..10]));
        assert!(KeyShare::from_text("0:00").is_err());
        assert!(KeyShare::from_text("1:zz").is_err());
        assert!(deal(0, 3, &mut rng()).is_err());
        assert!(deal(4, 3, &mut rng()).is_err());
        // t = 1: any single share decrypts.
        let ct = encrypt(&d.public_key, b"", b"x", &mut rng());
        let s = d.shares[0].decryption_share(&ct, &mut rng()).unwrap();
        assert_eq!(combine(1, &[s], &ct, b"").unwrap(), b"x");
    }
}

//! Crypto primitives. All keys are 32 bytes and wrapped in `Zeroizing`.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rand::{CryptoRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{Error, Result};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

pub type Key = Zeroizing<[u8; KEY_LEN]>;

/// Argon2id parameters, stored in the password slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    /// Default for new vaults (see benchmark in docs/spikes/vault-format.md).
    pub const DEFAULT: Self = Self {
        m_kib: 128 * 1024,
        t: 3,
        p: 4,
    };

    /// Bounds enforced when *reading* a file, so a tampered header cannot
    /// make unlock trivially weak or exhaust memory.
    pub fn validate(&self) -> Result<()> {
        let ok = (8 * 1024..=2 * 1024 * 1024).contains(&self.m_kib)
            && (1..=20).contains(&self.t)
            && (1..=16).contains(&self.p);
        if ok {
            Ok(())
        } else {
            Err(Error::BadKdfParams)
        }
    }

    /// True when `self` costs an attacker less than `other` (lower memory or fewer passes).
    /// Lanes (`p`) are ignored: they change parallelism, not the total work.
    pub fn is_weaker_than(&self, other: &KdfParams) -> bool {
        self.m_kib < other.m_kib || self.t < other.t
    }
}

pub fn random_key(rng: &mut (impl RngCore + CryptoRng)) -> Key {
    let mut k = Zeroizing::new([0u8; KEY_LEN]);
    rng.fill_bytes(k.as_mut());
    k
}

pub fn random_bytes<const N: usize>(rng: &mut (impl RngCore + CryptoRng)) -> [u8; N] {
    let mut b = [0u8; N];
    rng.fill_bytes(&mut b);
    b
}

/// Argon2id(password, salt) -> 32-byte KEK. Bounds are NOT checked here so tests
/// can use tiny parameters; callers reading files must call `KdfParams::validate`.
pub fn kek_from_password(password: &[u8], salt: &[u8], params: KdfParams) -> Result<Key> {
    let p = Params::new(params.m_kib, params.t, params.p, Some(KEY_LEN)).map_err(|_| Error::BadKdfParams)?;
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
        .hash_password_into(password, salt, out.as_mut())
        .map_err(|_| Error::BadKdfParams)?;
    Ok(out)
}

/// HKDF-SHA256 for high-entropy inputs (recovery key, Hello signature).
pub fn kek_from_secret(ikm: &[u8], salt: &[u8], info: &[u8]) -> Key {
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, out.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

pub fn seal(key: &[u8; KEY_LEN], nonce: &[u8; NONCE_LEN], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(nonce), Payload { msg: plaintext, aad })
        .expect("XChaCha20-Poly1305 encryption does not fail for in-memory buffers")
}

/// Returns `None` on tag mismatch; the caller maps it to the right error.
pub fn open(key: &[u8; KEY_LEN], nonce: &[u8; NONCE_LEN], ciphertext: &[u8], aad: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad })
        .ok()
        .map(Zeroizing::new)
}

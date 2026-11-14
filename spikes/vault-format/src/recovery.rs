//! Recovery key: 32 random bytes + 2-byte SHA-256 checksum, Base32 (RFC 4648,
//! no padding), grouped by 5 characters for printing in the Emergency Kit.
//!
//! Example shape: `ABCDE-FGHIJ-...` (55 characters + 10 dashes).
//! The key has 256 bits of entropy, so HKDF (not Argon2) is enough to derive its KEK.

use data_encoding::BASE32_NOPAD;
use rand::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    crypto::{random_key, Key, KEY_LEN},
    Error, Result,
};

const GROUP: usize = 5;

/// Generates a new recovery secret and its printable form.
pub fn generate(rng: &mut (impl RngCore + CryptoRng)) -> (Key, Zeroizing<String>) {
    let secret = random_key(rng);
    let text = encode(&secret);
    (secret, text)
}

pub fn encode(secret: &[u8; KEY_LEN]) -> Zeroizing<String> {
    let mut raw = Zeroizing::new(Vec::with_capacity(KEY_LEN + 2));
    raw.extend_from_slice(secret);
    raw.extend_from_slice(&Sha256::digest(secret)[..2]);
    let b32 = Zeroizing::new(BASE32_NOPAD.encode(&raw));
    let grouped = b32
        .as_bytes()
        .chunks(GROUP)
        .map(|c| std::str::from_utf8(c).expect("base32 is ASCII"))
        .collect::<Vec<_>>()
        .join("-");
    Zeroizing::new(grouped)
}

/// Parses user input: case-insensitive, ignores spaces and dashes,
/// and maps the commonly confused `0`/`1`/`8` to `O`/`I`/`B`.
pub fn decode(input: &str) -> Result<Key> {
    let cleaned: Zeroizing<String> = Zeroizing::new(
        input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| match c.to_ascii_uppercase() {
                '0' => 'O',
                '1' => 'I',
                '8' => 'B',
                u => u,
            })
            .collect(),
    );
    let raw = Zeroizing::new(BASE32_NOPAD.decode(cleaned.as_bytes()).map_err(|_| Error::BadRecoveryKey)?);
    if raw.len() != KEY_LEN + 2 {
        return Err(Error::BadRecoveryKey);
    }
    let (secret, checksum) = raw.split_at(KEY_LEN);
    if Sha256::digest(secret)[..2] != *checksum {
        return Err(Error::BadRecoveryKey);
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(secret);
    Ok(key)
}

//! Fuzz entry points (v0.6 plan H5), shared by the `cargo-fuzz` targets in
//! `crates/xpass-core/fuzz` and by the replay test below, which runs every seed
//! and regression input on the stable toolchain in normal CI.
//!
//! Each function must never panic for any input, except on a broken invariant
//! it asserts on purpose. The password path is not fuzzed: a valid header asks
//! Argon2 for up to 2 GiB per input; unit tests cover it.
//!
//! Only compiled for tests and with the `fuzzing` feature.

use std::sync::OnceLock;

use rand::{rngs::StdRng, SeedableRng};

use crate::{crypto::KdfParams, format, model::Entry, otp, recovery, vault, Unlock, UnlockedVault};

/// Stand-in for a Windows Hello signature (RSA-2048 = 256 bytes).
pub const HELLO_SIGNATURE: [u8; 256] = [0x5A; 256];

/// Smallest KDF accepted when reading a file; only used to build the seed vault.
const SEED_KDF: KdfParams = KdfParams {
    m_kib: 8 * 1024,
    t: 1,
    p: 1,
};

/// Parses a vault file; a header that parses must encode back to the same bytes,
/// because those bytes are the payload's AAD.
pub fn parse(data: &[u8]) {
    if let Ok(file) = format::parse(data) {
        assert_eq!(file.header.encode(), file.header_bytes, "header does not round-trip");
    }
}

/// Unlocks with the Hello slot and opens the input as a sibling of the seed
/// vault: slot selection, AAD building and both AEAD opens.
pub fn unlock_hello(data: &[u8]) {
    let _ = UnlockedVault::unlock(
        data,
        Unlock::Hello {
            signature: &HELLO_SIGNATURE,
        },
    );
    let _ = seed().vault.open_sibling(data);
    let _ = vault::hello_challenge_for_file(data);
}

/// Decodes a typed recovery key; a key that decodes must encode and decode again
/// to the same bytes.
pub fn recovery_decode(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(key) = recovery::decode(text) {
        let again = recovery::decode(&recovery::encode(&key)).expect("encoded key decodes");
        assert_eq!(*again, *key, "recovery key does not round-trip");
    }
}

/// Parses a TOTP secret (URI or base32, from imports and paste, v0.8 plan V3); one
/// that parses must give a code of the right length and survive its own URI.
pub fn otp_parse(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(totp) = otp::Totp::parse(text) {
        let code = totp.code_at(1_700_000_000_000);
        assert_eq!(code.code.len(), usize::from(totp.digits), "code length");
        assert!(code.remaining_ms > 0 && code.remaining_ms <= u64::from(totp.period) * 1000);
        let again = otp::Totp::parse(&totp.to_uri()).expect("normalized URI parses");
        assert_eq!(again, totp, "TOTP does not round-trip through its URI");
    }
}

struct Seed {
    vault: UnlockedVault,
    /// Files: password + recovery slots, then with a Hello slot.
    files: [Vec<u8>; 2],
    recovery_text: String,
}

/// Deterministic seed vault (fixed RNG seed), built once per process.
fn seed() -> &'static Seed {
    static SEED: OnceLock<Seed> = OnceLock::new();
    SEED.get_or_init(|| {
        let mut rng = StdRng::seed_from_u64(0x5eed);
        let (mut v, recovery_text) =
            UnlockedVault::create(&mut rng, b"fuzz-seed-password", SEED_KDF).expect("seed vault");
        let mut entry = Entry::new("Example", 1_700_000_000_000);
        entry.id = uuid::Uuid::from_u128(1); // `Entry::new` picks a random id
        entry.username = "user@example.com".into();
        entry.password = "Sample-Pass-01".into();
        entry.urls = vec!["https://example.com".into()];
        v.payload.entries.push(entry);
        let without_hello = v.save(&mut rng).expect("save seed");
        let salt = UnlockedVault::new_hello_salt(&mut rng);
        v.set_hello_slot(&mut rng, salt, &HELLO_SIGNATURE);
        let with_hello = v.save(&mut rng).expect("save seed");
        Seed {
            vault: v,
            files: [without_hello, with_hello],
            recovery_text: recovery_text.to_string(),
        }
    })
}

/// Seed inputs per target: `(target, file name, bytes)`. Written to
/// `fuzz/seeds/<target>/` by `cargo run -p xpass-core --example fuzz_seeds --features fuzzing`.
pub fn seed_inputs() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let s = seed();
    let [plain, hello] = &s.files;
    let spaced = s.recovery_text.to_lowercase().replace('-', " ");
    vec![
        ("parse", "vault-password-recovery.xpv", plain.clone()),
        ("parse", "vault-hello.xpv", hello.clone()),
        ("unlock_hello", "vault-hello.xpv", hello.clone()),
        ("unlock_hello", "vault-password-recovery.xpv", plain.clone()),
        ("recovery_decode", "key.txt", s.recovery_text.clone().into_bytes()),
        ("recovery_decode", "key-lower-spaced.txt", spaced.into_bytes()),
        (
            "otp_parse",
            "uri-bitwarden.txt",
            b"otpauth://totp/Example:user%40example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Example".to_vec(),
        ),
        (
            "otp_parse",
            "uri-sha512-8-60.txt",
            b"otpauth://totp/ACME%20Co:alice?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=ACME%20Co&algorithm=SHA512&digits=8&period=60".to_vec(),
        ),
        ("otp_parse", "base32-keepassxc.txt", b"jbsw y3dp ehpk 3pxp jbsw y3dp ehpk 3pxp".to_vec()),
    ]
}

/// Runs a target by name (used by the replay test).
pub fn run(target: &str, data: &[u8]) {
    match target {
        "parse" => parse(data),
        "unlock_hello" => unlock_hello(data),
        "recovery_decode" => recovery_decode(data),
        "otp_parse" => otp_parse(data),
        _ => panic!("unknown fuzz target {target}"),
    }
}

pub const TARGETS: [&str; 4] = ["parse", "unlock_hello", "recovery_decode", "otp_parse"];

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    #[test]
    fn seed_vault_unlocks_with_the_fixed_signature() {
        let hello = &seed().files[1];
        let v = UnlockedVault::unlock(
            hello,
            Unlock::Hello {
                signature: &HELLO_SIGNATURE,
            },
        )
        .unwrap();
        assert_eq!(v.vault_id(), seed().vault.vault_id());
        assert!(seed().vault.open_sibling(&seed().files[0]).is_ok());
    }

    #[test]
    fn generated_seeds_pass_every_target() {
        for (target, _, data) in seed_inputs() {
            run(target, &data);
        }
    }

    /// Replays the checked-in seeds and every crash input added under
    /// `fuzz/regressions/<target>/` once fixed. Runs on stable in normal CI.
    #[test]
    fn replay_checked_in_inputs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz");
        let mut count = 0;
        for target in TARGETS {
            for dir in ["seeds", "regressions"] {
                let Ok(entries) = fs::read_dir(root.join(dir).join(target)) else {
                    continue;
                };
                for entry in entries {
                    let path = entry.unwrap().path();
                    if path.is_file() && path.file_name().is_some_and(|n| n != ".gitkeep") {
                        run(target, &fs::read(&path).unwrap());
                        count += 1;
                    }
                }
            }
        }
        assert!(
            count >= seed_inputs().len(),
            "seed files missing under {}",
            root.display()
        );
    }
}

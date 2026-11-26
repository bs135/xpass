//! Writes the published test vector for docs/FORMAT.md §10 to `docs/vectors/v1/`:
//! the golden vault file and `vector.json` with every input and intermediate value
//! needed to check an independent implementation (keys, salts, nonces, plaintext).
//!
//! `cargo run -p xpass-core --example format_vector`
//!
//! The vault is the one `tests/vault.rs` `golden_vector_v1` builds (same seed, password,
//! KDF and entries); `tests/vault.rs` `published_vector_matches` checks the files.
//! Everything here is synthetic: never put real data in a test vector.

use std::{fs, path::Path};

use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use serde_json::json;
use uuid::Uuid;
use xpass_core::{
    crypto::{self, KdfParams},
    format::{self, Header, SlotParams},
    model::Entry,
    recovery, Unlock, UnlockedVault,
};

const PASSWORD: &str = "correct horse battery staple";
const KDF: KdfParams = KdfParams {
    m_kib: 8 * 1024,
    t: 1,
    p: 1,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn entry(id: u128, title: &str, password: &str) -> Entry {
    let mut e = Entry::new(title, 1_700_000_000_000);
    e.id = Uuid::from_u128(id);
    e.username = "alice".into();
    e.password = password.into();
    e.urls = vec!["https://example.com".into()];
    e
}

fn main() -> std::io::Result<()> {
    let mut rng = ChaCha20Rng::seed_from_u64(42);
    let (mut v, recovery_text) = UnlockedVault::create(&mut rng, PASSWORD.as_bytes(), KDF).expect("create");
    v.payload.entries.push(entry(1, "GitHub", "gh-secret"));
    v.payload.entries.push(entry(2, "Mail", "mail-secret"));
    let file = v.save(&mut rng).expect("save");

    let parsed = format::parse(&file).expect("parse");
    let header = &parsed.header;
    let mut slots = Vec::new();
    let mut vault_key = None;
    for slot in &header.slots {
        let aad = Header::slot_aad(&header.vault_id, &slot.params);
        let (kind, params, kek) = match &slot.params {
            SlotParams::Password { kdf, salt } => (
                "password",
                json!({"m_kib": kdf.m_kib, "t": kdf.t, "p": kdf.p, "salt": hex(salt)}),
                crypto::kek_from_password(PASSWORD.as_bytes(), salt, *kdf).expect("argon2"),
            ),
            SlotParams::Recovery { salt } => {
                let secret = recovery::decode(&recovery_text).expect("recovery key");
                (
                    "recovery",
                    json!({"salt": hex(salt), "recovery_secret": hex(secret.as_ref())}),
                    crypto::kek_from_secret(secret.as_ref(), salt, b"xpass/kek-recovery/v1"),
                )
            }
            SlotParams::Hello { .. } => unreachable!("the vector has no Hello slot"),
        };
        let key = crypto::open(&kek, &slot.nonce, &slot.wrapped, &aad).expect("slot unwraps");
        vault_key.get_or_insert_with(|| hex(&key));
        slots.push(json!({
            "kind": kind,
            "params": params,
            "kek": hex(kek.as_ref()),
            "aad": hex(&aad),
            "nonce": hex(&slot.nonce),
            "wrapped": hex(&slot.wrapped),
        }));
    }
    let unlocked = UnlockedVault::unlock(&file, Unlock::Password(PASSWORD.as_bytes())).expect("unlock");
    let plaintext = serde_json::to_string(&unlocked.payload).expect("payload JSON");

    let vector = json!({
        "description": "xPass vault format v1 / payload schema 1 test vector (docs/FORMAT.md §10). Synthetic data.",
        "file": "vault.xpv",
        "file_length": file.len(),
        "file_sha256": hex(&<sha2::Sha256 as sha2::Digest>::digest(&file)),
        "password": PASSWORD,
        "recovery_key": recovery_text.as_str(),
        "vault_id": hex(&header.vault_id),
        "header_length": parsed.header_bytes.len(),
        "header": hex(parsed.header_bytes),
        "slots": slots,
        "vault_key": vault_key.expect("at least one slot"),
        "payload_nonce": hex(&header.payload_nonce),
        "payload_aad": "the header bytes above",
        "ciphertext_length": parsed.ciphertext.len(),
        "plaintext": plaintext,
    });

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/vectors/v1");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("vault.xpv"), &file)?;
    fs::write(
        dir.join("vector.json"),
        format!("{}\n", serde_json::to_string_pretty(&vector).expect("vector JSON")),
    )?;
    println!(
        "wrote {} ({} bytes) and vector.json",
        dir.join("vault.xpv").display(),
        file.len()
    );
    Ok(())
}

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xpass_core::{
    crypto::KdfParams,
    format::{self, SlotKind},
    model::{CustomField, Entry, HistoryItem, HISTORY_LIMIT},
    recovery,
    vault::{hello_challenge, hello_challenge_for_file},
    Error, Unlock, UnlockedVault,
};

/// Smallest parameters accepted by `KdfParams::validate`, to keep tests fast.
const TEST_KDF: KdfParams = KdfParams {
    m_kib: 8 * 1024,
    t: 1,
    p: 1,
};
const PW: &[u8] = b"correct horse battery staple";

fn rng(seed: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(seed)
}

fn entry(id: u128, title: &str, password: &str) -> Entry {
    let mut e = Entry::new(title, 1_700_000_000_000);
    e.id = Uuid::from_u128(id);
    e.username = "alice".into();
    e.password = password.into();
    e.urls = vec!["https://example.com".into()];
    e
}

/// New vault with two entries, saved once. Returns (file, recovery text).
fn sample(seed: u64) -> (Vec<u8>, String) {
    let mut r = rng(seed);
    let (mut v, rec) = UnlockedVault::create(&mut r, PW, TEST_KDF).unwrap();
    v.payload.entries.push(entry(1, "GitHub", "gh-secret"));
    v.payload.entries.push(entry(2, "Mail", "mail-secret"));
    (v.save(&mut r).unwrap(), rec.to_string())
}

fn err(r: Result<UnlockedVault, Error>) -> Error {
    r.expect_err("expected unlock to fail")
}

#[test]
fn roundtrip_with_password() {
    let (file, _) = sample(1);
    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    assert_eq!(v.payload.entries.len(), 2);
    assert_eq!(v.payload.entries[0].password, "gh-secret");
    assert_eq!(v.payload.revision, 1);
    assert_eq!(v.slot_kinds(), vec![SlotKind::Password, SlotKind::Recovery]);
}

#[test]
fn wrong_password_is_rejected() {
    let (file, _) = sample(2);
    assert_eq!(
        err(UnlockedVault::unlock(&file, Unlock::Password(b"nope"))),
        Error::BadCredentials
    );
}

#[test]
fn recovery_key_unlocks_and_tolerates_formatting() {
    let (file, rec) = sample(3);
    assert_eq!(rec.len(), 55 + 10, "55 base32 chars + 10 dashes");
    UnlockedVault::unlock(&file, Unlock::Recovery(&rec)).unwrap();

    let sloppy = rec.replace('-', " ").to_lowercase();
    UnlockedVault::unlock(&file, Unlock::Recovery(&sloppy)).unwrap();
}

#[test]
fn recovery_key_typo_is_caught_by_checksum() {
    let (file, rec) = sample(4);
    let mut chars: Vec<char> = rec.chars().collect();
    chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
    let typo: String = chars.into_iter().collect();
    assert_eq!(
        err(UnlockedVault::unlock(&file, Unlock::Recovery(&typo))),
        Error::BadRecoveryKey
    );
    assert_eq!(recovery::decode("ABCDE").unwrap_err(), Error::BadRecoveryKey);
}

#[test]
fn hello_slot_unlocks_with_signature() {
    let (file, _) = sample(5);
    let mut r = rng(50);
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    assert_eq!(hello_challenge_for_file(&file).unwrap_err(), Error::NoSuchSlot);

    // Enrollment: app signs the challenge with Windows Hello (faked here).
    let salt = UnlockedVault::new_hello_salt(&mut r);
    let challenge = hello_challenge(v.vault_id(), &salt);
    let signature = Sha256::digest(&challenge).repeat(8); // stand-in for a 256-byte RSA signature
    v.set_hello_slot(&mut r, salt, &signature);
    let file = v.save(&mut r).unwrap();

    // Unlock: read challenge from the locked file, sign, unlock.
    assert_eq!(hello_challenge_for_file(&file).unwrap(), challenge);
    let v2 = UnlockedVault::unlock(&file, Unlock::Hello { signature: &signature }).unwrap();
    assert_eq!(v2.vault_key_fingerprint(), v.vault_key_fingerprint());
    assert_eq!(
        err(UnlockedVault::unlock(&file, Unlock::Hello { signature: b"forged" })),
        Error::BadCredentials
    );
}

#[test]
fn every_saved_file_uses_a_fresh_nonce() {
    let (file, _) = sample(6);
    let mut r = rng(60);
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let a = v.save(&mut r).unwrap();
    let b = v.save(&mut r).unwrap();
    let na = format::parse(&a).unwrap().header.payload_nonce;
    let nb = format::parse(&b).unwrap().header.payload_nonce;
    assert_ne!(na, nb);
    assert_eq!(
        UnlockedVault::unlock(&b, Unlock::Password(PW))
            .unwrap()
            .payload
            .revision,
        3
    );
}

#[test]
fn any_single_byte_flip_is_detected() {
    let (file, rec) = sample(7);
    // Recovery unlock uses HKDF only, so the loop over every byte stays fast.
    // Every header byte is AAD for the payload, so flipping a byte in the
    // (unused) password slot must fail too.
    for i in 0..file.len() {
        let mut bad = file.clone();
        bad[i] ^= 0x01;
        assert!(
            UnlockedVault::unlock(&bad, Unlock::Recovery(&rec)).is_err(),
            "flip at byte {i} not detected"
        );
    }
}

#[test]
fn kdf_downgrade_is_rejected() {
    let (file, _) = sample(8);
    // Password slot params start at: magic 4 + version 2 + id 16 + cipher 1 + count 1 + kind 1 + len 2 = 27.
    let m_kib_at = 27;
    let mut weaker = file.clone();
    weaker[m_kib_at..m_kib_at + 4].copy_from_slice(&(8 * 1024 + 1u32).to_le_bytes());
    assert_eq!(
        err(UnlockedVault::unlock(&weaker, Unlock::Password(PW))),
        Error::BadCredentials
    );

    let mut absurd = file.clone();
    absurd[m_kib_at..m_kib_at + 4].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        err(UnlockedVault::unlock(&absurd, Unlock::Password(PW))),
        Error::BadKdfParams
    );
}

#[test]
fn version_and_magic_are_checked() {
    let (file, _) = sample(9);
    let mut v2 = file.clone();
    v2[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        err(UnlockedVault::unlock(&v2, Unlock::Password(PW))),
        Error::UnsupportedVersion(2)
    );

    let mut magic = file.clone();
    magic[0] = b'Z';
    assert_eq!(
        err(UnlockedVault::unlock(&magic, Unlock::Password(PW))),
        Error::Format("bad magic")
    );
}

#[test]
fn change_password_keeps_vault_key() {
    let (old_file, rec) = sample(10);
    let mut r = rng(100);
    let mut v = UnlockedVault::unlock(&old_file, Unlock::Password(PW)).unwrap();
    let fp_before = v.vault_key_fingerprint();
    v.change_password(&mut r, b"new password", TEST_KDF).unwrap();
    let new_file = v.save(&mut r).unwrap();

    assert_eq!(
        err(UnlockedVault::unlock(&new_file, Unlock::Password(PW))),
        Error::BadCredentials
    );
    UnlockedVault::unlock(&new_file, Unlock::Password(b"new password")).unwrap();
    UnlockedVault::unlock(&new_file, Unlock::Recovery(&rec)).unwrap();

    // Limitation (IDEA.md §3.3): the old file from git history still opens with
    // the old password and yields the same VaultKey that protects the new file.
    let from_history = UnlockedVault::unlock(&old_file, Unlock::Password(PW)).unwrap();
    assert_eq!(from_history.vault_key_fingerprint(), fp_before);
    assert_eq!(v.vault_key_fingerprint(), fp_before);
}

#[test]
fn rotate_replaces_vault_key_and_all_slots() {
    let (old_file, old_rec) = sample(11);
    let mut r = rng(110);
    let mut v = UnlockedVault::unlock(&old_file, Unlock::Password(PW)).unwrap();
    let salt = UnlockedVault::new_hello_salt(&mut r);
    v.set_hello_slot(&mut r, salt, b"sig");
    let fp_old = v.vault_key_fingerprint();

    let new_rec = v.rotate(&mut r, b"rotated password", TEST_KDF).unwrap();
    let new_file = v.save(&mut r).unwrap();

    assert_ne!(v.vault_key_fingerprint(), fp_old);
    assert_eq!(v.slot_kinds(), vec![SlotKind::Password, SlotKind::Recovery]);
    assert_eq!(
        err(UnlockedVault::unlock(&new_file, Unlock::Recovery(&old_rec))),
        Error::BadCredentials
    );
    assert_eq!(
        err(UnlockedVault::unlock(&new_file, Unlock::Hello { signature: b"sig" })),
        Error::NoSuchSlot
    );
    let reopened = UnlockedVault::unlock(&new_file, Unlock::Recovery(&new_rec)).unwrap();
    assert_eq!(reopened.payload.entries.len(), 2);
}

#[test]
fn delete_leaves_a_tombstone() {
    let (file, _) = sample(12);
    let mut r = rng(120);
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    v.payload.entries[0].delete(1_800_000_000_000);
    let file = v.save(&mut r).unwrap();

    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let t = &v.payload.entries[0];
    assert!(t.deleted);
    assert_eq!(t.id, Uuid::from_u128(1));
    assert_eq!(t.updated_at, 1_800_000_000_000);
    assert!(t.password.is_empty() && t.title.is_empty() && t.username.is_empty());
}

#[test]
fn parser_never_panics_on_garbage() {
    let (file, rec) = sample(13);
    for len in 0..file.len() {
        assert!(
            UnlockedVault::unlock(&file[..len], Unlock::Recovery(&rec)).is_err(),
            "truncation at {len}"
        );
    }
    let mut r = rng(130);
    for i in 0..5_000 {
        let mut buf = vec![0u8; (r.next_u32() % 600) as usize];
        r.fill_bytes(&mut buf);
        if i % 2 == 0 && buf.len() >= 6 {
            buf[..4].copy_from_slice(format::MAGIC); // get past the magic check
            buf[4..6].copy_from_slice(&format::VERSION.to_le_bytes());
        }
        let _ = format::parse(&buf);
        let _ = UnlockedVault::unlock(&buf, Unlock::Recovery(&rec));
    }
}

// ---- open_sibling (used by sync in v0.3) ----

#[test]
fn open_sibling_reads_another_copy_without_credentials() {
    let (file, _) = sample(14);
    let mut r = rng(140);
    let a = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let mut b = a.clone();
    b.payload.entries.push(entry(3, "Bank", "bank-secret"));
    b.change_password(&mut r, b"changed elsewhere", TEST_KDF).unwrap();
    let b_file = b.save(&mut r).unwrap();

    let theirs = a.open_sibling(&b_file).unwrap();
    assert_eq!(theirs.payload.entries.len(), 3);
    assert_eq!(theirs.vault_key_fingerprint(), a.vault_key_fingerprint());

    // The sibling adopts the other file's slots: saving it keeps the new password.
    let mut theirs = theirs;
    let merged = theirs.save(&mut r).unwrap();
    UnlockedVault::unlock(&merged, Unlock::Password(b"changed elsewhere")).unwrap();
    assert_eq!(
        err(UnlockedVault::unlock(&merged, Unlock::Password(PW))),
        Error::BadCredentials
    );
}

#[test]
fn open_sibling_rejects_a_different_vault() {
    let (file, _) = sample(15);
    let (other, _) = sample(16);
    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    assert_eq!(v.open_sibling(&other).unwrap_err(), Error::DifferentVault);
}

#[test]
fn open_sibling_reports_key_rotated_elsewhere() {
    let (file, _) = sample(17);
    let mut r = rng(170);
    let a = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let mut b = a.clone();
    b.rotate(&mut r, PW, TEST_KDF).unwrap();
    let b_file = b.save(&mut r).unwrap();
    assert_eq!(a.open_sibling(&b_file).unwrap_err(), Error::KeyChanged);
}

#[test]
fn open_sibling_detects_a_tampered_payload() {
    let (file, _) = sample(18);
    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let mut bad = file.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    assert_eq!(v.open_sibling(&bad).unwrap_err(), Error::Tampered);
}

// ---- D5: unknown fields survive ----

#[test]
fn unknown_fields_survive_unlock_and_save() {
    let (file, _) = sample(19);
    let mut r = rng(190);
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    // Simulates fields written by a newer build.
    v.payload
        .extra
        .insert("sync_hint".into(), serde_json::json!({"device": "laptop"}));
    v.payload.entries[0]
        .extra
        .insert("totp".into(), serde_json::json!("otpauth://totp/x"));
    let file = v.save(&mut r).unwrap();

    // An "older" session loads and saves without touching them.
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    v.payload.entries[1].title = "Mail (edited)".into();
    let file = v.save(&mut r).unwrap();

    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    assert_eq!(v.payload.extra["sync_hint"]["device"], "laptop");
    assert_eq!(v.payload.entries[0].extra["totp"], "otpauth://totp/x");
    assert_eq!(v.payload.entries[1].title, "Mail (edited)");
}

#[test]
fn unknown_fields_survive_json_roundtrip() {
    let json = r#"{
        "schema": 1, "revision": 7, "future_top": [1, 2],
        "entries": [{
            "id": "00000000-0000-0000-0000-000000000001",
            "created_at": 1, "updated_at": 2, "title": "T",
            "passkeys": [{"rp": "example.com"}]
        }]
    }"#;
    let p: xpass_core::model::Payload = serde_json::from_str(json).unwrap();
    assert_eq!(p.revision, 7);
    assert!(
        p.entries[0].urls.is_empty() && !p.entries[0].favorite,
        "missing known fields default"
    );
    let back = serde_json::to_value(&p).unwrap();
    assert_eq!(back["future_top"], serde_json::json!([1, 2]));
    assert_eq!(back["entries"][0]["passkeys"][0]["rp"], "example.com");
}

// ---- Model ----

#[test]
fn model_fields_roundtrip() {
    let (file, _) = sample(20);
    let mut r = rng(200);
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let e = &mut v.payload.entries[0];
    e.urls.push("https://github.com/login".into());
    e.favorite = true;
    e.fields.push(CustomField {
        name: "PIN".into(),
        value: "1234".into(),
        secret: true,
    });
    e.push_history(HistoryItem {
        at: 5,
        password: "old".into(),
        username: Some("bob".into()),
        note: None,
    });
    let file = v.save(&mut r).unwrap();

    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    let e = &v.payload.entries[0];
    assert_eq!(e.urls.len(), 2);
    assert!(e.favorite);
    assert_eq!(
        e.fields[0],
        CustomField {
            name: "PIN".into(),
            value: "1234".into(),
            secret: true
        }
    );
    assert_eq!(e.history[0].username.as_deref(), Some("bob"));
    assert_eq!(e.history[0].note, None);
}

#[test]
fn history_is_capped_and_drops_the_oldest() {
    let mut e = entry(1, "X", "p");
    for at in 0..(HISTORY_LIMIT as i64 + 5) {
        e.push_history(HistoryItem {
            at,
            password: format!("p{at}"),
            username: None,
            note: None,
        });
    }
    assert_eq!(e.history.len(), HISTORY_LIMIT);
    assert_eq!(e.history.first().unwrap().at, 5);
    assert_eq!(e.history.last().unwrap().at, HISTORY_LIMIT as i64 + 4);
}

#[test]
fn tombstone_wipes_history_fields_and_unknown_fields() {
    let mut e = entry(1, "X", "p");
    e.push_history(HistoryItem {
        at: 1,
        password: "old".into(),
        username: None,
        note: None,
    });
    e.fields.push(CustomField {
        name: "k".into(),
        value: "v".into(),
        secret: true,
    });
    e.urls.push("https://x".into());
    e.favorite = true;
    e.extra.insert("future".into(), serde_json::json!("secret?"));
    e.delete(9);
    assert!(e.deleted && e.history.is_empty() && e.fields.is_empty() && e.urls.is_empty());
    assert!(!e.favorite && e.extra.is_empty());
    assert_eq!(
        (e.id, e.created_at, e.updated_at),
        (Uuid::from_u128(1), 1_700_000_000_000, 9)
    );
}

// ---- D6: re-wrap weak password slots ----

#[test]
fn weak_password_slot_needs_rewrap() {
    let (file, _) = sample(21);
    let mut r = rng(210);
    let mut v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    assert_eq!(v.password_kdf(), Some(TEST_KDF));
    assert!(v.password_slot_needs_rewrap());

    v.change_password(&mut r, PW, KdfParams::DEFAULT).unwrap();
    assert!(!v.password_slot_needs_rewrap());
    let file = v.save(&mut r).unwrap();
    let v = UnlockedVault::unlock(&file, Unlock::Password(PW)).unwrap();
    assert_eq!(v.password_kdf(), Some(KdfParams::DEFAULT));
}

#[test]
fn kdf_weakness_ignores_lanes() {
    let d = KdfParams::DEFAULT;
    assert!(!d.is_weaker_than(&d));
    assert!(!KdfParams { p: 1, ..d }.is_weaker_than(&d));
    assert!(KdfParams {
        m_kib: d.m_kib / 2,
        ..d
    }
    .is_weaker_than(&d));
    assert!(KdfParams { t: d.t - 1, ..d }.is_weaker_than(&d));
    assert!(!KdfParams {
        m_kib: d.m_kib * 2,
        t: d.t,
        p: 2
    }
    .is_weaker_than(&d));
}

/// Freezes format v1 + payload schema 1: same seed + same input must give the same bytes.
/// If this fails after a change, the on-disk format or the payload JSON changed:
/// bump `format::VERSION` or `model::SCHEMA_VERSION` (or explain why neither is needed).
#[test]
fn golden_vector_v1() {
    let (file, rec) = sample(42);
    let digest: String = Sha256::digest(&file).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(file.len(), GOLDEN_LEN, "file length changed");
    assert_eq!(digest, GOLDEN_SHA256, "file bytes changed");
    assert_eq!(&rec[..11], GOLDEN_RECOVERY_PREFIX);
}

const GOLDEN_LEN: usize = 837;
const GOLDEN_SHA256: &str = "09a57f185e9542ec3c70b5c6fc340731d26ecdf62e7ce26fce756ce966aec117";
const GOLDEN_RECOVERY_PREFIX: &str = "4SKHR-6MHTJ";

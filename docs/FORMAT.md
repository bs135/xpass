# xPass vault format v1

Status: **frozen** for 1.0 (2026-11-26): file format **version 1**, payload **schema 1**. This document is the
reference for the `vault.xpv` file and the sync contract. It is written from the code and names the source of
each part. If this document and the code disagree, the code wins and this document has a bug.
[docs/spikes/vault-format.md](spikes/vault-format.md) is the historical spike and is out of date.

"Frozen" means that after 1.0, any change that an existing reader would misread needs a new `version` (envelope) or
`schema` (payload), a migration, and a new golden vector (§10). §9 lists what may change without a new number and
which builds check which number.

Sources: `crates/xpass-core/src/` `format.rs`, `crypto.rs`, `vault.rs`, `recovery.rs`, `model.rs`, `merge.rs`,
`clock.rs`, `maintenance.rs`, `restore.rs`, `otp.rs`, `sync/mod.rs`; `src-tauri/src/` `github.rs`, `history.rs`,
`sync_state.rs`, `settings.rs`, `hello.rs`; tests in `crates/xpass-core/tests/` `vault.rs`, `merge.rs`,
`restore.rs`, `sync.rs`.

## 1. Overview

```text
password  --Argon2id(salt, m, t, p)---------------> KEK_pw    -+
recovery  --HKDF-SHA256(salt, "xpass/kek-recovery/v1")-> KEK_rec -+- XChaCha20-Poly1305 wrap -> VaultKey
Hello sig --HKDF-SHA256(salt, "xpass/kek-hello/v1")----> KEK_hello -+                                |
                                                                                                      v
                                                          payload JSON --XChaCha20-Poly1305(AAD = header)--> body
```

- One random 256-bit **VaultKey** encrypts the payload. It never changes for a password change. A rotation
  replaces it.
- The VaultKey is stored only wrapped, once per **key slot** (password, recovery key, Windows Hello). Each slot
  has its own salt and nonce.
- The whole header is the payload's associated data: changing any header byte (including an unused slot) makes
  decryption fail.

## 2. File layout

All integers are little-endian. Sizes in bytes. Source: `format.rs`.

```text
header:
  magic          4   "XPSV" (58 50 53 56)
  version        u16 = 1
  vault_id       16  random at creation, never changes for the vault's life
  cipher         u8  = 1 (XChaCha20-Poly1305)
  slot_count     u8  1..=8
  slots          slot_count × slot
  payload_nonce  24  fresh random on every save
body:
  ciphertext     rest of the file: payload JSON encrypted + 16-byte Poly1305 tag

slot:
  kind           u8   1 = password, 2 = recovery, 3 = hello
  params_len     u16  length of params
  params         password: m_kib u32, t u32, p u32, salt 16   (28 bytes)
                 recovery: salt 16                             (16 bytes)
                 hello:    salt 32                             (32 bytes)
  nonce          24
  wrapped_len    u16  = 48
  wrapped        48   VaultKey (32) encrypted + tag (16)
```

Slot sizes: password 105, recovery 93, hello 109 bytes. Fixed header part: 24 + 24 bytes. A vault with a
password and a recovery slot therefore has a 246-byte header; with Hello, 355 bytes.

Writers emit slots sorted by `kind`; readers do not check the order. At most one slot per kind is allowed, so a
valid file has 1–3 slots even though `slot_count` may say up to 8. A new vault has a password and a recovery slot;
the Hello slot is optional and device-local (§5.3).

### Parsing rules

A reader must refuse the file (no partial result) when any of these hold. The parser reads the fields in file
order and stops at the first problem:

| Check (in the order met) | Error |
|---|---|
| fewer bytes than a field needs, anywhere (also `params` shorter than its kind needs) | `Format("truncated")` |
| magic is not `XPSV` | `Format("bad magic")` |
| `version` ≠ 1 | `UnsupportedVersion(version)` |
| `cipher` ≠ 1 | `Format("unknown cipher")` |
| `slot_count` is 0 or above 8 | `Format("bad slot count")` |
| per slot, after `params_len` bytes were taken: unknown `kind` | `Format("unknown slot kind")` |
| per slot: `params` longer than the kind needs | `Format("trailing bytes in slot params")` |
| per slot, after the nonce: `wrapped_len` ≠ 48 | `Format("bad wrapped key length")` |
| per slot, after `wrapped`: a kind seen before | `Format("duplicate slot kind")` |
| ciphertext shorter than 16 bytes | `Format("ciphertext too short")` |

A parsed header must encode back to the same bytes; the fuzz target `parse` checks this, because those bytes are
the payload's AAD.

## 3. Cryptography

Source: `crypto.rs`, `vault.rs`.

| Use | Algorithm |
|---|---|
| Password KEK | Argon2id, version 0x13, output 32 bytes, salt = slot salt (16 bytes) |
| Recovery and Hello KEK | HKDF-SHA256, salt = slot salt, IKM = recovery secret (32 bytes) or Hello signature, info below, output 32 bytes |
| Key wrap and payload | XChaCha20-Poly1305 (24-byte nonce, 16-byte tag) |
| Randomness | the OS CSPRNG (`OsRng`) for keys, salts, nonces and `vault_id` |

HKDF info strings (ASCII, no terminator): `xpass/kek-recovery/v1`, `xpass/kek-hello/v1`.

### Argon2id parameters

- Default for new and re-wrapped password slots: `m_kib = 131072` (128 MiB), `t = 3`, `p = 4`.
- Bounds (`KdfParams::validate`): `8192 ≤ m_kib ≤ 2097152` (8 MiB – 2 GiB), `1 ≤ t ≤ 20`, `1 ≤ p ≤ 16`.
  Checked when a password slot is read (unlock), before any Argon2 work, and also when one is written (create,
  change password, rotate). Outside them: `BadKdfParams`. A tampered header can therefore neither make unlocking
  trivially cheap nor exhaust memory.
- D6: after a successful password unlock, a slot weaker than the default (lower `m_kib` or `t`; `p` is ignored)
  is re-wrapped with the default and saved.

### Associated data

- **Slot wrap**: `AAD = magic ‖ version (u16 LE) ‖ vault_id ‖ kind (u8) ‖ params`. A slot cannot be moved to
  another vault, and its salt or KDF cost cannot be changed without breaking the unwrap.
- **Payload**: `AAD = the exact header bytes`, from `magic` through `payload_nonce`.

### Errors when opening

In this order (`UnlockedVault::unlock`):
1. the parsing rules of §2;
2. no slot of the requested kind: `NoSuchSlot`;
3. password slot outside the bounds: `BadKdfParams`; recovery key with a bad length, character or checksum:
   `BadRecoveryKey`;
4. the slot does not unwrap: `BadCredentials` (wrong password, recovery key or Hello signature);
5. the payload does not decrypt with the unwrapped key: `Tampered`;
6. the plaintext is not valid payload JSON: `Payload(..)`;
7. `schema` is greater than 1: `UnsupportedSchema(schema)` (checked since v0.8, §9).

## 4. Operations

| Operation | VaultKey | Slots | Notes |
|---|---|---|---|
| Create | new | password + recovery | returns the printable recovery key once |
| Save | same | same | `revision += 1`, new `payload_nonce` |
| Change password | same | password replaced | old copies still open with the old password and expose the same VaultKey |
| New recovery key | same | recovery replaced | the old key still opens older copies |
| Rotate | **new** | password + recovery new, Hello dropped | compromise response; older copies keep the old key |
| Enable Hello | same | hello added | needs a fresh Hello signature |

## 5. Credentials

### 5.1 Password

UTF-8 bytes of the master password, unchanged (no normalization). The app requires a zxcvbn score of at least 3
when the password is set; the format does not check it.

### 5.2 Recovery key

Source: `recovery.rs`.

- 32 random bytes `S`, plus the first 2 bytes of `SHA-256(S)` as a checksum.
- Encoded as RFC 4648 Base32 without padding (55 characters), grouped by 5 with `-`: 65 printable characters.
  Example shape: `ABCDE-FGHIJ-…`.
- Input is accepted case-insensitively, any whitespace and dashes are ignored, and `0` / `1` / `8` are read as
  `O` / `I` / `B`. An invalid character, a wrong length or a wrong checksum gives `BadRecoveryKey` before any key
  derivation.
- `S` is the HKDF input of the recovery KEK.

### 5.3 Windows Hello

- The app keeps a TPM-backed RSA-2048 key named `xpass-<vault_id>` (the UUID in lower-case hyphenated form;
  Windows `KeyCredentialManager`). It signs with PKCS#1 v1.5, which is deterministic, so the same challenge always
  gives the same signature.
- Challenge (62 bytes): `"xpass-hello-v1" ‖ vault_id (16 bytes) ‖ slot salt (32 bytes)`. It can be read from a
  locked file (`hello_challenge_for_file`).
- The signature is the HKDF input of the Hello KEK.
- The Hello slot is **device-local**. A synced copy from another device may carry that device's Hello slot. When
  comparing copies, Hello slots are not a key change, and each device keeps its own (sync S4).

## 6. Payload schema 1

Source: `model.rs`. The plaintext is UTF-8 JSON, written compactly (no whitespace) into an exact-size buffer,
known keys in the order of the tables below, unknown keys after them. Readers must accept any valid JSON with
these keys (key order and whitespace do not matter). A key with a default must be **absent** to take it: an
explicit `null` is an error. Integers: `schema` u32, `revision` u64, timestamps i64 (Unix ms).

### Payload (top level)

| Key | Type | Required | Meaning |
|---|---|---|---|
| `schema` | integer | yes | `1` |
| `revision` | integer | yes | incremented on every save |
| `entries` | array of Entry | yes | live entries and tombstones |
| *other keys* | any | no | kept as they are (`extra`, §9) |

### Entry

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | UUID string | required | stable for the entry's life |
| `created_at` | integer (Unix ms) | required | |
| `updated_at` | integer (Unix ms) | required | last change; the merge compares it (§7) |
| `deleted` | bool | `false` | tombstone |
| `title` | string | `""` | |
| `username` | string | `""` | |
| `password` | string | `""` | |
| `urls` | array of string | `[]` | first one is the main website |
| `notes` | string | `""` | |
| `tags` | array of string | `[]` | |
| `favorite` | bool | `false` | |
| `fields` | array of CustomField | `[]` | custom fields, in display order |
| `history` | array of HistoryItem | `[]` | older credentials, oldest first |
| *other keys* | any | | kept as they are (`extra`, §9) |

**Tombstone**: `deleted = true`. `id`, `created_at` and `updated_at` (the deletion time) are kept. All other known
keys are still written, with empty values (`""`, `[]`, `false`); unknown keys are removed.

### CustomField

| Key | Type | Default |
|---|---|---|
| `name` | string | required |
| `value` | string | required |
| `secret` | bool | `false` |

A secret field is treated like the password: hidden, revealed only on request, protected in a KDBX export.

**One-time passwords (TOTP)** are custom fields, not a separate key (`otp.rs`). A field holds a TOTP secret when
it is `secret`, its trimmed value parses as a TOTP secret, and either:
- the value starts with `otpauth://` (any case); or
- the trimmed name is `TOTP` or `OTP` (any case); then the value may be a URI or bare Base32.

Parsing:
- **URI** (Key URI format): `otpauth://totp/<label>?<parameters>`. The label is `issuer:account`; the separator is
  a literal `:` or `%3A`, and both parts are percent-decoded. Parameters: `secret` (required), `issuer` (overrides
  the label's issuer), `algorithm` `SHA1` / `SHA256` / `SHA512`, `digits` 6–8, `period` 15–120 s. Defaults are
  SHA1, 6 digits and 30 s. Unknown parameters are ignored, except `encoder`, which is refused (Steam).
- **Base32**: RFC 4648 alphabet. Spaces, dashes and tabs are ignored, lower case is accepted, and trailing `=` is
  optional.
- Refused: `otpauth://hotp/` and other types; a decoded secret outside 10–128 bytes; issuer or account over 128
  bytes or containing control characters; any input over 2048 bytes.

xPass writes new TOTP secrets as a field named `TOTP` whose value is the normalized URI. That URI keeps the label
`issuer:account`, writes `secret` without padding, then `issuer`, and then only the parameters that differ from
the defaults.

### HistoryItem

| Key | Type | Default | Meaning |
|---|---|---|---|
| `at` | integer (Unix ms) | required | an edit or import: when the credentials were replaced; a merge conflict: the losing version's `updated_at`; a restore: the replaced version's `updated_at` |
| `password` | string | `""` | |
| `username` | string | omitted when absent | the entry's username at the time, when it was not empty |
| `note` | string | omitted when absent | `conflict` (lost a merge), `restore`, `import` |

Canonical form (`normalized_history`), which every write and every merge keeps:
1. empty items and items equal to the entry's current `(password, username)` are dropped;
2. items with the same `(password, username)` are deduplicated, keeping the greatest by the order of step 3 (an
   absent `username` and `""` are different keys);
3. items are sorted ascending by `(at, password, username, note)`;
4. only the newest 20 (`HISTORY_LIMIT`) are kept.

## 7. Merge and time

Source: `merge.rs`, `clock.rs`, `maintenance.rs`.

- **Per-entry last writer wins (D1).** For each `id`: the higher `updated_at` wins; on a tie, a tombstone wins;
  still tied, the entry whose `serde_json` bytes (without `history`) compare greater byte by byte wins. The
  winner keeps all its keys, `fields` and unknown entry keys included: those of the losing versions are not
  merged in. Its history becomes the union of all versions' histories plus each live version's credentials with
  note `conflict`, in canonical form (§6). A tombstone winner keeps no history.
- **Entries** in the merged payload are sorted by `(created_at, id)`. `schema` and `revision` take the maximum;
  unknown top-level keys are united (on the same key, the larger serialized value wins).
- **Timestamps (D2).** A device writes `updated_at = max(now, last_seen + 1)`, where `last_seen` is the highest
  `updated_at` it has seen. A clock that is behind still produces newer values.
- **Tombstone purge (D3).** Tombstones with `updated_at < now − 180 days` are removed before a merge result or a
  local change is pushed (not by the key-less push on lock or exit). A device that has not synced for longer can
  bring such an entry back.
- **Clean device and an older remote (v0.8).** A device without local changes adopts the remote only when the
  remote holds every local entry at the same or a winning version (`merge::covers`; tombstones past the purge
  age ignored). Otherwise it merges and pushes. This way a rolled-back remote or a history rewrite racing a push
  loses no edit.
- **Restore** of a local backup is a new edit (`restore.rs`): every differing entry is rewritten with a fresh
  timestamp, entries created after the backup become tombstones, and the current `revision` and unknown top-level
  keys stay (`schema` takes the maximum). So the next sync does not undo it.

## 8. Sync contract

Source: `sync/mod.rs`, `src-tauri/src/github.rs`, `src-tauri/src/sync_state.rs`.

- The remote is **one file** (default path `xpass/vault.xpv` in the user's repository, on the configured or the
  default branch). It holds exactly the bytes of a vault file (§2).
- Compare-and-swap on the **git blob SHA** of the file: `SHA-1("blob " ‖ decimal length ‖ 0x00 ‖ bytes)`. A
  write names the SHA it expects. If the remote moved, the write is refused, and the device reads, merges and
  tries again, at most 5 times (`MAX_ATTEMPTS`), then `TooManyConflicts`. A remote body that does not parse or
  decrypt is read once more after 2 s before it counts as an error.
- Each change is one commit with the message `xpass: update vault`.
- **Remove old versions** (`history.rs`) replaces the branch with one parentless commit, `xpass: history removed`,
  holding the branch's whole current tree. Other files in the branch therefore lose their history too. It runs
  only when the vault blob on the branch equals this device's `baseSha`, and it does nothing when the branch has a
  single commit already.
- Local bookkeeping, `sync-state.json` next to the vault (camelCase JSON, every key optional):
  - `remoteId`: `lowercase(owner/repo)@branch:path`, with an empty branch meaning the repository's default branch;
  - `baseSha`: remote blob last reconciled with, or `null`;
  - `localSha`: blob SHA of the local file right after that, or `null`;
  - `slotsDirty`: a password or recovery slot changed locally and is not pushed yet;
  - `lastSyncAt`: Unix ms, or `null`.

  A missing or unreadable file, or one with another `remoteId`, means "never synced with this remote". The local
  file has unsynced changes when its blob SHA differs from `localSha`.
- A remote with another `vault_id` is refused (`DifferentVault`). If the remote payload does not open with the
  local VaultKey, the result depends on the slots other than Hello:
  - they differ: the key was rotated elsewhere (`KeyChanged`), and the master password of that copy is needed;
  - they are equal: the file is damaged (`Tampered`).
- A remote whose `schema` is newer than this build is refused (`UnsupportedSchema`, since v0.8).

## 9. Compatibility rules

Readers and writers of format 1 / schema 1:

1. **Unknown keys in Payload and Entry are preserved.** They are kept in `extra` and written back on save, so an
   older build does not drop data a newer one wrote. They are not zeroized in memory. Limits: a tombstone clears
   them, and in a merge an entry's unknown keys come from the winning version only (§7). Unknown top-level keys
   are united.
2. **CustomField and HistoryItem have no `extra`.** An older build drops any key it does not know when it saves
   the entry again. **New keys must not be added to these two objects within schema 1.** Put new data in a new
   Entry key instead.
3. A new **optional** key in Payload or Entry, whose absence means the old behaviour, does not need a new schema
   number. Every other payload change (a changed meaning, a required key, a new rule in the merge) needs
   `schema = 2`.
4. Any change to the envelope (§2) or the cryptography (§3: algorithms, AAD, info strings, KDF bounds) needs
   `version = 2`. Every build refuses versions it does not know (`UnsupportedVersion`), so an old build never
   misreads a new file.
5. **Which builds check `schema`:** builds from v0.8 refuse a `schema` above the one they know
   (`UnsupportedSchema`). Builds up to v0.7 do not check it: they would open a `schema = 2` file and save it back
   as if it were schema 1. As long as such builds may still sync a vault, a payload change that they must not
   misread needs `version = 2` as well, not only `schema = 2`.
6. Raising the default Argon2 parameters needs no new version: they are stored per slot and re-wrapped on unlock
   (D6). Lowering the read bounds is a security change and needs one.

## 10. Reference file (golden vector)

`crates/xpass-core/tests/vault.rs` `golden_vector_v1` builds a vault with `ChaCha20Rng::seed_from_u64(42)` (for
both create and save), the password `correct horse battery staple`, the smallest KDF the bounds allow
(`m_kib = 8192`, `t = 1`, `p = 1`) and two entries (ids `Uuid::from_u128(1)` and `(2)`, timestamps
`1_700_000_000_000`), saved once (`revision = 1`). It checks:
- the file is **837 bytes**;
- its SHA-256 is `09a57f185e9542ec3c70b5c6fc340731d26ecdf62e7ce26fce756ce966aec117`;
- the recovery key starts with `4SKHR-6MHTJ`.

The 837 bytes are a 246-byte header, 575 bytes of payload JSON and the 16-byte tag. Any change to the bytes written
for the same input fails this test. Changing it means one of §9.3–§9.5 applies, or the change needs a written
reason in the pull request.

Other tests that pin the format: header tampering (`any_single_byte_flip_is_detected`), fresh nonces
(`every_saved_file_uses_a_fresh_nonce`), KDF bounds (`kdf_downgrade_is_rejected`), recovery checksum
(`recovery_key_typo_is_caught_by_checksum`), unknown keys kept (`unknown_fields_survive_unlock_and_save`,
`unknown_fields_survive_json_roundtrip`), newer schema refused (`newer_payload_schema_is_refused`), and the merge
properties in `tests/merge.rs`.

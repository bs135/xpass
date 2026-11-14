# Spike: vault file format v1 + crypto

Status: **passed** (2026-11-14). Code: [`spikes/vault-format`](../../spikes/vault-format).
Related: [IDEA.md §3.2–3.4](../IDEA.md#32-key-hierarchy-critical), [windows-hello.md](windows-hello.md).

## Goal

Validate the custom vault format before planning:

1. Key hierarchy: one random VaultKey, wrapped independently by password, recovery key and Windows Hello.
2. Tamper detection over the whole file, including KDF parameters.
3. Password change vs. compromise response (VaultKey rotation).
4. A data model that is ready for multi-device merge.
5. Argon2id cost on real hardware, and file size growth.

## Decisions taken in the spike

| Topic | Choice | Why |
|---|---|---|
| AEAD | **XChaCha20-Poly1305** (`chacha20poly1305`) | 24-byte random nonces: no nonce bookkeeping across devices. Fast in software. |
| Password KDF | **Argon2id** (`argon2` 0.5), default m = 128 MiB, t = 3, p = 4 | ~0.9 s on the target laptop (see benchmark). Params live in the slot, so they can be raised later. |
| Recovery / Hello KDF | **HKDF-SHA256** | Both inputs have high entropy (256-bit key, RSA signature); Argon2 adds nothing. |
| Envelope | Hand-written binary header | Small, explicit, every read bounds-checked; easy to fuzz. |
| Payload | JSON (`serde_json`) inside the ciphertext | Readable after decryption, easy CSV/KDBX export, schema versioned. |
| Recovery key | 32 random bytes + 2-byte checksum, Base32, groups of 5 | 65 characters printed; typos are caught before any crypto runs. |

## Format v1

All integers little-endian.

```text
header (AAD for the payload):
  magic         4   "XPSV"
  version       u16 = 1
  vault_id      16  random, stable for the vault's life
  cipher        u8  = 1 (XChaCha20-Poly1305)
  slot_count    u8  (1..=8, at most one slot per kind)
  slots         slot_count * slot
  payload_nonce 24  fresh on every save
body:
  ciphertext    JSON payload + 16-byte tag

slot:
  kind          u8  1 = password, 2 = recovery, 3 = hello
  params_len    u16
  params        password: m_kib u32, t u32, p u32, salt 16
                recovery: salt 16
                hello:    salt 32
  nonce         24
  wrapped_len   u16 = 48
  wrapped       VaultKey (32) + tag (16)
```

Binding:

- **Slot wrap AAD** = `magic ‖ version ‖ vault_id ‖ kind ‖ params`. A slot cannot be moved to another vault, and its
  salt or KDF cost cannot be changed without breaking the unwrap.
- **Payload AAD** = the exact header bytes. Any change to the header (including unused slots) breaks decryption.
- **Hello challenge** = `"xpass-hello-v1" ‖ vault_id ‖ salt`; the app reads it from the locked file, asks Hello to
  sign it, then unlocks (flow proven in the Windows Hello spike).

A vault with password + recovery slots has a 246-byte header.

Payload (schema 1):

```json
{
  "schema": 1,
  "revision": 7,
  "entries": [
    { "id": "uuid", "created_at": 1700000000000, "updated_at": 1700000000000, "deleted": false,
      "title": "", "username": "", "password": "", "url": "", "notes": "", "tags": [] }
  ]
}
```

- `revision` increments on every save (helps sync detect stale overwrites).
- Deletion turns an entry into a **tombstone**: id and timestamps kept, content wiped, `deleted = true`.
- Payload structs derive `Zeroize`/`ZeroizeOnDrop`; plaintext buffers are `Zeroizing<Vec<u8>>`.

## Operations

| Operation | Effect |
|---|---|
| `create(password, kdf)` | New vault_id + VaultKey, password slot, recovery slot. Returns the recovery key to show once. |
| `unlock(file, Password / Recovery / Hello)` | Parse, unwrap VaultKey from the matching slot, decrypt payload. |
| `save()` | Bump revision, fresh payload nonce, re-encrypt. |
| `change_password()` | Re-wrap the **same** VaultKey. Old files still open with the old password. |
| `rotate(password, kdf)` | **Compromise response**: new VaultKey, new password slot, new recovery key; Hello slot dropped (re-enroll). |
| `set_hello_slot(salt, signature)` / `remove_hello_slot()` | Hello enrollment in two steps (salt → sign → wrap). |

## Test results

`cargo test`: **14 / 14 passed**.

| Test | What it proves |
|---|---|
| `roundtrip_with_password` | Create → save → unlock gives back the entries. |
| `wrong_password_is_rejected` | `BadCredentials`, no payload access. |
| `recovery_key_unlocks_and_tolerates_formatting` | Recovery key works; lowercase and spaces instead of dashes accepted. |
| `recovery_key_typo_is_caught_by_checksum` | A one-character typo returns `BadRecoveryKey`. |
| `hello_slot_unlocks_with_signature` | Challenge read from the locked file; correct signature unlocks, forged one fails. |
| `every_saved_file_uses_a_fresh_nonce` | Payload nonce differs per save; revision increments. |
| `any_single_byte_flip_is_detected` | Flipping **any** byte of the file makes unlock fail. |
| `kdf_downgrade_is_rejected` | Lowering `m_kib` in the header → `BadCredentials`; absurd values → `BadKdfParams`. |
| `version_and_magic_are_checked` | Unknown version / magic rejected with a clear error. |
| `change_password_keeps_vault_key` | New password works, old fails on the new file — and the **old file still yields the same VaultKey** (IDEA.md §3.3, now demonstrated). |
| `rotate_replaces_vault_key_and_all_slots` | New VaultKey; old recovery key and Hello slot no longer work on the new file. |
| `delete_leaves_a_tombstone` | Tombstone keeps id/timestamps, wipes content. |
| `parser_never_panics_on_garbage` | Every truncation of a valid file + 5,000 random buffers: errors, no panics. |
| `golden_vector_v1` | Fixed seed → fixed 747-byte file with a fixed SHA-256. Freezes format v1. |

## Benchmark

Machine: Intel Core i7-1185G7, 24 GB RAM, Windows 11. `cargo run --release --example bench`.

Argon2id (median of 3):

| m (MiB) | t | p | time |
|---:|---:|---:|---:|
| 64 | 2 | 4 | 340 ms |
| 64 | 3 | 4 | 436 ms |
| 128 | 2 | 4 | 635 ms |
| **128** | **3** | **4** | **908 ms** |
| 256 | 2 | 4 | 1329 ms |
| 256 | 3 | 4 | 2691 ms |
| 512 | 2 | 4 | 3543 ms |
| 512 | 3 | 4 | 4200 ms |

Vault size (realistic entries with notes and tags; recovery unlock, so no Argon2 in the timings):

| entries | file | save | unlock |
|---:|---:|---:|---:|
| 0 | 0.3 KB | 19 µs | 31 µs |
| 100 | 37 KB | 0.6 ms | 1.1 ms |
| 500 | 187 KB | 2.5 ms | 5.0 ms |
| 2000 | 749 KB | 9.8 ms | 19 ms |

## Conclusions

1. **The format works.** Key hierarchy, tamper detection, password change, rotation and recovery all behave as
   designed in IDEA.md; the golden vector freezes v1.
2. **Default KDF: m = 128 MiB, t = 3, p = 4** (~0.9 s). Password unlock is rare once Hello is enabled, so this cost
   is acceptable. Store params per slot and re-wrap with stronger params on a later unlock if the default changes.
3. **The `argon2` 0.5 crate is single-threaded**: p = 4 lanes run one after another. A multi-threaded implementation
   could afford ~256 MiB in the same time. Revisit when choosing the final crate version.
4. **Size is not a concern for crypto** (2000 entries: <20 ms), but **2000 entries ≈ 750 KB is close to the GitHub
   Contents API 1 MB limit** for the default JSON response. Spike 3 must use the raw media type
   (`Accept: application/vnd.github.raw`) for downloads, or the Git Data API.
5. `change_password` alone does not help after a leak — the test demonstrates it. The UI must steer users to
   **rotate** when they suspect compromise.

## Follow-ups for the real implementation

- [ ] Compress the payload before encryption (e.g. zstd)? Saves space for sync; the length leak is negligible for a
  personal vault. Decide during planning.
- [ ] Separate secret fields from metadata so the frontend receives only metadata (IDEA.md §3.6).
- [ ] Per-field `updated_at` or LWW per entry — decided in spike 3 (merge).
- [ ] Atomic local writes (write temp file + rename) and keep the last N local backups.
- [ ] Re-wrap the password slot automatically when stored KDF params are below the current default.

## Running the spike

```bash
cd spikes/vault-format
cargo test
cargo run --release --example bench
```

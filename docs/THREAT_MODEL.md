# xPass — Threat Model

Status: **v0.6** (2026-11-24), written against the code on `develop` after v0.5.0. Updated when a release changes
what is stored, who can reach it or how it is protected. Decisions it refers to: [IDEA.md §3](IDEA.md#3-design-issues),
[PLAN.md §2](PLAN.md#2-decisions), and the per-milestone plans (`plan-v0.X.0.md`).

xPass is a **client-side encrypted** password manager for one person on Windows. There is no xPass server: the only
remote party is the user's own private GitHub repository, which receives the encrypted vault file.

## 1. Assets

| Asset | Where it lives | Sensitivity |
|---|---|---|
| Entry secrets (passwords, notes, custom fields, password history) | Encrypted in `vault.xpv`; plaintext only in the Rust process while unlocked | Critical |
| Entry metadata (titles, usernames, URLs, tags) | Same as above; also in the WebView while unlocked (list, detail) | High |
| **VaultKey** (random 256-bit) | Rust memory while unlocked; on disk only wrapped in header slots | Critical |
| Master password | Typed by the user; in the WebView form and Rust memory during unlock / re-auth | Critical |
| Recovery key (Emergency Kit) | Printed / saved by the user; in memory only while shown or typed | Critical |
| Windows Hello key | TPM (RSA-2048, never leaves it); only its signature reaches Rust, briefly | Critical |
| GitHub PAT (fine-grained, one repo, Contents R/W) | Windows Credential Manager (`xpass/github-token`); Rust memory during a request | High (write access to the vault repo) |
| Local backups | `%APPDATA%\xpass\backups\` (last 10 encrypted copies) | As the vault, but with the credentials valid when written |
| Exported files | Where the user saves them: CSV is **plaintext**, KDBX is encrypted with an export password | Critical (CSV) |
| Non-secret state | `settings.json` (repo, branch, path, timeouts, language), `sync-state.json` (blob SHAs) | Low |

## 2. Trust boundaries

```text
 ┌──────── WebView2 process (React, untrusted for secrets) ────────┐
 │ CSP: self only, no eval, connect-src = IPC only                  │
 │ sees: metadata, a secret only on reveal / edit form              │
 └──────────────▲──────────────────────────────────────────────────┘
                │ (1) Tauri IPC: allow-listed commands, JSON
 ┌──────────────┴──────────── Rust process ─────────────────────────┐
 │ xpass-core: parse, KDF, AEAD, merge   src-tauri: state, OS glue │
 └───▲─────────────▲─────────────▲───────────────▲──────────────────┘
     │(2) disk      │(3) OS       │(4) network    │(5) files chosen by the user
  vault.xpv,     Credential Mgr,  api.github.com   CSV / KDBX import, export
  backups        TPM / Hello,     (rustls, HTTPS)
                 clipboard
```

1. **WebView ↔ Rust.** The frontend is treated as able to read whatever it is sent, never as trusted to decide
   anything security-relevant: every check (lock state, re-auth, throttle, URL scheme, file paths) is in Rust.
2. **Disk.** Anything on disk may be read or modified by someone with the file (stolen laptop, backup tool, sync
   folder). Only the header and ciphertext of `vault.xpv` are there; the header is authenticated.
3. **OS services.** Credential Manager is DPAPI per user (readable by any process of that user); the TPM key needs
   Hello consent per signature; the clipboard is global.
4. **Network.** GitHub sees the encrypted file, its size, commit times and the IP address. TLS via rustls with
   the platform roots; `https_only`.
5. **Imported files** are hostile input; exported files leave xPass's control once written.

## 3. Attackers and what they get

| # | Attacker | Has | Gets | Why |
|---|---|---|---|---|
| A1 | Someone with the **encrypted vault** (leaked / public repo, stolen PAT, stolen backup, stolen powered-off laptop) | `vault.xpv` and its git history | Nothing without a credential; can try **offline guessing** of the master password | Argon2id 128 MiB / t=3 / p=4 per guess; zxcvbn score ≥ 3 required for the master password; recovery key 256-bit; Hello needs the TPM. Header tampering is detected (AAD); KDF downgrade in a file is refused. |
| A2 | Someone who learned an **old** master password | Old password + an old copy (git history, local backup) | Every old copy. "Change password" keeps the VaultKey, so the old copy also yields the **current** VaultKey: together with the current file (A1) they read **current** data | Hence **rotate VaultKey** after a suspected compromise (new key, payload re-encrypted, new Emergency Kit, Hello dropped). Old copies stay readable: see §5. |
| A3 | **Stolen or unattended PC while locked** (or while the app is closed) | Disk, user session maybe | Same as A1. Hello cannot be used without the user's biometric / PIN | Auto-lock on idle, Windows lock, display off, sleep, app exit; lock drops the session. |
| A4 | Someone at the **unlocked** PC | The open app | Everything the user can do — except export, change password, rotate, new recovery key and Hello enable, which ask for the password (export: or Hello) again | Re-auth for sensitive actions (throttled); revealed secrets auto-hide after 15 s. |
| A5 | **Malware running as the user** | Same rights as xPass | **Not protected.** Can read process memory while unlocked, log keystrokes, read the PAT from Credential Manager, replace the binary, read the clipboard while a secret is on it | Out of scope for any user-mode password manager. Mitigations only limit the window: lock early, short clipboard timeout. |
| A6 | **Malicious dependency** (npm or crate) | Code in the build | Frontend code could exfiltrate what the WebView sees; Rust code anything | npm: few deps, lockfile, `npm audit`, CSP blocks network from the WebView, no secrets in stores. Crates: lockfile, `cargo deny` (T31). Remaining risk accepted. |
| A7 | **Hostile CSV** | A file the user imports | Crash or memory blow-up at most | 10 MB / 10 000-row limits before parsing, UTF-8 only, pure-Rust parser, errors without file content, nothing written before the preview is confirmed. |
| A8 | **Hostile or broken remote file** (repo write access via a stolen PAT) | Ability to replace `vault.xpv` on GitHub | Denial of service (sync error); cannot inject entries without the VaultKey | Remote file is parsed and AEAD-checked like a local one; `DifferentVault` / `KeyChanged` / `Tampered` stop the merge; local file and backups untouched. A rolled-back (older, valid) remote copy only re-adds what merge already knows (LWW, tombstones). |
| A9 | **Network attacker** | Path to GitHub | Metadata (timing, size) | TLS (rustls), HTTPS only; the file is encrypted anyway. |
| A10 | Fake **"update available"** | Control of the update response | Nothing installed: the update check (T35) only shows a version and opens a fixed `github.com/bs135/xpass/releases/` URL in the browser | No download, no auto-install in v0.6. |

## 4. Security properties and where they are enforced

| Property | Enforced by | Proof |
|---|---|---|
| Payload confidentiality + integrity | XChaCha20-Poly1305, random 24-byte nonce per save, AAD = whole header (`xpass-core/src/vault.rs`) | `tests/vault.rs`: `any_single_byte_flip_is_detected`, `every_saved_file_uses_a_fresh_nonce` |
| Key hierarchy (IDEA §3.2) | VaultKey wrapped per slot; slot AAD = magic, version, vault id, slot kind + params (`format.rs` `slot_aad`) | `change_password_keeps_vault_key`, `rotate_replaces_vault_key_and_all_slots`, `hello_slot_unlocks_with_signature` |
| No KDF downgrade from a file | `KdfParams::validate` on read; password slot re-wrapped on unlock when weaker than the default (D6) | `kdf_downgrade_is_rejected` |
| Parser never panics | Bounds-checked `Reader`, slot count 1..=8, unique kinds, fixed wrapped length | `parser_never_panics_on_garbage`; fuzz targets (T33) |
| Recovery key typos caught before KDF | Checksum in `recovery::decode` | `recovery_key_typo_is_caught_by_checksum` |
| Online guessing slowed | `throttle.rs`: 0, 0, 1, 2 … 30 s, shared by unlock and re-auth; in memory (restart resets it — offline guessing is bounded by Argon2, not by this) | `throttle.rs` tests |
| Strong master password | zxcvbn score ≥ 3 (`strength.rs`), also for the KDBX export password | `strength.rs` tests |
| Hello is a key, not a yes/no (IDEA §3.1) | `KeyCredentialManager` sign → HKDF → KEK (`hello.rs`); no `UserConsentVerifier` | `hello_slot_unlocks_with_signature`; manual v0.3 checklist |
| Secrets stay in Rust | `list_entries` / `get_entry` send no secrets; `reveal_secret` / `copy_field` by reference (`FieldRef`); import preview has no passwords / notes; export returns only the file name | `entries.rs` tests (`list_is_metadata_only_sorted_and_skips_tombstones`, `secret_custom_field_is_never_in_details`), `state/import.rs` tests |
| WebView cannot reach the network or files | CSP (`tauri.conf.json`), capabilities limited to events + window controls (`capabilities/default.json`), no `fs` / `shell` / `dialog` / `opener` / `http` plugins; links opened by Rust after an http(s) check (`entries::normalize_url`, `shell.rs`) | Manual baseline check per release |
| Clipboard | Excluded from history, cloud and monitoring; cleared after the timeout and on lock only if the sequence number is unchanged (`clipboard.rs`) | `clipboard.rs` tests; manual `Win+V` check |
| Auto-lock | Idle (`GetLastInputInfo`), `WTS_SESSION_LOCK`, display off, `PBT_APMSUSPEND`, app exit (`lock_watch.rs`) | Manual v0.1 checklist |
| Wipe on lock | `UnlockedVault` (VaultKey `Zeroizing`), `Payload` / `Entry` / `HistoryItem` / `CustomField` `ZeroizeOnDrop`; pending import dropped; clipboard cleared | Code review (this document) |
| PAT handling | Credential Manager only; never returned to the WebView, written to settings, logged or put in errors; `Debug` redacts it (`secrets.rs`, `github.rs`) | `github.rs` redaction test |
| Atomic writes | Temp file + flush + rename; backup before each write (`storage.rs`); exports in the target folder, never `%TEMP%` | `storage.rs` tests |
| Single instance, no devtools in release | `tauri-plugin-single-instance`; `devtools` feature not enabled | Manual baseline check |

## 5. Accepted limits

Known gaps that v0.6 does **not** close. Each was weighed against its cost; revisit before 1.0.

| Limit | Impact | Why accepted / what reduces it |
|---|---|---|
| Malware as the user (A5) | Full compromise while unlocked; PAT readable any time | No user-mode defence exists; lock early. |
| JavaScript strings cannot be wiped; WebView2 is a separate process | A revealed password or the edit form's password may stay in WebView memory until GC | Secrets reach the WebView only on explicit reveal / edit; auto-hide 15 s. |
| Tauri IPC buffers (request and response JSON) are not wiped | Passwords sent to Rust (unlock, re-auth, edit) and revealed secrets leave copies in freed memory | After T32 the Rust-side `String`s are `Zeroizing`; the framework's buffers stay. |
| `serde_json` scratch buffer when decoding escaped strings; `csv` crate buffers | Fragments of plaintext in freed memory | No API to wipe them; short-lived. |
| Decrypted payload not page-locked | Entry plaintext could be written to the pagefile | T32 locks the VaultKey page only; locking the payload needs a custom allocator. |
| Hibernation | `hiberfil.sys` contains all RAM, locked pages included | xPass locks on suspend; hibernation after the lock contains no key. A crash dump (WER) while unlocked would contain secrets — excluding the process from WER is a v0.7+ item. |
| Git history and local backups keep old encrypted copies | An old password (A2) opens old copies forever | Rotate after a suspected compromise; README explains. History rewrite (IDEA §3.3) is a v0.7+ item. Local backups after a rotation are still the old key's copies (last 10). |
| Metadata leaks to GitHub | Commit times, file size ≈ entry count, IP address | Accepted for personal use (IDEA §3.3). |
| `Payload.extra` / `Entry.extra` (unknown fields from newer versions, D5) not zeroized | Only matters for fields written by a future version | `serde_json::Value` has no zeroize support. |
| Plaintext CSV export | Anyone with the file reads everything | Warning + checkbox + re-auth; KDBX offered first; result screen says to delete it. |
| Unsigned builds | A tampered installer cannot be told apart by signature; SmartScreen warns | Publish SHA-256 checksums (T36); code signing is a v0.7+ item. |
| Hello key name `xpass-<vault_id>` is in the per-user namespace | Another app of the same user can ask to sign with it — but each signature shows the Hello prompt | Spike finding (D9); A5 already wins in that case. |
| Unlock throttle resets on restart | Online guessing at the UI is limited only by Argon2 cost after a restart | ~1 s per guess on the target laptop; the real defence against guessing is the password strength gate. |
| Update check trusts GitHub's TLS and API | A compromised GitHub account could announce a fake version | Nothing is downloaded; the user installs from the release page (signed updater is v0.7+). |

## 6. Security self-review (v0.6)

Checklist from [IDEA.md §3](IDEA.md#3-design-issues) and the milestone plans, checked against the code. Status:
**OK** = done and tested / verified; **v0.6** = fixed by the named task in this milestone; **limit** = accepted
(§5); **later** = planned after v0.6.

| # | Item | Status | Evidence / note |
|---|---|---|---|
| R1 | Hello via `KeyCredentialManager` signature → KEK, not `UserConsentVerifier`; only the wrapped VaultKey stored (§3.1) | OK | `hello.rs`, `vault.rs` (`INFO_HELLO`), v0.3 checklist |
| R2 | Random VaultKey wrapped by password / recovery / Hello; header as AAD; KDF params stored and raisable (§3.2) | OK | `format.rs`, `vault.rs`, D6 re-wrap |
| R3 | Change password re-wraps only; rotation re-encrypts with a new key and a new Emergency Kit (§3.3) | OK | `rotate_replaces_vault_key_and_all_slots`; online-only with sync (S6) |
| R4 | Git history leaks old vaults; optional history rewrite (§3.3) | later | README explains rotation; rewrite is v0.7+ |
| R5 | Conflict detection via blob SHA, per-entry LWW + tombstones, offline-first (§3.4) | OK | `xpass-core/tests/sync.rs`, `merge.rs`; v0.4 two-device checklist |
| R6 | REST Contents API, fine-grained PAT, `401` handled (§3.5) | OK | `github.rs`; "token expired" banner |
| R7 | Frontend never gets passwords except reveal / edit (§3.6) | OK | §4 "Secrets stay in Rust" |
| R8 | Strict CSP, no remote content, minimal capabilities, no `fs`, devtools off (§3.6) | OK | `tauri.conf.json`, `capabilities/default.json` |
| R9 | npm supply chain: minimal deps, audit (§3.6, PLAN §6) | v0.6 | 8 runtime deps; `npm audit` in CI — **T31** |
| R10 | Crate advisories and licences (no AGPL / GPL) | v0.6 | `cargo deny` in CI — **T31** |
| R11 | `zeroize` on keys and payload; `secrecy`-style care at deserialization (§3.6) | v0.6 | Keys / payload OK. **Gap:** IPC arguments carrying passwords (`unlock_password`, `create_vault`, `change_password`, `rotate_vault_key`, `new_recovery_key`, `restore_backup`, `hello_enable`, `sync_join`, `sync_resolve_key_change`, `sync_replace_local`, `sync_set_token`, `export_vault`, `Auth::Password`, `EntryInput.password`, `password_strength`) are plain `String` → **T32** |
| R12 | No plaintext copies from `Vec` growth when saving | v0.6 | `UnlockedVault::save` uses `serde_json::to_vec` → **T32** (pre-sized buffer) |
| R13 | `VirtualLock` for keys (§3.6) | v0.6 | VaultKey inline in `UnlockedVault` → **T32** (`LockedKey`); payload: limit |
| R14 | Clipboard excluded from history / cloud / monitoring; clear only if unchanged (§3.7) | OK | `clipboard.rs` + tests |
| R15 | Auto-lock: idle, Windows lock, sleep, exit (§4) | OK | `lock_watch.rs`; also display off (Modern Standby) |
| R16 | Strength check for the master password (§4) | OK | zxcvbn ≥ 3 |
| R17 | Header parser fuzzed (§6 testing) | v0.6 | Random-input unit test today → **T33** `cargo-fuzz` |
| R18 | Wrong-password throttling | OK | `throttle.rs`; shared with re-auth |
| R19 | Re-auth before export, password change, rotation, new kit, Hello enable (a recovery-key session is not asked again) | OK | `authorize` / `reauthorize` in `state.rs`, `state/reauth.rs` |
| R20 | Import: limits before parsing, no file content in errors or preview, pending rows zeroized and expired | OK | `xpass-interop` tests, `state/import.rs` (`IMPORT_TTL`) |
| R21 | Export: temp + rename next to the target, never `%TEMP%`; lock re-checked before writing | OK | `state/export.rs` |
| R22 | URLs opened only as http(s) with a host, by Rust | OK | `entries::normalize_url`, `shell.rs` |
| R23 | PAT never in logs / settings / errors / WebView | OK | `secrets.rs`, `github.rs` redaction test |
| R24 | Logs carry no secrets | OK | `eprintln!` only for lock reasons, window messages, Hello focus and panic location (panic payload not printed); release builds have no console |
| R25 | Update check from Rust, no WebView network (D10) | v0.6 | **T35** |
| R26 | Release integrity for unsigned builds | v0.6 | SHA-256 checksums — **T36**; code signing later |
| R27 | Branch protection on `main` while releasing | v0.6 | Ruleset currently disabled; release workflow stops pushing to `main` — **T34**, user re-enables the ruleset |
| R28 | Crash dumps while unlocked | later | WER exclusion v0.7+ |
| R29 | Signed auto-update | later | v0.7+ |

Findings of this review that were not already in the v0.6 plan: `sync_set_token` (PAT) and `copy_text` also take a
plain `String` over IPC (`copy_text` wraps it right after) — added to T32's list (R11).

## 7. Reporting a vulnerability

Report it privately through GitHub ("Security" tab → "Report a vulnerability") rather than in a public issue.

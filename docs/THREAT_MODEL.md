# xPass — Threat Model

Status: **1.0** (2026-11-26), written against the code on `develop` after v0.5.0 and updated for v0.7.0 (signed
updates, code signing, crash dumps, history rewrite, release provenance) and v0.8.0 (TOTP codes, custom field
editing, password health, payload schema check). 1.0.0 has the same code as v0.8.0, and §6 was redone for it. No
independent review has checked this document or the code yet (v1.0 plan W1). The vault format is specified in
[FORMAT.md](FORMAT.md). Updated when a release changes what is stored, who can reach it or how it is protected. Decisions it refers to: [IDEA.md §3](IDEA.md#3-design-issues),
[PLAN.md §2](PLAN.md#2-decisions), and the per-milestone plans (`plan-v0.X.0.md`).

xPass is a **client-side encrypted** password manager for one person on Windows. There is no xPass server: the only
remote party is the user's own private GitHub repository, which receives the encrypted vault file.

## 1. Assets

| Asset | Where it lives | Sensitivity |
|---|---|---|
| Entry secrets (passwords, **secret** custom fields, password history) | Encrypted in `vault.xpv`; plaintext only in the Rust process while unlocked, except an explicit reveal or copy | Critical |
| TOTP secrets (v0.8, secret custom fields) | Same as entry secrets; only the current code reaches the WebView | Critical: with the password, they open the account on their own (see §5) |
| Entry content shown on open (titles, usernames, URLs, tags, **notes**, non-secret custom fields) | Encrypted in `vault.xpv`; in the WebView while an entry is open (list: titles, usernames, hosts, tags) | High. Notes are not treated as secrets: users are told to keep secrets in the password or a secret custom field |
| **VaultKey** (random 256-bit) | Rust memory while unlocked; on disk only wrapped in header slots | Critical |
| Master password | Typed by the user; in the WebView form and Rust memory during unlock / re-auth | Critical |
| Recovery key (Emergency Kit) | Printed / saved by the user; in memory only while shown or typed | Critical |
| Windows Hello key | TPM (RSA-2048, never leaves it); only its signature reaches Rust, briefly | Critical |
| GitHub PAT (fine-grained, one repo, Contents R/W) | Windows Credential Manager (`xpass/github-token`); Rust memory during a request | High (write access to the vault repo) |
| Local backups | `%APPDATA%\com.bs135.xpass\backups\` (last 10 encrypted copies) | As the vault, but with the credentials valid when written |
| Exported files | Where the user saves them: CSV is **plaintext**, KDBX is encrypted with an export password | Critical (CSV) |
| Non-secret state | `settings.json` (repo, branch, path, timeouts, language), `sync-state.json` (blob SHAs); neither is encrypted or authenticated | Low (see §5 for what a changed `settings.json` can do) |
| **Update signing key** (minisign, v0.7) | Owner's offline backup (with its password) and the GitHub environment `release` secrets; never in the repository | Critical for every user: whoever has it can sign an update that xPass installs after a click (A11) |
| **Code-signing certificate** (self-signed PFX, v0.7) | Same as the update key | High: whoever has it can sign binaries with the published thumbprint (A12) |

## 2. Trust boundaries

```text
 ┌──────── WebView2 process (React, untrusted for secrets) ────────┐
 │ CSP: self only, no eval, connect-src = IPC only                  │
 │ sees: entry content incl. notes, secrets only on reveal / edit, │
 │ TOTP codes, generated passwords                                  │
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
   the platform roots; `https_only`. Since v0.7 also `github.com/bs135/xpass/releases` for the update manifest
   (`latest.json`) and installers; an installer runs only after its minisign signature verifies against the
   public key built into the app, and only after the user clicks **Install**.
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
| A8 | **Hostile or broken remote file** (repo write access via a stolen PAT) | Ability to replace `vault.xpv` on GitHub | Denial of service (sync error); cannot inject entries without the VaultKey | Remote file is parsed and AEAD-checked like a local one; `DifferentVault` / `KeyChanged` / `Tampered` stop the merge; local file and backups untouched. A rolled-back (older, valid) remote copy only re-adds what merge already knows (LWW, tombstones); since v0.7 a device without local changes also merges (instead of adopting the remote) when the remote lacks some of its entries, so a rollback or a history rewrite racing a push loses no edit. |
| A9 | **Network attacker** | Path to GitHub | Metadata (timing, size) | TLS (rustls), HTTPS only; the file is encrypted anyway. |
| A10 | **Fake or tampered update** | Control of the update response or the download (network, CDN, a GitHub account without the signing key) | Nothing installed; at most misleading release notes or a version shown that cannot be installed | `latest.json` itself is **not** signed: it carries, per installer, the minisign signature of that installer. Each downloaded installer must verify against the key built into the app, and the signature's trusted comment must name the offered version (`requireSignedVersion`); download URLs only under `github.com/bs135/xpass/releases/download/v<version>/`; only plain `x.y.z` versions newer than the running one; the user confirms every install (v0.7 U3, U4, U6). The release notes and date shown before the install are unauthenticated text (§5). |
| A11 | **Update signing key and GitHub account both compromised**, or a compromised step of the release job | The minisign private key and its password, and the ability to publish a release; or code running in the job that holds them | A signed malicious update reaches every user who clicks **Install** | Key password; the key lives only in the owner's offline backup and the `release` environment, whose deployment branches are limited and which only the release job on `main` uses; `main` protected by an active ruleset; third-party actions in the workflows pinned to commit SHAs (1.0), so a moved tag cannot change the code that sees the secrets; nothing installs without a click. Not prevented once both are lost; a leak means a new key and a manual reinstall for every user (user guide "Updates"). |
| A12 | Someone with the **code-signing PFX** and its password | The certificate's private key | Binaries that carry our thumbprint | Same protection as the update key (environment secret, offline backup). The thumbprint is never a trust root on its own: checksums, attestations and the update signature still hold. Rotation per v0.7 U8, announced in the release notes and README. |

## 4. Security properties and where they are enforced

| Property | Enforced by | Proof |
|---|---|---|
| Payload confidentiality + integrity | XChaCha20-Poly1305, random 24-byte nonce per save, AAD = whole header (`xpass-core/src/vault.rs`) | `tests/vault.rs`: `any_single_byte_flip_is_detected`, `every_saved_file_uses_a_fresh_nonce` |
| Key hierarchy (IDEA §3.2) | VaultKey wrapped per slot; slot AAD = magic, version, vault id, slot kind + params (`format.rs` `slot_aad`) | `change_password_keeps_vault_key`, `rotate_replaces_vault_key_and_all_slots`, `hello_slot_unlocks_with_signature` |
| No KDF downgrade from a file | `KdfParams::validate` on read; password slot re-wrapped on unlock when weaker than the default (D6) | `kdf_downgrade_is_rejected` |
| Parser never panics | Bounds-checked `Reader`, slot count 1..=8, unique kinds, fixed wrapped length | `parser_never_panics_on_garbage`; `cargo-fuzz` targets in `crates/xpass-core/fuzz` (T33) |
| Recovery key typos caught before KDF | Checksum in `recovery::decode` | `recovery_key_typo_is_caught_by_checksum` |
| Online guessing slowed | `throttle.rs`: 0, 0, 1, 2 … 30 s, shared by unlock and re-auth; in memory (restart resets it — offline guessing is bounded by Argon2, not by this) | `throttle.rs` tests |
| Strong master password | zxcvbn score ≥ 3 (`strength.rs`), also for the KDBX export password | `strength.rs` tests |
| Hello is a key, not a yes/no (IDEA §3.1) | `KeyCredentialManager` sign → HKDF → KEK (`hello.rs`); no `UserConsentVerifier` | `hello_slot_unlocks_with_signature`; manual v0.3 checklist |
| Secrets stay in Rust | `list_entries` sends metadata; `get_entry` sends no password, no secret custom field value and no history password (it does send notes and non-secret field values); `reveal_secret` / `copy_field` by reference (`FieldRef`); import preview has no passwords / notes; export returns only the file name. Plaintext that does cross the boundary on purpose: a revealed secret, the edit form's typed password and field values, TOTP codes, a password made by `generate_password` (shown to the user) and text sent back to `copy_text` to copy it | `entries.rs` tests (`list_is_metadata_only_sorted_and_skips_tombstones`, `secret_custom_field_is_never_in_details`), `state/import.rs` tests |
| WebView cannot reach the network or files | CSP (`tauri.conf.json`: `default-src 'self'`, `connect-src` IPC only, `script-src 'self'`; `style-src` allows `'unsafe-inline'`), capabilities limited to events + window controls (`capabilities/default.json`), no `fs` / `shell` / `dialog` / `opener` / `http` plugins; links opened by Rust after an http(s) check (`entries::normalize_url`, `shell.rs`) | Manual baseline check per release |
| Clipboard | Excluded from history, cloud and monitoring; cleared after the timeout and on lock only if the sequence number is unchanged (`clipboard.rs`) | `clipboard.rs` tests; manual `Win+V` check |
| Auto-lock | Idle (`GetLastInputInfo`: input anywhere on the PC, so working in other apps keeps the vault unlocked; can be set to never), `WTS_SESSION_LOCK`, display off, `PBT_APMSUSPEND`, app exit (`lock_watch.rs`). Closing the window hides to the tray without locking (setting) | Manual v0.1 checklist |
| Wipe on lock | `UnlockedVault` (VaultKey `Zeroizing`), `Payload` / `Entry` / `HistoryItem` / `CustomField` `ZeroizeOnDrop`; pending import dropped; clipboard cleared | Code review (this document) |
| PAT handling | Credential Manager only; never returned to the WebView, written to settings, logged or put in errors; `Debug` redacts it (`secrets.rs`, `github.rs`) | `github.rs` redaction test |
| Atomic writes | Temp file + flush + rename; backup before each write (`storage.rs`); exports in the target folder, never `%TEMP%` | `storage.rs` tests |
| Single instance, no devtools in release | `tauri-plugin-single-instance`; `devtools` feature not enabled | Manual baseline check |
| Signed updates only, after a click (v0.7) | `tauri-plugin-updater` called from Rust only (no `updater:*` capability), minisign public key in `tauri.conf.json`, `requireSignedVersion`, URL and version checks, pre-releases refused (`update.rs`); the vault is locked and pending edits pushed before the installer starts | `update.rs` tests; v0.7 manual checks (tampered installer, other key, signed-version mismatch, WebView `plugin:updater|check` refused); `verify-updater` job on each release |
| No crash dump with secrets (v0.7) | `WerSetFlags(NOHEAP)`, `WerAddExcludedApplication` per user, `WerRegisterExcludedMemoryBlock` for every `LockedKey` page (`crash_dump.rs`, `xpass-core::secret::set_page_hooks`); the NSIS uninstaller removes the exclusion | `secret.rs` hook test; manual crash test (debug `XPASS_CRASH_TEST=1`) left no dump, report or event |
| Binaries signed by a published key (v0.7) | Authenticode (self-signed, SHA-256, RFC 3161 timestamp) in the release build; `scripts/verify-authenticode.ps1` checks every installer and the exe files inside the NSIS one before the draft release is published | Release workflow; user guide "Check the download" |
| Release provenance (v0.7) | `SHA256SUMS.txt`, CycloneDX SBOMs, `actions/attest-build-provenance` and `attest-sbom` per release | `gh attestation verify` (user guide "Check the download") |
| TOTP codes without the secret (v0.8) | `otp.rs`: RFC 4226 / 6238, parser with size and label limits; `totp_code` and `copy_field` (`totp`) return or copy only the current code; the add dialog sends the secret once and gets back the normalized URI | `otp.rs` RFC vectors, `entries.rs` / `state.rs` TOTP tests; fuzz target `otp_parse` (its first run found a length bug, fixed with a regression file) |
| Custom field edits without revealing secrets (v0.8) | `save_entry` fields as `set` or `keep { index, name }`: a stored secret value never has to reach the WebView to be kept, renamed or reordered; limits checked before any change | `entries.rs` field tests (stale `keep` → `not_found`, limits, redacted `Debug`) |
| Password health by id only (v0.8) | `health.rs`: passwords copied into `Zeroizing` buffers outside the session lock and wiped after the check; the result carries entry ids only | `health.rs` tests (`the_report_carries_no_password`), `state.rs` cache test |
| Newer payload schema refused (v0.8) | `vault.rs` `decode_payload`: unlock and sync refuse `schema` above the one the build knows, instead of saving it back as schema 1 | `newer_payload_schema_is_refused` |
| Old vault versions removable from GitHub (v0.7) | `history.rs`: parentless commit of the same tree, head re-checked before a forced ref update, vault read back; only when this device is synced and clean, after re-auth | `history.rs`, `state/sync.rs` tests; `xpass-core/tests/sync.rs` remote-moved-back tests |

## 5. Accepted limits

Known gaps that 1.0 does **not** close. Each was weighed against its cost.

| Limit | Impact | Why accepted / what reduces it |
|---|---|---|
| Malware as the user (A5) | Full compromise while unlocked; PAT readable any time | No user-mode defence exists; lock early. |
| JavaScript strings cannot be wiped; WebView2 is a separate process | A revealed password or the edit form's password may stay in WebView memory until GC | Secrets reach the WebView only on explicit reveal / edit; auto-hide 15 s. |
| Tauri IPC buffers (request and response JSON) are not wiped | Passwords sent to Rust (unlock, re-auth, edit) and revealed secrets leave copies in freed memory (Tauri parses the request into a `serde_json::Value` and deserializes each argument from it) | On the Rust side every secret argument is moved into a `Zeroizing` buffer or a `ZeroizeOnDrop` struct as soon as the command runs; the framework's buffers stay. |
| `serde_json` scratch buffer when decoding escaped strings; `csv` crate buffers | Fragments of plaintext in freed memory | No API to wipe them; short-lived. |
| Decrypted payload not page-locked | Entry plaintext could be written to the pagefile | Since T32 the VaultKey has its own locked page (`xpass-core::secret::LockedKey`); locking the payload needs a custom allocator. |
| Hibernation | `hiberfil.sys` contains all RAM, locked pages included | xPass locks on suspend; hibernation after the lock contains no key. |
| Crash dumps outside WER (v0.7) | An administrator's `LocalDumps` policy that ignores the exclusion, a debugger, or Task Manager "Create dump file" can still dump the unlocked process | WER is excluded (§4); the others need admin rights or the user's own session (A5). |
| WebView2 crash reports | WebView2 (Crashpad) may keep crash reports of its own processes in its user-data folder | No documented switch turns them off; the WebView holds metadata and, briefly, a revealed secret only. |
| MSI uninstall keeps the WER exclusion | The per-user `ExcludedApplications` value for `xpass.exe` stays after an MSI uninstall | Harmless (it only stops WER reports for a file of that name); the NSIS uninstaller removes it. |
| Git history and local backups keep old encrypted copies | An old password (A2) opens old copies | Rotate after a suspected compromise, then **Remove old versions from GitHub** (v0.7). That rewrite cannot reach what GitHub still serves by commit SHA for a while, forks, clones or caches, and it removes the history of other files in the same branch too; a full purge needs GitHub Support. Local backups after a rotation are still the old key's copies (last 10). |
| Metadata leaks to GitHub | Commit times, file size ≈ entry count, IP address | Accepted for personal use (IDEA §3.3). |
| `Payload.extra` / `Entry.extra` (unknown fields from newer versions, D5) not zeroized | Only matters for fields written by a future version | `serde_json::Value` has no zeroize support. |
| TOTP secret in the same vault as the password (v0.8) | Whoever opens the vault (A2, A4, A5) has both factors of that account: two-factor login becomes one factor | The user's choice per entry; README and the user guide say so. Keeping the TOTP in a separate authenticator keeps the factors apart. |
| TOTP codes reach the WebView (v0.8) | A code is shown and may stay in WebView memory | Codes are valid for one period (usually 30 s) and only while the entry is open; the secret stays in Rust except on an explicit reveal in the edit form. |
| Custom fields have no history (v0.8) | Changing or deleting a secret field loses the old value (except in local backups and git history) | The form asks first; `HistoryItem` cannot hold fields without a schema change (FORMAT.md §9). |
| Payload schema checked only from v0.8 | Builds up to v0.7 would open a newer schema and save it back as schema 1 | FORMAT.md §9.5: a breaking payload change also bumps the envelope `version` while such builds may still sync. |
| Plaintext CSV export | Anyone with the file reads everything | Warning + checkbox + re-auth; KDBX offered first; result screen says to delete it. |
| Notes and non-secret custom fields are not secrets | They reach the WebView whenever the entry is open (A4 sees them without a reveal) | By design, like the username; the user guide says to keep secrets in the password or a secret custom field. |
| Deleting the imported CSV is an ordinary delete | On an SSD, in a cloud-synced folder or in backups, copies of the plaintext may remain | The user guide says so and warns against exporting to such places. |
| Release notes in the update dialog are unauthenticated | A party controlling `latest.json` (A10) can show misleading notes or a date | They cannot install anything; the installer and its version are signed. The user guide says so. |
| `settings.json` is not authenticated | Someone who can write the user's files can change the sync repository / branch / path, turn off auto-lock or the startup update check | Needs write access to the user's profile (A5 territory). It cannot point updates elsewhere (fixed endpoint and key in the binary), and a changed sync target holding another vault is refused (`DifferentVault`). |
| Release environment without a required reviewer | A push to `main` starts a release that uses the signing secrets without a second approval | One-person project; `main` is protected by a ruleset and changes reach it only through pull requests. The owner may add themself as required reviewer. |
| No independent security review (1.0) | Mistakes in design or code that the self-review missed | Tests, fuzzing, the self-review (§6) and the format spec; a third-party review stays possible later (v0.7 plan U14). |
| Self-signed builds (v0.7) | The Authenticode signature proves which key signed a file only to someone who checks the thumbprint; Windows reports it as not trusted and SmartScreen still warns | Checksums, build provenance attestations and the update signature stay; users are never asked to trust the certificate. A publicly trusted certificate is optional later (v0.7 plan U8). |
| Hello key name `xpass-<vault_id>` is in the per-user namespace | Another app of the same user can ask to sign with it — but each signature shows the Hello prompt | Spike finding (D9); A5 already wins in that case. |
| Unlock throttle resets on restart | Online guessing at the UI is limited only by Argon2 cost after a restart | ~1 s per guess on the target laptop; the real defence against guessing is the password strength gate. |
| Update manifest size not capped | `latest.json` is read by `tauri-plugin-updater` without a size limit | It comes only from our release on github.com over HTTPS, with a 15 s timeout; its content is checked (signature, URL, version) before anything is used. |
| Moving from v0.6.x | v0.6 has no updater, so the latest release is installed by hand once | User guide "Updates"; the v0.6 check points to the release page. |

## 6. Security self-review (1.0)

Checklist from [IDEA.md §3](IDEA.md#3-design-issues) and the milestone plans, checked again against the code of
1.0.0 (2026-11-26; first written for v0.6). Status: **OK** = done and tested / verified; **limit** = accepted
(§5). The task that delivered an item is named in the evidence.

| # | Item | Status | Evidence / note |
|---|---|---|---|
| R1 | Hello via `KeyCredentialManager` signature → KEK, not `UserConsentVerifier`; only the wrapped VaultKey stored (§3.1) | OK | `hello.rs`, `vault.rs` (`INFO_HELLO`), v0.3 checklist |
| R2 | Random VaultKey wrapped by password / recovery / Hello; header as AAD; KDF params stored and raisable (§3.2) | OK | `format.rs`, `vault.rs`, D6 re-wrap |
| R3 | Change password re-wraps only; rotation re-encrypts with a new key and a new Emergency Kit (§3.3) | OK | `rotate_replaces_vault_key_and_all_slots`; online-only with sync (S6) |
| R4 | Git history leaks old vaults; optional history rewrite (§3.3) | OK | v0.7 T41: Settings → Sync → **Remove old versions from GitHub** (re-auth, synced and clean only, forced update refused by branch protection → nothing changed). What it cannot purge: limit (§5) |
| R5 | Conflict detection via blob SHA, per-entry LWW + tombstones, offline-first (§3.4) | OK | `xpass-core/tests/sync.rs`, `merge.rs`; v0.4 two-device checklist |
| R6 | REST Contents API, fine-grained PAT, `401` handled (§3.5) | OK | `github.rs`; "token expired" banner |
| R7 | Frontend never gets passwords except reveal / edit (§3.6) | OK | §4 "Secrets stay in Rust". Notes and non-secret fields do reach it on open: limit (§5) |
| R8 | Strict CSP, no remote content, minimal capabilities, no `fs`, devtools off (§3.6) | OK | `tauri.conf.json`, `capabilities/default.json`; `style-src 'unsafe-inline'` is the one relaxation (no script) |
| R9 | npm supply chain: minimal deps, audit (§3.6, PLAN §6) | OK | 8 runtime deps; `npm audit` in CI and weekly (v0.6 T31) |
| R10 | Crate advisories and licences (no AGPL / GPL) | OK | `cargo deny` in CI and weekly (v0.6 T31) |
| R11 | `zeroize` on keys and payload; `secrecy`-style care at deserialization (§3.6) | OK | Keys / payload `Zeroizing` / `ZeroizeOnDrop`. IPC arguments: every command moves secret arguments into `Zeroizing` on entry, `Auth` and `EntryInput` are `ZeroizeOnDrop` (the T30 draft listed them as a gap; the T32 check showed the move already wipes the deserialized buffer). T32 fixed what was left: `Debug` of `EntryInput` / `Generated` printed passwords (never logged, now redacted), and the generator built passwords in growing `String`s (reallocation copies). |
| R12 | No plaintext copies from `Vec` growth when saving | OK | T32: `UnlockedVault::save` counts the JSON size first and serializes into an exact-size `Zeroizing` buffer (test `payload_json_is_written_without_reallocation`) |
| R13 | `VirtualLock` for keys (§3.6) | OK | T32: VaultKey in `LockedKey` (own page, locked before the key is written, wiped before unlock); a failure is logged once. Payload: limit |
| R14 | Clipboard excluded from history / cloud / monitoring; clear only if unchanged (§3.7) | OK | `clipboard.rs` + tests |
| R15 | Auto-lock: idle, Windows lock, sleep, exit (§4) | OK | `lock_watch.rs`; also display off (Modern Standby). Idle counts input on the whole PC; close-to-tray keeps it unlocked (user guide "Locking") |
| R16 | Strength check for the master password (§4) | OK | zxcvbn ≥ 3 |
| R17 | Header parser fuzzed (§6 testing) | OK | T33: `cargo-fuzz` targets `parse` (header must re-encode to the same AAD bytes), `unlock_hello`, `recovery_decode`, and since v0.8 `otp_parse`; every target 15 min each Sunday with a cached corpus (v0.7 T42) and on demand; seeds and regressions replayed by `cargo test` on every CI run |
| R18 | Wrong-password throttling | OK | `throttle.rs`; shared with re-auth |
| R19 | Re-auth before export, password change, rotation, new kit, Hello enable (a recovery-key session is not asked again) | OK | `authorize` / `reauthorize` in `state.rs`, `state/reauth.rs` |
| R20 | Import: limits before parsing, no file content in errors or preview, pending rows zeroized and expired | OK | `xpass-interop` tests, `state/import.rs` (`IMPORT_TTL`) |
| R21 | Export: temp + rename next to the target, never `%TEMP%`; lock re-checked before writing | OK | `state/export.rs` |
| R22 | URLs opened only as http(s) with a host, by Rust | OK | `entries::normalize_url`, `shell.rs` |
| R23 | PAT never in logs / settings / errors / WebView | OK | `secrets.rs`, `github.rs` redaction test |
| R24 | Logs carry no secrets | OK | `eprintln!` only for lock reasons, window messages, Hello focus and panic location (panic payload not printed); release builds have no console |
| R25 | Update check from Rust, no WebView network (D10) | OK | Since v0.7 (T39) `update.rs` reads `latest.json` from `github.com/bs135/xpass/releases/latest/download/` through `tauri-plugin-updater` (rustls, HTTPS only, 15 s timeout, no token; no size cap: limit, §5), refuses pre-releases and non-newer versions, opens only `https://github.com/bs135/xpass/releases/…`; startup check opt-in, at most daily; no `updater:*` capability, CSP unchanged |
| R26 | Release integrity | OK | T36: `SHA256SUMS.txt` per release; v0.7 T43: self-signed Authenticode with a published thumbprint (not trusted by Windows: limit, §5); T42: SBOMs and build provenance attestations |
| R27 | Branch protection on `main` while releasing | OK | Ruleset "Protect Production/Main" active (checked 2026-11-26); the release workflow never pushes to `main` (v0.6 T34) |
| R28 | Crash dumps while unlocked | OK | v0.7 T40: WER exclusion, no heap, `LockedKey` pages excluded; other dump paths: limit (§5) |
| R29 | Signed auto-update | OK | v0.7 T38 / T39: minisign-signed installers (the signature also names the version), checked in Rust before running, installed only after a click; `latest.json` itself is not signed (A10); signing key: A11 |
| R30 | SBOM and build provenance per release | OK | v0.7 T42: CycloneDX SBOMs (Rust, npm), `attest-build-provenance`, `attest-sbom`, all in `SHA256SUMS.txt` |
| R31 | Fuzzing runs regularly | OK | v0.7 T42: weekly scheduled run of every target with a cached corpus |
| R32 | TOTP secrets stay in Rust | OK | v0.8 T45 / T47: codes computed in Rust, copy gives the code, reveal only as a secret field; limit: same vault as the password (§5) |
| R33 | Password health leaks nothing | OK | v0.8 T48: ids only, copies wiped, no network check |
| R34 | Format written down and frozen | OK | v0.8 T49: [FORMAT.md](FORMAT.md), checked against the code by an independent review; payload schema now checked |
| R35 | Release pipeline supply chain | OK | Third-party actions pinned to commit SHAs in all workflows (1.0); signing secrets only in the `release` environment with limited deployment branches; no required reviewer: limit (§5) |
| R36 | Recovery documented for users | OK | [USER_GUIDE.md §5](USER_GUIDE.md#5-backups-and-recovery): forgotten password, lost kit, both lost, lost PC, Hello reset, restore, own backups |

Findings of the v0.6 review: `sync_set_token` (PAT) and `copy_text` take a plain `String` over IPC. Rechecked in
T32: like the other commands they move it into `Zeroizing` on entry, so R11 needed no signature changes.

Findings of the 1.0 review (a documentation and process review of README, SECURITY.md, this document, FORMAT.md,
PLAN.md, the links and the release script): notes were wrongly listed as secrets (fixed here: they are shown on
open, §1, §5); `latest.json` was described as signed (A10, R29); R25 and R27 were out of date; the backup path was
wrong; third-party actions were referenced by movable tags (R35, pinned); unauthenticated release notes,
`settings.json` and the ordinary CSV delete were not listed as limits (§5); user recovery was not documented (R36).
No code change was needed for 1.0.

## 7. Reporting a vulnerability

Report it privately through GitHub ("Security" tab → "Report a vulnerability") rather than in a public issue.
Supported versions, what to include, response times and scope: [SECURITY.md](../SECURITY.md).

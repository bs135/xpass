# xPass — Idea Review

Status: **idea validation** (before implementation planning).
Goal of this document: lock down the concept, assess feasibility, and adjust the design and tech stack.

## 1. Original concept

A lightweight, self-sovereign personal password manager built with **Tauri (Rust + Web)** for Windows,
using a private GitHub repository as encrypted cloud storage.

- All encryption/decryption happens locally in the Rust backend.
- Encrypted vault (`vault.enc`) stored in a private GitHub repo: free sync, availability, commit history.
- `Argon2id` key derivation + `AES-256-GCM` authenticated encryption.
- Quick unlock with Windows Hello after the initial master password setup.
- Clipboard protection: auto-clear and exclusion from Windows Clipboard History.
- `zeroize` for memory hygiene; GitHub PAT stored in Windows Credential Manager.
- Stack: Tauri, Rust (`argon2`, `aes-gcm`, `zeroize`, `keyring`, `reqwest`, `windows`), TypeScript + React + Tailwind, GitHub REST API.

## 2. Verdict

**Highly feasible.** The Tauri v2 + Rust + React/Tailwind + `keyring` + GitHub sync stack is already proven in
`xmark` and `logledge`; the app shell (release CI, i18n, theming, tray, single-instance, PAT storage via `keyring`)
can be reused almost as-is. There is no blocking technical risk.

Three design points must be fixed before planning:

1. Windows Hello as originally described adds no real security.
2. There is no key hierarchy (a random vault key wrapped by the master password, Hello and a recovery key),
   which makes password change and recovery hard.
3. Multi-device sync and conflict merging are not addressed.

### Decisions (2026-11-14)

| Topic | Decision |
|---|---|
| Vault format | **Custom format** (section 3.2). Export to CSV and KDBX is planned as an escape hatch. |
| Sync | **Multi-device sync with conflict handling**, **offline-first** (section 3.4). |
| Data entry UX (MVP) | **Secure copy only** (section 3.7). Auto-type is a post-MVP option if a spike shows it is feasible. **No browser extension.** |
| Key hierarchy & recovery | **Required**: random VaultKey wrapped by master password, Windows Hello and a recovery key (Emergency Kit) (section 3.2). |
| Mobile access | **Not needed.** Desktop only; KDBX export remains the escape hatch if that changes. |

## 3. Design issues

### 3.1 Windows Hello + Credential Manager (critical)

Credential Manager is **not** protected by Windows Hello. It uses user-scoped DPAPI, so any process running as the
user (including malware) can read its secrets without a fingerprint or PIN. Storing a cached unlock key there makes
Hello a cosmetic prompt and leaves the vault key readable from disk with plain user privileges.

Correct approach (similar to KeePassXC):

- Use `Windows.Security.Credentials.KeyCredentialManager`. Hello creates an RSA key in the TPM; the private key never
  leaves the TPM.
- When enabling Hello, call `RequestSignAsync(<fixed challenge>)`. RSA PKCS#1 v1.5 signatures are deterministic;
  feed the signature into HKDF to get a **wrapping key**, and use it to wrap the vault key. Only the *wrapped* vault
  key is persisted (file or Credential Manager).
- Unlocking requires a signature, and signing requires Hello.
- Do not use `UserConsentVerifier`: it only returns yes/no and produces no key material.

Known risks: in desktop apps the Hello prompt often appears *behind* the app window and needs a foreground
workaround; calling WinRT async APIs through the `windows` crate from Tauri takes effort. **Spike this first.**

### 3.2 Key hierarchy (critical)

Do not encrypt the vault directly with the Argon2-derived key.

```
Master Password ──Argon2id(salt, params)──► KEK_pw    ─┐
Windows Hello   ──sign(challenge)─►HKDF──► KEK_hello  ─┼─ wrap ─► VaultKey (random 256-bit)
Recovery Key (printed) ──────────────────► KEK_rec    ─┘              │
                                                          AEAD-encrypt payload
```

Benefits:

- Changing the master password only re-wraps the VaultKey; no full re-encryption.
- Several independent unlock methods: password, Hello, recovery key.
- **Recovery key / Emergency Kit**: forgetting the master password otherwise means losing everything.

Vault file format:

- Plaintext header: format version, KDF params (memory, iterations, parallelism, salt), list of wrapped keys, nonce.
- The header is bound as **AAD**, so params cannot be tampered with undetected.
- Argon2id starting point: m = 64–256 MiB, t = 3, p = 4; stored in the header so they can be raised later.
- Cipher: AES-256-GCM with a random 12-byte nonce is fine at personal save rates. **XChaCha20-Poly1305**
  (24-byte nonce) removes nonce concerns entirely. Either is acceptable; test vectors are what matters.

### 3.3 Git history leaks old vaults (critical)

Every save is a commit containing the full encrypted blob, and those blobs **stay in history forever**.

- After changing a possibly-leaked master password, **old versions in history still decrypt with the old password**.
  With a key hierarchy it is worse: the old password unwraps the old VaultKey, which still encrypts the new data.
- Therefore a password change due to suspected compromise must offer **VaultKey rotation**. Optionally offer a
  history rewrite (orphan branch + force push), documenting that GitHub may keep old objects reachable by SHA for a while.
- Metadata still leaks: commit timestamps and file size (approximate entry count). Acceptable for personal use;
  document it in the threat model.

### 3.4 Multi-device sync and conflicts

With a single blob, two devices editing concurrently conflict at file level, and git merge cannot help with ciphertext.

- **Detection**: GitHub Contents API `PUT` requires the current blob `sha`; a mismatch returns `409` — built-in
  optimistic locking.
- **Merge**: fetch remote, decrypt both, merge per entry. Each entry has an `id` (UUID), `updated_at`, and a tombstone
  on delete; last-write-wins per entry. **These fields must be in the data model from day one.**
- **Offline-first**: always keep a local cached `vault.enc` plus a "pending push" flag; the app must open offline.

### 3.5 GitHub access: REST API, not git CLI

Unlike `logledge` (which shells out to `git`), the REST Contents API fits xPass better:

- No dependency on an installed git, no working tree.
- A single small file (~100 KB for 500 entries) is within the Contents API's 1 MB limit.
- `sha` gives optimistic locking for free.

PAT: use a **fine-grained PAT** scoped to one repo with only `Contents: Read & Write`. Handle `401` on token expiry.

Put storage behind a `trait StorageBackend` so a local folder, OneDrive or WebDAV backend can be added later
without touching the core.

### 3.6 Frontend memory and Tauri hardening

"Plaintext never touches the frontend" is only partially achievable:

- The entry list (title, username, URL) must reach the WebView to be displayed. **Passwords, TOTP secrets and secure
  notes must not.** The frontend calls `copy_secret(id)` / `reveal_secret(id)`, and Rust writes directly to the clipboard.
- JS strings cannot be zeroized, and WebView2 runs in a separate process. Document this as best-effort.
- For a password manager, **the npm supply chain is the largest risk**. Keep frontend dependencies minimal, use a
  strict CSP, load no remote content, grant minimal Tauri capabilities, do not expose the `fs` plugin to the frontend,
  and disable devtools in release builds.
- `zeroize` limits: `serde_json` and `String` reallocations leave copies in RAM. Use `secrecy` and be careful at
  deserialization boundaries. Optionally use `VirtualLock` to keep keys out of the pagefile.

### 3.7 Clipboard

- `arboard` on Windows supports `exclude_from_history()`, `exclude_from_cloud()` and `exclude_from_monitoring()`.
  Disable cloud clipboard too, not just `Win+V` history.
- On auto-clear, **only clear if the clipboard still holds our value** (compare `GetClipboardSequenceNumber`),
  otherwise the user's later copy gets wiped.

## 4. Missing from the original concept

| Item | Notes |
|---|---|
| **Auto-lock** | Lock after N idle minutes, on Windows lock (WTS session notifications), on sleep/hibernate. Mandatory. |
| **Browser fill** | Biggest UX gap. **Decided:** MVP is secure copy only; global hotkey + auto-type (KeePass-style) may follow if feasible; no browser extension. |
| Password generator, strength check (zxcvbn) | Required for the master password: a leaked repo allows offline brute force. |
| Import / Export | CSV import from Bitwarden, Chrome, KeePass. Export to avoid lock-in. |
| TOTP | Can come after MVP. |
| Mobile access | **Decided:** not needed. |
| Threat model | State what is covered (GitHub leak, stolen device while locked) and what is not (malware running while the vault is unlocked). |

Wording: "zero-knowledge" is usually marketing for server-based products. **"Client-side / end-to-end encrypted"**
is more accurate here.

## 5. Custom format vs KDBX

**Decision: custom format**, with CSV and KDBX export. The analysis below is kept for reference.

Instead of a custom `vault.enc`, use **KeePass KDBX4** via the `keepass` Rust crate (KDBX4 also uses Argon2id + AES/ChaCha).

- **Pros**: no lock-in; readable by KeePassXC and, on mobile, KeePassDX (Android) / Strongbox (iOS); a format proven
  for years; less custom crypto.
- **Cons**: heavier format; KDBX merging is more complex; the multi-unlock key hierarchy from 3.2 must live outside the file.

Honestly: KeePassXC + a git repo already gets close to xPass. xPass's own value is a focused UX, automatic GitHub sync
and correctly implemented Windows Hello. If a custom format is chosen, provide **KDBX/CSV export** as an escape hatch.

## 6. Adjusted tech stack

| Area | Proposal |
|---|---|
| Crypto | `argon2`, `aes-gcm` or `chacha20poly1305`, `hkdf`, `sha2`, `rand`/`getrandom`, `zeroize`, `secrecy` |
| GitHub | `reqwest` (`rustls-tls`, default features off); no need for `octocrab` |
| OS secrets | `keyring` v3 (reuse `credentials.rs` from logledge) |
| Windows Hello | `windows` crate (`Security_Credentials`, `Security_Cryptography`) |
| Clipboard | `arboard` |
| Tauri plugins | `single-instance`, `global-shortcut` (auto-type / quick search), `autostart`. **No `fs`.** |
| Frontend | React + Tailwind + Zustand + i18next as in xmark/logledge, with a minimal dependency set |
| Testing | Crypto and vault-format test vectors; merge tests; fuzzing of the header parser |

Design an `UnlockProvider` trait so Touch ID on macOS can be added later. Tauri is cross-platform; only Hello is Windows-specific.

## 7. Next steps (before planning)

Feasibility spikes (~1–2 days each):

1. **Windows Hello PoC in Tauri**: `KeyCredentialManager` create → sign → stable key across runs; solve the
   prompt-behind-window issue. Highest risk. **Passed** — see [spikes/windows-hello.md](spikes/windows-hello.md)
   (stable across reboot; Tauri-window focus still to verify).
2. **Vault format + crypto**: header, AAD, key wrap/unwrap, test vectors, Argon2 benchmark on the slowest target machine.
   **Passed** — see [spikes/vault-format.md](spikes/vault-format.md) (XChaCha20-Poly1305, Argon2id 128 MiB / t=3 / p=4).
3. **GitHub Contents API**: GET/PUT with `sha`, reproduce `409`, per-entry merge.

Optional spike (post-MVP): auto-type via global hotkey (`SendInput`, target window detection, focus handling).

Decisions: see the table in section 2. All open questions are resolved; the next phase is the feasibility spikes,
then implementation planning.

# v0.3.0 Plan: Windows Hello unlock

Status: **draft** (2026-11-17). Parent: [PLAN.md §4 v0.3](PLAN.md#v03--windows-hello-size-m-12-weeks).
Spike: [docs/spikes/windows-hello.md](spikes/windows-hello.md).

## Goal

Unlock the vault with Windows Hello (face, fingerprint or PIN) instead of typing the master password, using the
Hello-derived KEK design confirmed by the spike. The master password stays the root credential: it is always
offered next to Hello, it is needed to turn Hello on, and it is the fallback whenever Hello fails or is reset.
The security baseline from v0.1/v0.2 stays: strict CSP, minimal capabilities, no new plugins, no key material in
the WebView.

## What already exists

- `xpass-core` already has the Hello slot: `SlotKind::Hello` (format kind 3, `salt` 32 bytes),
  `UnlockedVault::new_hello_salt`, `set_hello_slot(rng, salt, signature)`, `remove_hello_slot`,
  `Unlock::Hello { signature }`, `hello_challenge(vault_id, salt)` and `hello_challenge_for_file(file)`.
  The KEK is `HKDF-SHA256(ikm = signature, salt, info = "xpass/kek-hello/v1")` and wraps the VaultKey with
  XChaCha20-Poly1305 like the other slots (the spike used AES-GCM; only the wrap cipher differs).
- `UnlockedVault::rotate` already clears all slots, so a rotation drops the Hello slot.
- `change_password` and `replace_recovery_key` keep the other slots, so the Hello slot survives them.

v0.3 is therefore mostly app work: the Windows API layer, the session wiring and the UI. No vault format change.

## Decisions

- **Credential per vault**: Hello credential name `xpass-<vault-id>` (spike open item; a dash instead of a
  slash to stay clear of key-container naming rules). Unpackaged apps share
  the credential namespace per user, so the name must be specific. A rotation keeps the vault id; it deletes the
  old credential (its key wraps nothing any more), and re-enrolling creates it again under the same name.
- **Signature stays in Rust**: the WebView only calls `unlock_hello` / `hello_enable` / `hello_disable` and gets
  status codes back. The challenge, signature and KEK never cross IPC; the signature is `Zeroizing` and dropped
  right after HKDF.
- **No lock held while the prompt is open**: Hello sign runs on a blocking thread *before* taking the unlock
  gate, so the password field stays usable while the Hello dialog is open. If the password unlock finishes
  first, the Hello result is discarded (and the pending operation is cancelled if `IAsyncInfo::Cancel` works).
- **Throttle**: Hello unlocks do not count as failed guesses (each attempt needs user verification by Windows,
  and Windows has its own lockout). A Hello signature that does not open the slot (`BadCredentials`) means the
  credential was re-created; it is handled like `NotFound`, not as a wrong password.
- **Enabling needs the current master password** (like change password / new kit), so a walk-up user at an
  unlocked vault cannot silently add a Hello key. Skipped right after a recovery-key unlock, like the other
  sensitive actions. **Disabling** needs no password (it only removes access).
- **Hello reset (`NotFound`)**: the vault is locked, so the slot cannot be removed from the file yet. The app
  remembers "Hello broken" in memory, shows the password form with an explanation, and after the next password
  unlock removes the Hello slot, saves, and offers to set Hello up again.
- **Auto-prompt**: when the vault is locked, has a Hello slot and the window is visible and focused, the unlock
  screen starts the Hello prompt once per lock. Cancel leaves the user on the password form with a
  "Use Windows Hello" button; no repeated prompts.
- **Focus**: port the spike watcher (find `Credential Dialog Xaml Host`, `SetForegroundWindow`) and verify it
  from the Tauri window. `KeyCredentialManager` has no HWND-owned interop (`windows` 0.62 only has
  `IUserConsentVerifierInterop`, which gives a yes/no and no key), so the watcher is the only option.
- **Crate**: add `windows` 0.62 (WinRT: `Security_Credentials`, `Security_Cryptography`, `Storage_Streams`,
  `Foundation`) next to the existing `windows-sys` (Win32). Only used under `cfg(windows)`.
- **Testability**: `AppState` talks to Hello through a small `HelloProvider` trait (`is_supported`,
  `ensure_credential`, `sign`, `delete`). The app uses the WinRT implementation; tests use a fake that returns a
  deterministic signature or a chosen failure.

## Tasks

Each task is one feature branch off an up-to-date `develop` and one PR into `develop`.

### T13 — Hello backend (`feature/t13-hello-core`)

**Add**
- `src-tauri/src/hello.rs`: port of the spike: `is_supported`, `open` / `create` (`xpass-<vault-id>`,
  `FailIfExists` then open), `sign(challenge) -> Zeroizing<Vec<u8>>`, `delete`, status mapping to
  `HelloError { Unavailable, NotFound, Canceled, DeviceLocked, Failed }` (`UserPrefersPassword` → `Canceled`).
  Blocking `.join()` only on `spawn_blocking` threads. Focus watcher from the spike, logging only to
  `eprintln!` in debug builds and never with data.
- `HelloProvider` trait + `WinHello` implementation + `FakeHello` for tests.
- `AppState`:
  - `hello_status() -> { supported, enrolled, broken }` (`enrolled` read from the vault file header, so it works
    while locked; `broken` = in-memory flag from a failed Hello unlock).
  - `unlock_hello()`: read file → `hello_challenge_for_file` → sign (no gate) → `unlock(Unlock::Hello)`.
    `NotFound` / slot mismatch → set `broken`, return `hello_not_found`.
  - `enable_hello(current: Option<&str>)`: authorize → ensure credential → new salt → sign
    `hello_challenge(vault_id, salt)` → `set_hello_slot` → persist. Clears `broken`.
  - `disable_hello()`: `remove_hello_slot` → persist → delete the credential (best effort).
  - After any password/recovery unlock with `broken` set: remove the Hello slot, save, keep a
    `hello_reenroll_suggested` flag for the UI.
  - `rotate_vault_key` result reports `hello_dropped: bool`.
- `SessionInfo` gets `hello: { enrolled, broken }` so the unlock screen knows what to show without an extra call.
- `commands/hello.rs`: `hello_status`, `unlock_hello`, `hello_enable`, `hello_disable`,
  `hello_dismiss_reenroll`; `rotate_vault_key` also returns `helloDropped`.
- `error.rs` codes: `hello_unavailable`, `hello_not_found`, `hello_canceled`, `hello_device_locked`,
  `hello_busy` (a second request while a Hello dialog is open), `hello_failed`.
- `Cargo.toml`: `windows` 0.62 with the WinRT features above.
- Tests (fake provider): enable → lock → Hello unlock; enable needs the password (not after recovery);
  disable removes the slot; `NotFound` sets `broken` and the next password unlock removes the slot;
  wrong signature is not throttled; rotate drops the slot and reports it; change password and new kit keep it;
  Hello unlock while already unlocked is a no-op.

**Done when:** `cargo test --workspace` green; on a real machine (manual) enable + unlock work through the
dev build with a throwaway vault.

### T14 — Settings: turn Hello on/off (`feature/t14-hello-settings`)

**Change**
- `SettingsScreen` → Security: "Windows Hello" block. States: not available on this PC (explain how to set up
  Hello in Windows Settings), off (button "Turn on", asks for the master password with the existing
  `PasswordPrompt`, then the Hello prompt), on ("Turn off" with a confirm dialog).
- Rotate VaultKey: the explanation lists "Windows Hello will be turned off"; when `helloDropped`, after the new
  Emergency Kit screen show "Set up Windows Hello again?" with a button that runs the enable flow.
- After an unlock that removed a broken Hello slot: a one-time notice with "Set up again".
- `src/lib/api.ts`: typed wrappers; i18n (en, vi): `hello.*`, error messages for the new codes.

**Done when:** manual checklist items for enable/disable/rotate pass; CI green.

### T15 — Unlock screen with Hello (`feature/t15-hello-unlock`)

**Change**
- `UnlockScreen` (password mode): when `hello.enrolled && !hello.broken`, a large "Unlock with Windows Hello"
  button on top and the password form below it, always visible. Auto-prompt once per lock when the window is
  visible and focused (`document.visibilityState` + window focus); a manual click always works.
- Failure handling: `hello_canceled` → stay, focus the password field, no error banner; `hello_not_found` →
  explain that Windows Hello was reset or changed, hide the Hello button, ask for the master password;
  `hello_device_locked` → "Windows Hello is locked, unlock it in Windows or use your password";
  `hello_unavailable` / `hello_failed` → generic message, password fallback.
- While the Hello prompt is open the Hello button shows a busy state; typing the password and submitting still
  works.
- i18n (en, vi).

**Done when:** manual checklist passes (including focus of the prompt); CI green.

### T16 — Chore: GitHub Actions off Node 20 (`chore/t16-actions-node24`)

**Change** (`.github/workflows/ci.yml`, `release.yml`)
- `actions/checkout@v4` → `@v7`, `actions/setup-node@v4` → `@v7`, `actions/upload-artifact@v4` → `@v7`,
  `mathieudutour/github-tag-action@v6.2` → `@v7` (all run on `node24`; check the v7 release notes that
  `dry_run`, `default_bump: false` and `release_branches` behave as before).
- `swatinem/rust-cache@v2` already runs on Node 24 (v2.9.x); keep it. `tauri-apps/tauri-action@v0` already runs
  on Node 24; keep `v0` (v1 renamed inputs and changed asset names) and remove the invalid
  `releaseAssetNamePattern` input (in `v0` the valid name is `assetNamePattern`, already set).

**Done when:** CI green on the PR with no Node 20 deprecation warning; the release workflow file passes
`actionlint` (or a `workflow_dispatch` build on the branch succeeds).

### T17 — Docs and release prep (`docs/t17-v0.3-docs`)

- `docs/spikes/windows-hello.md`: close the open items (Tauri window focus, Hello reset, credential name) with
  the results from the manual tests.
- `docs/PLAN.md`: v0.3 status; risk table row "Hello prompt behind Tauri window" updated.
- `CHANGELOG.md` 0.3.0 section, README status; this plan → **done**.

## Manual test checklist (v0.3.0)

Use a throwaway vault: `$env:XPASS_DATA_DIR = "$env:TEMP\xpass-dev"; npm run tauri dev`.

- [ ] Settings → Security on a PC without Hello set up: "not available" with a hint; no button.
- [ ] Turn on: wrong master password is rejected (and throttled like unlock); right password → Hello prompt →
      "On". Cancel the Hello prompt → stays "Off", no change in the vault file.
- [ ] Lock → unlock screen auto-prompts Hello once; success opens the vault. Cancel → password form focused, no
      second prompt; "Unlock with Windows Hello" button prompts again.
- [ ] Hello prompt appears **in front of** the xPass window (auto-prompt and button), both after a manual lock
      and after restoring from tray / minimize with lock-on-minimize on.
- [ ] While the Hello prompt is open, type the master password and unlock: vault opens, nothing breaks when the
      Hello prompt is then cancelled or completed.
- [ ] **Reboot** → start xPass → Hello unlock works.
- [ ] **Hello reset**: remove the PIN in Windows Settings (or delete the credential) → Hello unlock explains the
      reset and asks for the password → after the password unlock the Hello slot is gone and "Set up again" is
      offered → re-enroll works.
- [ ] Wrong finger / face several times until Windows locks Hello → "Windows Hello is locked" message, password
      works.
- [ ] Change password and new Emergency Kit: Hello unlock still works afterwards.
- [ ] Rotate VaultKey: explanation mentions Hello; after rotation Hello is off and re-enroll is offered;
      re-enroll → Hello unlock works; the old recovery key does not.
- [ ] Turn off → Hello button gone from the unlock screen; the credential `xpass-<vault-id>` is deleted.
- [ ] Two vaults (two `XPASS_DATA_DIR`s) with Hello on: each unlocks with its own credential.
- [ ] Recovery-key unlock with Hello on: works; turning Hello on right after a recovery unlock needs no password.
- [ ] Logs (dev console, stderr) contain no signature, KEK or VaultKey bytes.
- [ ] Release build: devtools, context menu and reload still disabled; CSP and capabilities unchanged.
- [ ] CI (after T16): no Node 20 deprecation annotations.

## Out of scope

Hello for sensitive actions while unlocked (reveal / copy / export re-auth), Hello on other platforms, a
"require Hello every N days, then password" policy, `tauri-action` v1 migration, signed installers.
Closing the Windows Hello dialog automatically when the password unlock wins the race (the dialog stays
open until the user cancels or confirms it; either outcome is harmless because the vault is already open).

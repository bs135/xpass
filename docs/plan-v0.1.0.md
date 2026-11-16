# v0.1.0 Plan: Local Vault

Status: **draft** (2026-11-16). Parent: [PLAN.md §4 v0.1](PLAN.md#v01--local-vault-size-l-34-weeks).

## Goal

A usable, secure, **local-only** password manager for Windows: create a vault with a master password and an
Emergency Kit, unlock/lock (with automatic lock), manage entries, copy secrets safely, generate passwords, change
the master password and rotate the VaultKey. No Windows Hello (v0.2), no sync (v0.3).

## Approach

- Reuse the app shell and tooling from **xmark** / **logledge** (Tauri v2, React 19, TS, Tailwind 4 via
  `@tailwindcss/vite`, Zustand, i18next en/vi, lucide-react, release workflow, single instance, tray,
  `disableWebviewDefaults`).
- **Deliberate differences** from those apps: strict CSP instead of `null`; no `fs`, `store`, `dialog` or `opener`
  plugins; all persistence and URL opening go through Rust commands.
- Security-critical logic lives in `crates/xpass-core` (ported from the spikes). `src-tauri` only wires it to the
  OS and the UI.
- Native window decorations in v0.1 (custom title bar can come later).
- Minimal frontend dependencies: `react`, `react-dom`, `@tauri-apps/api`, `zustand`, `i18next`, `react-i18next`,
  `lucide-react`, `clsx`. Password strength is computed in Rust (`zxcvbn` crate), not with an npm package.

## Pre-work (git)

1. Merge `docs/idea-review` into `main` (docs + spikes).
2. Create `develop` from `main`. Each task below = one feature branch → PR into `develop`.
3. Release `v0.1.0`: merge `develop` → `main` (release workflow tags and builds).

---

## Tasks

Each task is one PR. Order matters; T2–T3 can run in parallel with T1 once the workspace exists.

### T1 — Scaffolding and CI

**Add**
- `Cargo.toml` (workspace: `crates/xpass-core`, `src-tauri`), `rust-toolchain.toml` (stable), `.gitignore`.
- `src-tauri/` from the Tauri v2 React-TS template: identifier `com.bs135.xpass`, product name `xpass`,
  window `main` 1000×700, `visible: false` until the frontend is ready.
- `src-tauri/tauri.conf.json` → `security.csp`:
  `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`.
- `src-tauri/capabilities/default.json`: `core:default` + the few window permissions actually used (show, hide,
  set-focus, unminimize, close). Nothing else.
- Frontend: Vite + React + TS + Tailwind 4, `src/i18n/` (en, vi), `src/lib/api.ts` (typed `invoke` wrappers),
  `src/lib/disableWebviewDefaults.ts` (from logledge), base layout and theme (dark/light/system).
- `.github/workflows/ci.yml` (on PR, `windows-latest`): `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
  `cargo test --workspace`, `npm ci`, `tsc --noEmit`, `oxlint`, `vitest run`.
- `.github/workflows/release.yml`: adapted from xmark, **Windows only** (NSIS + MSI), triggered on `main`.

**Done when:** `npm run tauri dev` opens an empty window with the theme and language switch; CI is green on a PR.

### T2 — `xpass-core`: format, crypto, model

Port `spikes/vault-format` into `crates/xpass-core` with these changes:

- `thiserror` for errors; module layout `format`, `crypto`, `recovery`, `model`, `vault`.
- Model per [PLAN.md §3.4](PLAN.md#34-data-model-payload-schema-1): `urls: Vec<String>`, `favorite`,
  `fields: Vec<CustomField>`, `history: Vec<HistoryItem>` (max 20), `extra: serde_json::Map` with
  `#[serde(flatten)]` on `Payload` and `Entry` (D5). `extra` is `#[zeroize(skip)]`; document that unknown fields
  are not zeroized.
- `UnlockedVault::open_sibling` (from the sync spike) kept for v0.3.
- `KdfParams::DEFAULT` = 128 MiB / t=3 / p=4; `UnlockedVault::password_slot_needs_rewrap()` (D6).
- Tests: port all 14 spike tests + `open_sibling` tests; add D5 round-trip test (unknown fields survive
  unlock → save); regenerate the golden vector for the new schema.

**Done when:** `cargo test -p xpass-core` green; golden vector committed.

### T3 — `xpass-core`: merge, clock, maintenance

- `merge.rs` from the sync spike, extended with **D1**: when both sides have the same id with different content,
  the winner keeps its content and the loser's `password`/`username` are appended to the winner's `history`
  (deduplicated, capped at 20). Merge must stay commutative and idempotent (history sorted by `at`, then content).
- `clock.rs` (**D2**): `next_timestamp(now_ms, last_seen_ms) = max(now_ms, last_seen_ms + 1)`;
  `Payload::max_updated_at()`.
- `maintenance.rs` (**D3**): `purge_tombstones(payload, now_ms, 180 days)`.
- Tests: port the merge tests and the randomized algebra test (now including history), plus D1/D2/D3 unit tests.
- Engine/sync code is **not** ported yet (v0.3).

**Done when:** tests green, including the randomized commutative/idempotent/associative check.

### T4 — Vault session and local storage (Rust)

**Add** in `src-tauri/src/`:
- `paths.rs`: `%APPDATA%\xpass\` (via `app.path().app_data_dir()`), `vault.xpv`, `backups/`, `settings.json`.
- `storage.rs`: atomic write (temp + rename), keep the last 10 backups (`vault-<yyyyMMdd-HHmmss>.xpv`).
- `session.rs`: `VaultSession { vault: UnlockedVault, last_seen_ts }` with `apply(edit)` that stamps
  `updated_at` via D2, saves and writes atomically. `AppState { session: Mutex<Option<VaultSession>> }`.
- `commands/session.rs`: `vault_status` (`NoVault | Locked | Unlocked`), `create_vault`, `unlock_password`,
  `unlock_recovery`, `lock`. Argon2 runs in `spawn_blocking`.
- Wrong-password throttling: in-memory delay 0 / 1 / 2 / 4 / 8 / … max 30 s after consecutive failures.
- `error.rs`: errors serialized to the frontend as stable codes (`bad_credentials`, `tampered`,
  `bad_recovery_key`, `io`, …), never with secret data.
- On `unlock_password`: if `password_slot_needs_rewrap()`, re-wrap with the default params and save (D6).

**Done when:** command handler tests (temp dir) cover create → lock → unlock, wrong password, tampered file,
backup rotation.

### T5 — Setup, Emergency Kit, unlock UI

- `password_strength(pw)` command (`zxcvbn` crate) → score 0–4 + feedback; setup requires score ≥ 3.
- **Setup** screen: master password + confirm + strength meter → `create_vault`.
- **Emergency Kit** screen: recovery key in groups, explanation, "Print / Save as PDF" (`window.print()` on a
  print-only layout), then confirmation by re-typing 2 random groups before continuing.
- **Unlock** screen: password field, error/throttle message, "Use recovery key" link.
- **Recovery unlock** → forced "Set a new master password" (`change_password`), then offer a new Emergency Kit
  (rotate the recovery slot only).
- Frontend session store reacts to `vault://locked` (clears all entry state).

**Done when:** a new user can create a vault, print the kit, lock, and unlock with password or recovery key.

### T6 — Entries

- Commands: `list_entries` → metadata only (`id, title, username, url_host, tags, favorite, updated_at`; no
  tombstones); `get_entry` → all fields **except** `password` and secret custom fields; `save_entry(input)` where
  `password: Option<String>` (`None` = unchanged) and a changed password is pushed to `history`;
  `delete_entry` (tombstone); `set_favorite`.
- UI: two-pane layout (list + detail), search over metadata (title, username, URL, tags), favorites filter,
  create/edit form with show/hide password, delete with confirm.
- `open_url(id, index)` command: opens only `http(s)` URLs with the OS default browser (no `opener` plugin).
- Shortcuts: `Ctrl+F` search, `Ctrl+N` new, `Ctrl+E` edit, `Del` delete, `Ctrl+L` lock.

**Done when:** CRUD works and survives restart; the WebView state contains no passwords unless an edit form or a
reveal is open (checked in devtools during development).

### T7 — Secure copy, reveal, generator

- `clipboard.rs`: `arboard` with `exclude_from_history()`, `exclude_from_cloud()`, `exclude_from_monitoring()`;
  store `GetClipboardSequenceNumber` after writing; clear after the timeout (default 20 s) **only if** the
  sequence number is unchanged; also clear on lock and on exit.
- `copy_field(id, field)` (username, password, url, custom field) and shortcuts `Ctrl+B` (username),
  `Ctrl+C` on a selected entry (password).
- `reveal_secret(id, field)`; UI hides it again after 15 s or on blur.
- `generator.rs` (`OsRng`): character mode (length 8–128, sets, exclude ambiguous) and passphrase mode
  (EFF large wordlist embedded, separator, capitalize, add digit); entropy estimate shown in the UI.
- Generator available as a standalone panel and inside the entry form.

**Done when:** a copied password does not appear in `Win+V` history; it is cleared after the timeout unless the
user copied something else meanwhile.

### T8 — Auto-lock

- `lock_watch.rs`: a dedicated thread with a message-only window (`HWND_MESSAGE`) that registers
  `WTSRegisterSessionNotification` and `RegisterSuspendResumeNotification`; locks on `WTS_SESSION_LOCK` and
  `PBT_APMSUSPEND`.
- Idle lock: check `GetLastInputInfo` every 10 s; lock after the configured timeout (default 5 min, options
  1/5/15/30/60 min, never).
- Lock = drop the session (zeroize), clear clipboard if ours, emit `vault://locked`.
- Optional setting: lock when minimized to tray (default off).

**Done when:** manual checklist passes: `Win+L`, sleep, idle timeout, quit.

### T9 — Security and general settings

- `settings.rs`: `settings.json` (serde) for auto-lock timeout, clipboard timeout, lock-on-minimize, theme,
  language. No secrets.
- Settings UI: **Security** (change password with current-password check; rotate VaultKey with explanation →
  new Emergency Kit flow; show "last kit generated" date), **General** (theme, language, timeouts).
- `change_password` and `rotate_vault_key` commands verify the current password first.

**Done when:** after rotate, the previous recovery key no longer unlocks the vault and a backup taken before the
rotate still opens with the old credentials (expected, explained in the UI).

### T10 — Tray, single instance, hardening, release

- Tray menu: Open, Lock, Quit; window close → hide to tray (setting), `tauri-plugin-single-instance` focuses the
  existing window.
- Hardening review: CSP active (no inline scripts), capabilities minimal, devtools absent in release build,
  `disableWebviewDefaults` on, no `console.log` of entry data, panic hook without secrets.
- App icon, About section (version, license, repo link via `open_url`).
- README (features, security model summary, build), CHANGELOG.
- Release `v0.1.0`.

**Done when:** release workflow produces the Windows installer; manual test checklist below passes on a clean
Windows user profile.

---

## Manual test checklist (v0.1)

- [ ] First run → setup → kit printed → confirmation required.
- [ ] Restart → unlock with password; wrong password shows throttle delay.
- [ ] Unlock with recovery key → forced new password → new kit offered.
- [ ] Modify one byte of `vault.xpv` → clear "file corrupted or modified" error, backup restore hint.
- [ ] Entry CRUD, search, favorites, open URL.
- [ ] Copy password → not in `Win+V`; cleared after 20 s; copying something else in between is not wiped.
- [ ] `Win+L`, sleep, idle timeout → app locked, entry list cleared.
- [ ] Change password; rotate VaultKey → old recovery key rejected.
- [ ] Tray: close hides, Lock works, second launch focuses the existing window.
- [ ] Release build: devtools unavailable, context menu and reload disabled.

## Risks

| Risk | Mitigation |
|---|---|
| Message-only window for WTS/power events is fiddly | Isolate in `lock_watch.rs`; fall back to polling `OpenInputDesktop` if needed. |
| Strict CSP breaks Vite dev or Tailwind | Dev uses Tauri's dev CSP handling; verify the release build early (T1). |
| Secrets leaking into the WebView | Command contract (T6) + devtools check during development + review in T10. |
| `serde(flatten)` with `extra` and zeroize | `extra` skipped by zeroize, documented; only unknown future fields live there. |

## Out of scope for v0.1

Windows Hello, sync, import/export, password history UI, custom field UI, TOTP, auto-type, custom title bar.

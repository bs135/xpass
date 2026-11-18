# xPass — Implementation Plan

Status: **approved** (2026-11-16). Inputs: [IDEA.md](IDEA.md) (decisions) and the three passed spikes:
[windows-hello](spikes/windows-hello.md), [vault-format](spikes/vault-format.md), [github-sync](spikes/github-sync.md).

Each milestone is a runnable, releasable app. MVP = **v0.1 → v0.4**, complete with v0.4.0 (2026-11-18).

---

## 1. Scope

### MVP (v0.1 – v0.4)

- Local encrypted vault: create, unlock (password / recovery key), lock, entry CRUD, search.
- Key hierarchy with Emergency Kit (recovery key), change password, rotate VaultKey.
- Secure copy: clipboard excluded from history/cloud, auto-clear only if unchanged.
- Password generator.
- Custom title bar that follows the app theme.
- Auto-lock: idle timeout, Windows lock, sleep/hibernate, app close.
- Windows Hello quick unlock.
- GitHub sync: offline-first, multi-device, conflict merge.

### After MVP

- v0.5: CSV import (Bitwarden, Chrome, KeePassXC), CSV + KDBX export, password history UI.
- v0.6 → 1.0: hardening, security review, signed auto-update.
- Later: auto-type (global hotkey), TOTP, password health (weak/reused), custom fields UI, Touch ID (macOS).

### Out of scope

Browser extension, mobile, sharing between users, self-hosted server.

---

## 2. Decisions

Items the spikes left open. **All accepted on 2026-11-16.**

| # | Topic | Proposal | Rationale |
|---|---|---|---|
| D1 | Concurrent edit of the same entry | Entry-level LWW **+ keep the losing version in the entry's `history`** | No silent data loss; reuses the password-history feature; simpler than per-field merge. |
| D2 | Clock skew | `updated_at = max(now, last_seen_updated_at + 1)` per vault (HLC-lite) | A device with a slow clock still produces newer timestamps after seeing others. |
| D3 | Tombstones | Purge tombstones older than **180 days** | Bounded growth; a device offline > 6 months may resurrect entries (acceptable). |
| D4 | Compression | **None in format v1**; reserve a payload flag for later | 2000 entries ≈ 750 KB, raw GET has no 1 MB limit. |
| D5 | Forward compatibility | Unknown JSON fields preserved on entries and payload (`#[serde(flatten)] extra`) | An older app version must not drop fields written by a newer one during sync. |
| D6 | Argon2id | `argon2` 0.5, 128 MiB / t=3 / p=4; re-wrap password slot on unlock if below current default | ~0.9 s on the target laptop; Hello covers daily unlocks. |
| D7 | HTTP client | `reqwest` **async** on Tauri's tokio runtime, rustls | Spike used blocking; the app must not block the UI thread. |
| D8 | Sync triggers | On unlock, 5 min timer while unlocked, 10 s after the last edit (debounced), on lock/close (8 s timeout) | Few commits, fresh data; matches logledge's proven cadence. |
| D9 | Hello credential name | `xpass-<vault_id>` | Unpackaged apps share the per-user namespace (spike finding); a dash avoids key-container naming rules. |
| D10 | Update check | Rust command calling GitHub Releases API (not frontend `fetch`) | Keeps a strict CSP with no network access from the WebView. |

---

## 3. Architecture

### 3.1 Repository layout

```text
xpass/
├── crates/
│   └── xpass-core/          # pure Rust, no Tauri: format, crypto, model, merge, sync engine, RemoteStore trait
├── src-tauri/               # Tauri app: commands, state, Hello, clipboard, keyring, GitHub store, auto-lock, tray
├── src/                     # React + TS frontend
├── docs/                    # IDEA, PLAN, spikes, THREAT_MODEL, release notes
├── spikes/                  # kept as reference until v0.4, then removed
└── .github/workflows/       # ci.yml (lint + test), release.yml (Windows build, from xmark)
```

`xpass-core` is the security-critical part. Keeping it free of Tauri/Windows makes it fully unit-testable and
fuzzable on any OS.

### 3.2 Components

```text
┌──────────────────────── WebView (React) ────────────────────────┐
│ screens: Setup · Unlock · Vault list · Entry detail/edit ·      │
│          Generator · Settings (Security, Sync, General)         │
│ state: Zustand (metadata only)        i18n: en / vi             │
└───────────────▲──────────── IPC (tauri commands/events) ────────┘
                │  metadata, never passwords (except explicit reveal / edit form)
┌───────────────┴──────────────── src-tauri ──────────────────────┐
│ AppState (Mutex)  ── VaultSession { UnlockedVault, dirty, ... } │
│ commands/*        lock.rs (idle, WTS, power events)             │
│ clipboard.rs      hello.rs (KeyCredentialManager)               │
│ secrets.rs (keyring: PAT)   github.rs (async RemoteStore)       │
│ sync_worker.rs (timer, debounce, status events)   tray.rs       │
└───────────────▲─────────────────────────────────────────────────┘
                │
┌───────────────┴──────────── crates/xpass-core ──────────────────┐
│ format · crypto · recovery · model · vault · merge · engine     │
└─────────────────────────────────────────────────────────────────┘
```

### 3.3 Local files

| Path | Content |
|---|---|
| `%APPDATA%\xpass\vault.xpv` | Encrypted vault (atomic write, temp + rename) |
| `%APPDATA%\xpass\backups\vault-<ts>.xpv` | Last 10 local versions |
| `%APPDATA%\xpass\sync-state.json` | Remote identity, `base_sha`, `local_sha` (dirty = file SHA differs), `slots_dirty`, last sync time |
| `%APPDATA%\xpass\settings.json` | Non-secret settings (repo, branch, path, timeouts, language, theme) |
| Windows Credential Manager | GitHub PAT (`xpass/github-token`) |
| TPM (Windows Hello) | Hello key `xpass-<vault_id>`; only the *wrapped* VaultKey lives in the vault header |

### 3.4 Data model (payload schema 1)

```text
Payload { schema, revision, entries: [Entry], extra }
Entry {
  id, created_at, updated_at, deleted,
  title, username, password, urls: [String], notes, tags: [String], favorite,
  fields: [CustomField { name, value, secret }],      // stored now, UI later
  history: [HistoryItem { at, password, username?, note? }],   // last 20
  extra                                                // unknown fields preserved (D5)
}
```

### 3.5 IPC contract (commands)

Frontend never holds the VaultKey; it receives passwords only for an explicit reveal or while an edit form is open.

| Group | Commands |
|---|---|
| Session | `vault_status`, `create_vault`, `unlock_password`, `unlock_recovery`, `unlock_hello`, `lock` |
| Entries | `list_entries` (metadata), `get_entry` (no secrets), `save_entry`, `delete_entry`, `set_favorite`, `open_url` |
| Secrets | `copy_field(id, field)`, `reveal_secret(id, field)` (auto-hide in UI after 15 s) |
| Tools | `generate_password(opts)`, `password_strength(pw)` |
| Security | `change_password`, `rotate_vault_key`, `show_recovery_kit`, `hello_status`, `hello_enable`, `hello_disable` |
| Sync setup | `sync_get_config`, `sync_configure`, `sync_set_token`, `sync_disconnect`, `sync_test_connection`, `sync_open_token_page` |
| Sync | `sync_status`, `sync_now`, `sync_dismiss_password_changed`, `sync_join`, `sync_resolve_key_change`, `sync_replace_local`; events `sync://status`, `vault://changed` |
| Backups | `list_backups`, `restore_backup` |
| App | `settings_get/set`, `check_update` |

---

## 4. Milestones

Sizes assume one developer working part-time; they are for ordering, not commitments.

### v0.1 — Local vault (size: L, ~3–4 weeks)

**Goal:** a usable, secure, local-only password manager.

1. Scaffolding: Tauri v2 + React + TS + Tailwind + Zustand + i18next (from xmark/logledge), Cargo workspace with
   `xpass-core`, CI (`cargo fmt/clippy/test`, `oxlint`, `tsc`), release workflow adapted from xmark (Windows only).
2. `xpass-core`: port vault-format spike (format v1, crypto, recovery, model with D1/D5 fields), merge (D1–D3),
   golden vector, fuzz-style tests.
3. Setup flow: master password (zxcvbn strength gate), Emergency Kit screen (show + print/save PDF, confirm by
   re-typing 2 groups).
4. Unlock / lock: password, recovery key; wrong-password delay; lock on idle (default 5 min), Windows lock
   (`WTSRegisterSessionNotification`), sleep (`WM_POWERBROADCAST`), window close → tray.
5. Entries: list with search (title, username, URL, tags), detail, create/edit/delete (tombstone), favorites.
6. Secure copy: `arboard` with `exclude_from_history/cloud/monitoring`, clear after 20 s if sequence number unchanged.
7. Password generator (length, sets, passphrase mode).
8. Security settings: change password, rotate VaultKey (new Emergency Kit), auto-lock timeout.
9. Hardening baseline: strict CSP, minimal capabilities, no `fs`/`shell` plugins, devtools off in release,
   single instance.

**Acceptance:** vault survives restart; any byte flip in `vault.xpv` is reported, not crashed on; copied password
absent from `Win+V` and cleared after timeout; lock triggers verified manually; all core tests green.

### v0.2 — Custom title bar (size: S)

Self-drawn title bar merged into the header, following the app theme; no new plugins, minimal window
permissions. Details: [plan-v0.2.0.md](plan-v0.2.0.md).

### v0.3 — Windows Hello (size: M, ~1–2 weeks)

Implemented in T13–T16; details and the manual checklist: [plan-v0.3.0.md](plan-v0.3.0.md).

1. `hello.rs`: port spike (create/open/sign via `windows` crate, async `join` off the UI thread), focus watcher.
2. Enable flow (after password unlock): salt → sign → `set_hello_slot` → save. Disable: remove slot + delete credential.
3. Unlock screen: Hello button as default when the slot exists; password fallback always visible.
4. Failure handling: `NotFound` (Hello reset) → remove slot, explain, offer re-enroll; `UserCanceled` → stay on
   password screen; `SecurityDeviceLocked` → message.
5. Verify prompt focus from the Tauri window (open item from the spike).

**Acceptance:** Hello unlock works after reboot; Hello reset falls back cleanly; rotate drops Hello and prompts
re-enroll.

### v0.4 — GitHub sync (size: L, ~3 weeks) → **MVP complete**

Implemented in T18–T23; decisions S1–S15, details and the manual two-device checklist:
[plan-v0.4.0.md](plan-v0.4.0.md). Two decisions shape the behaviour beyond this outline:

- **Windows Hello stays per PC (S4).** A synced file may carry the Hello slot of the PC that pushed it; every other
  PC ignores it and keeps its own, and joining drops it. No format change.
- **Rotation is online when sync is on (S6).** The rotated file must reach GitHub before the local file changes
  (offline → refused, nothing changed); the other PCs then ask for the master password once and keep their local
  edits (S7).

1. `github.rs`: async port of the spike store (raw GET + local blob SHA, PUT with SHA, 409/422 → Conflict), retry
   unreadable GET once after 2 s.
2. Sync settings: repo `owner/name`, branch, path; PAT via hidden input → Credential Manager; "Test connection"
   (repo access + Contents write check); guide to create a fine-grained PAT.
3. First-run choice: **create new vault** (push) or **join existing vault** (clone, unlock with password, then
   optional Hello enroll on this device).
4. Sync worker (D8): offline-first engine from the spike, status events (Synced / Syncing / Offline / Error /
   Needs password), tray "Sync now".
5. Flows: `KeyChanged` → ask for the password, merge, keep local edits; `401/403` → "token expired" prompt;
   `DifferentVault` → stop and explain.
6. History: local backups (last 10); "restore previous version" from local backups.

**Acceptance:** two machines converge after concurrent offline edits; race (both sync at once) converges; PAT never
appears in logs/settings; app fully usable offline.

### v0.5 — Import / Export (size: M, ~2 weeks)

CSV import with column mapping presets (Bitwarden, Chrome, KeePassXC, generic), duplicate detection; CSV export
(plaintext warning + confirm); KDBX4 export via `keepass` crate; password history UI per entry.

### v0.6 → v1.0 — Hardening & release (size: M, ~2 weeks)

- `docs/THREAT_MODEL.md`; security self-review against IDEA.md §3; `cargo audit`, `cargo deny`, `npm audit`
  in CI; `cargo-fuzz` target for the header parser.
- Memory: `VirtualLock` for key material, review copies of secrets at IPC/serde boundaries.
- Update check (D10); optional signed updater later.
- Installer: NSIS; document SmartScreen warning for unsigned builds.
- README, CHANGELOG, screenshots; remove `spikes/`.

---

## 5. Testing strategy

| Layer | How |
|---|---|
| `xpass-core` | Unit tests ported from spikes (format, tamper, rotation, merge algebra, races, offline); golden vector freezes format v1; fuzz target (v0.6). |
| Sync | Engine tests with `MemoryStore`; manual two-device checklist against a private test repo each release touching sync. |
| Tauri commands | Rust tests on command handlers with a temp app dir. |
| Windows-specific | Manual checklist per release: Hello enroll/unlock/reset, clipboard history exclusion, lock triggers, tray. |
| Frontend | Vitest for stores/utilities; no E2E in MVP. |

---

## 6. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Frontend supply chain (npm) | Secrets exfiltrated from WebView | Minimal deps, lockfile, `npm audit`, strict CSP + no network from WebView, no secrets in frontend state. |
| Bug in merge loses data | High | Algebraic tests, history copy (D1), local backups, git history as last resort. |
| Hello prompt behind Tauri window | UX | Focus watcher from the spike; verified in v0.3: the prompt shows in front of the window. |
| Forgotten master password and lost Emergency Kit | Total loss | Kit is mandatory at setup with confirmation; reminder in Settings. |
| GitHub PAT expiry | Sync stops | Sync pauses with a "token expired" banner; edits stay local; a new token resumes (v0.4). |
| Public repository chosen by mistake | Encrypted vault and its history readable by anyone | Test connection flags it; saving needs an explicit acknowledgement (v0.4). |
| VaultKey rotated while another PC has unsynced edits | Edits encrypted with a discarded key | Rotation only online with sync on (S6); the other PC merges after the password prompt (S7). |
| Restored backup loses to newer copies on other PCs | Restore undone by the next sync | Restore is a new edit with fresh timestamps (S14). |
| Clock skew | Wrong LWW winner | D2 + history copy (D1). |

---

## 7. Workflow

- Branches: `main` (releases) ← `develop` ← feature branches, PRs as in xmark/logledge.
- Conventional Commits; release workflow bumps version and tags from commit types (as in xmark): `feat` → minor,
  `fix`/`perf` → patch. A milestone with a `feat` is therefore a minor release (`v0.X.0`); milestones are numbered
  to match.
- One `docs/plan-vX.Y.Z.md` per milestone with the detailed task list, written when the milestone starts.

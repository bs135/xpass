# v0.4.0 Plan: GitHub sync

Status: **done** (2026-11-18). Parent: [PLAN.md §4 v0.4](PLAN.md#v04--github-sync-size-l-3-weeks--mvp-complete).
Spike: [docs/spikes/github-sync.md](spikes/github-sync.md), code in [`spikes/github-sync`](../spikes/github-sync).

## Goal

Keep one vault in sync across several Windows PCs through a file in a private GitHub repository, using the
offline-first design the spike proved: every edit is local first, sync reconciles whenever GitHub is reachable,
concurrent edits merge per entry (D1–D3), and credential changes on another device are handled without data loss.
This completes the MVP.

The security baseline stays: the WebView has no network access (CSP unchanged), Tauri capabilities stay minimal,
every network call happens in Rust, and the GitHub token (PAT) never reaches logs, `settings.json`, the vault
file, error messages or the frontend after the user typed it.

## What already exists

- `xpass-core`: per-entry LWW `merge` with conflict history (D1), `clock::next_timestamp` (D2),
  `maintenance::purge_tombstones` (D3), `UnlockedVault::open_sibling` (adopts the other file's slots;
  `DifferentVault` / `KeyChanged` / `Tampered`), `rotate`, Hello slot helpers.
- `src-tauri`: `AppState` with a `Mutex<Option<VaultSession>>`, `VaultSession::apply` / `persist_with`
  (clone → change → save → atomic write → swap), `Storage` with 10 rolling backups, `settings.json`,
  tray with Open / Lock / Quit, lock watcher (idle, Windows lock, sleep, exit).
- Spike: blocking `GitHubStore` (raw GET + local blob SHA, PUT with `sha`, 409/422 → `Conflict`),
  `MemoryStore` with offline simulation and a "push between GET and PUT" hook, the sync loop and 12 tests.

## Decisions

- **S1 — Engine in `xpass-core`, transport in `src-tauri`.** Core gets a `sync` module: an async
  `RemoteStore` trait (`get`, `put(bytes, expected_sha)`; native `async fn` in traits, no tokio in core),
  `blob_sha`, a pure reconcile step and a small async driver. `GitHubStore` (reqwest) lives in the app.
  Core tests use `MemoryStore` and a ten-line `block_on` with `Waker::noop()` (no executor dependency).
- **S2 — Snapshot / commit around the network.** The session mutex is never held across an `await`. A sync
  takes a snapshot (vault clone, `base_sha`, dirty, an edit generation counter), talks to GitHub, then commits
  under the lock: if the user edited meanwhile (generation moved), the pushed result is merged into the current
  session with `merge` and the device stays dirty for the next round. If the vault was locked meanwhile, the
  pushed bytes are still written to disk (they are a valid encrypted vault) and `sync-state.json` is updated.
- **S3 — Crash-safe dirty tracking.** `sync-state.json` stores `base_sha` (remote blob SHA last agreed on) and
  `local_sha` (blob SHA of `vault.xpv` right after the last sync). Dirty = `blob_sha(vault.xpv) != local_sha`,
  so a crash between writing the vault and writing the state can only cause one extra (harmless) push, never a
  lost edit. The state also records the remote identity (repo, branch, path, vault id); changing any of them
  resets `base_sha`.
- **S4 — Windows Hello slots are device-local.** Each PC has its own Hello key, so a Hello slot is meaningless
  on another device. Rule: when adopting or merging a remote file, keep the **local** Hello slot and ignore the
  remote one; when joining, drop the remote Hello slot. A pushed file may carry the pusher's Hello slot
  (stripping it would need re-encryption, which a key-less push on lock cannot do); it is useless without that
  PC's TPM key and is ignored by every reader. No format change.
- **S5 — Local password / kit changes win over a concurrent remote one.** A local change of the password or
  recovery slot sets `slots_dirty`; the next merge keeps the local password/recovery slots instead of adopting
  the remote ones, and sync runs right away. Without a local change the remote slots are adopted silently
  (spike behaviour) and the UI shows a one-time notice "The master password was changed on another device".
- **S6 — Rotation is online when sync is on.** Rotating the VaultKey with sync configured: sync first, rotate
  in memory, `PUT` with `expected = base_sha`, and only after success write the local file. Offline or a
  conflicting push → rotation is aborted with a clear message (nothing changed). This avoids the case where
  the remote holds edits encrypted with a key this device already threw away. Without sync, rotation stays
  local as today.
- **S7 — Rotation elsewhere (`KeyChanged`).** Sync stops with status *Needs password*; local edits stay dirty.
  The user types the master password (current on the other device): the remote file is unlocked with it, the
  payloads are merged (our decrypted payload is still in memory), the remote key and slots are adopted, the
  local Hello slot is dropped (it wraps the old key) and re-enrolling is offered, then the merge is pushed.
- **S8 — Key-less push on lock and exit (D8).** On lock / app exit the session is dropped immediately (no key
  kept for the final sync). If the device is dirty, the worker pushes the local file bytes as they are, with
  `expected = base_sha`, 8 s timeout. If the remote moved on, nothing is pushed (merging needs the key) and the
  next unlock syncs.
- **S9 — Triggers (D8).** One worker task per app, fed by a channel: on unlock, 10 s after the last edit
  (debounced), every 5 min while unlocked, "Sync now" (Settings, header, tray), on lock/exit (S8), right after a
  local password/kit change (S5). Offline → retry with backoff (30 s, 1 min, 2 min, … capped at 5 min).
  Only one sync runs at a time.
- **S10 — GitHub settings.** `settings.json` gets `sync: { repo: "owner/name", branch: "" (= repository
  default branch), path: "xpass/vault.xpv", enabled }`. Repo input also accepts a `https://github.com/owner/name`
  URL. Commit message `xpass: update vault` (no device name, no entry data).
- **S11 — PAT in Windows Credential Manager** via `keyring` 3 (`windows-native`), target `xpass/github-token`
  (debug builds with `XPASS_DATA_DIR` append a hash of the directory, so dev vaults do not share the real
  token). The token is `Zeroizing<String>` in Rust, read from the credential store only when building a
  request, sent only as the `Authorization` header to `api.github.com`. The frontend only learns
  `hasToken: bool`. `GitHubStore` has a hand-written `Debug` without the token. reqwest errors are mapped to
  codes and never formatted into messages.
- **S12 — "Test connection".** `GET /repos/{repo}`: 401 → bad token, 404 → repo not found or not shared with
  the token, `private == false` → warning (the vault is encrypted, but a public repo exposes the file and its
  history to everyone; continue only after a confirm). Then `GET` the vault path: not there → "a new vault will
  be created"; an xpass vault with our `vault_id` → "found, same vault"; another `vault_id` → `DifferentVault`
  explanation. Write access cannot be checked without writing; a `403` on the first `PUT` is reported as
  "token lacks Contents: write".
- **S13 — HTTP client (D7).** `reqwest` 0.13 async on Tauri's tokio runtime, rustls, timeouts 15 s connect /
  60 s total (5 MB tested in the spike), `User-Agent: xpass/<version>`, `X-GitHub-Api-Version: 2022-11-28`.
  Status mapping: 409/422-with-sha → `Conflict`; 401 → `Unauthorized`; 403/429 with
  `x-ratelimit-remaining: 0` or `retry-after` → `RateLimited`; other 403 → `Forbidden`; connect/timeout/DNS →
  `Offline`. A GET body that fails to parse is retried once after 2 s (spike anomaly).
- **S14 — Restore keeps sync semantics.** Restoring a backup must not lose to LWW: core
  `restore(current, backup, ts)` writes, for every entry that differs, the backup's version stamped with a new
  D2 timestamp (current credentials go to `history` with note `restore`), and tombstones entries that did not
  exist in the backup. The result is an ordinary edit and syncs like one. A backup that does not open with the
  in-memory key (older VaultKey) asks for the password it was saved with.
- **S15 — Tombstone purge (D3)** runs on the merged payload before each push.
- **No new Tauri plugins, no capability changes** (`core:event:allow-listen` already covers `sync://status`).

## Tasks

Each task is one feature branch off an up-to-date `develop` and one PR into `develop`.

### T18 — Sync engine in core (`feature/t18-sync-core`)

**Add** `crates/xpass-core/src/sync.rs` (+ `tests/sync.rs`)
- `RemoteStore` async trait, `Remote { bytes, sha }`, `StoreError { Conflict, Offline, Unauthorized,
  Forbidden, RateLimited { retry_after_s }, RepoNotFound, InvalidResponse, Http(status) }` (no free-form
  strings, so nothing secret can leak through an error).
- `blob_sha(bytes)` (git blob SHA-1; add `sha1` dependency).
- `SyncState { base_sha, local_sha, slots_dirty }` (serde) and `is_dirty(local_bytes)` (S3).
- Pure reconcile: `reconcile(local: &UnlockedVault, dirty, slots_dirty, base_sha, remote) -> Step`
  (`UpToDate | Adopt(vault) | Push { vault, expected, outcome }`), with S4 (Hello local), S5 (slot
  precedence), S15 (purge before push).
- Async driver `sync_once(store, local: &mut impl LocalSide) -> Result<SyncOutcome, SyncError>`: snapshot →
  GET → reconcile → PUT → commit, up to 5 attempts on `Conflict`; `LocalSide` is implemented by the app
  (S2) and by an in-memory test double.
- `join(remote_bytes, password) -> UnlockedVault` (drops the remote Hello slot, S4) and
  `merge_after_key_change(ours, remote_bytes, password)` (S7).
- `push_raw(store, local_bytes, base_sha)` for the key-less push (S8).
- `restore(current, backup, ts) -> Payload` (S14).
- `sync::memory::MemoryStore` (always compiled, no dependencies; used by core tests and later by app tests).
- `UnlockedVault::same_key`, `same_slots`, `copy_slots_from` (slot rules S4/S5); `open_sibling` ignores Hello
  slots when telling `Tampered` from `KeyChanged`.
- Tests: the 12 spike tests ported, plus: edit during sync (generation moved) is kept and stays dirty; crash
  between vault and state write → one extra push, no lost edit; local password change vs remote password
  change (S5); Hello slot never crosses devices (S4); join drops the remote Hello slot; rotation elsewhere →
  `KeyChanged` → `merge_after_key_change` keeps local edits and adopts the new key; key-less push succeeds when
  the remote did not move and does nothing when it did; restore beats newer remote edits and tombstones
  entries added after the backup; purge before push.

**Done when:** `cargo test --workspace` green; no network or Tauri dependency in core.

### T19 — GitHub transport, token and sync settings (`feature/t19-github-settings`)

**Add / change** (`src-tauri`)
- `github.rs`: async `GitHubStore: RemoteStore` (S13), `test_connection` (S12), repo/URL parsing,
  status-mapping functions unit-tested without network.
- `secrets.rs`: PAT get/set/delete in Credential Manager (S11); `Zeroizing` everywhere.
- `settings.rs`: `sync` block (S10) with validation; `sync-state.json` read/write (S3) in `paths.rs`.
- Commands: `sync_get_config` (config + `hasToken`), `sync_configure`, `sync_set_token`, `sync_disconnect`
  (deletes token, setup and sync state; the local vault stays), `sync_test_connection` (values from the form,
  typed or stored token), `sync_open_token_page`. Setup commands are refused while the vault is locked.
  New error codes: `secret_store`, `sync_not_configured`, `sync_offline`, `sync_unauthorized`,
  `sync_forbidden`, `sync_rate_limited`, `sync_repo_not_found`, `sync_failed`. A public repository, a missing
  branch and what is at the path are fields of the test report, not errors.
- Frontend: Settings → **Sync** section: repo, branch (optional), path, token field (`type=password`,
  `autocomplete=off`, cleared after save, never shown again; "Replace token", "Turn off sync"),
  "Test connection" with the result; Save is enabled only after a passing test (public repository: after an
  explicit acknowledgement). Step-by-step fine-grained PAT guide (only this repository,
  *Contents: Read and write*, expiry; opens github.com in the default browser through the existing
  `shell::open_url`). i18n en/vi.
- `Cargo.toml`: `reqwest` 0.13 (rustls, http2, system proxy; no default features), `keyring` 3.6, `base64`,
  `tokio` (time only; already in the tree through Tauri).

**Done when:** CI green; manual: test connection against a private test repo (ok / bad token / missing repo /
public repo warning).

### T20 — Sync worker, status and tray (`feature/t20-sync-worker`)

**Add / change**
- `sync_worker.rs`: one tokio task, trigger channel and debounce/timer/backoff (S9), runs `sync_once` with the
  app's `LocalSide` (S2), key-less push on lock and on exit with an 8 s timeout (S8), never two syncs at once.
- `AppState`: edit generation counter, `slots_dirty` set by change password / new kit, notify the worker after
  edits and unlock; `VaultSession::last_seen_ts` refreshed after a merge (D2).
- Status: `sync_status` command + `sync://status` event `{ state: off | idle | syncing | offline | error |
  needsPassword | tokenInvalid | differentVault, dirty, lastSyncAt, errorCode }`; `sync_now` command.
- Header: small sync indicator (icon + tooltip: "Synced 2 min ago" / "Offline, changes saved on this PC" /
  error), click → "Sync now" or opens Sync settings for errors. Settings → Sync shows the same status.
- Tray: "Sync now" item (enabled only when sync is configured and the vault is unlocked); tooltip shows the
  sync state.
- `entries` view reloads after a sync that pulled or merged changes (event `vault://changed`).
- Tests (`MemoryStore`, temp app dir): triggers coalesce; offline → backoff → recovers; lock during a sync
  writes the pushed file and drops nothing; key-less push on lock; status transitions.

**Done when:** CI green; manual two-instance checks of the basic loop pass (see checklist).

### T21 — Join, credential flows and error states (`feature/t21-sync-flows`)

**Change**
- Setup screen: **Create a new vault** (existing flow, then optional "Set up sync" in Settings) or **Use my
  vault from GitHub**: repo + token + test connection → download → master password → local file written,
  `sync-state.json` initialized → optional Windows Hello enroll on this PC. No Emergency Kit screen (the user
  already has it); a reminder that the existing kit still applies.
- Sync on with a local vault and an empty remote path → first push creates the file.
- `DifferentVault`: sync stops; explanation with two choices: use another path, or **replace this PC's vault
  with the one on GitHub** (confirm dialog, current file kept as a backup, needs the remote vault's password).
- `KeyChanged` (S7): *Needs password* banner → password prompt → merge → push; Hello re-enroll offered.
- Token invalid / expired (401) or forbidden (403): banner "GitHub token expired or lacks access" → Sync
  settings, enter a new token; edits stay local meanwhile.
- Password changed on another device (S5): one-time notice after the adopting sync.
- Rotation with sync on (S6): runs online; offline/conflict → `sync_required` error, nothing changed.
- i18n en/vi for all messages.

**Done when:** CI green; manual checklist items for join, rotation elsewhere, token expiry and different vault
pass.

### T22 — Restore a previous version (`feature/t22-restore-backup`)

**Add / change**
- Commands: `list_backups` (timestamp, size, opens-with-current-key flag; entry count when it opens) and
  `restore_backup(name, password?)` using core `restore` (S14); the current file is backed up first.
- Settings → **General** (or Security): "Restore a previous version" list with a confirm dialog explaining
  that the restored state becomes a normal change and will sync to other devices.
- Tests: restore round trip, backup from before a rotation needs the old password, restore while sync is dirty.

**Done when:** CI green; manual restore check passes.

### T23 — Docs and release prep (`docs/t23-v0.4-docs`)

- `docs/spikes/github-sync.md`: close the follow-ups (conflict copy = D1, HLC = D2, purge = D3, retry on
  unreadable GET, async client, 401/403 flow) with links to the code.
- `docs/PLAN.md`: v0.4 status, decisions S4/S6 summarized, IPC table (sync commands), risks updated.
- `README`: "Sync with GitHub" section with the fine-grained PAT guide, what is stored where, metadata visible on
  GitHub (commit times, file size), how to remove the token.
- `CHANGELOG.md` 0.4.0 section; this plan → **done**.

## As built

Differences from the task texts above, found while implementing:

- **Status (T20):** the event payload is `{ phase, dirty, lastSyncAt, error, passwordChanged }`; `error` is the
  usual `{ code, … }` object. An extra phase `noToken` covers a sync setup without a token on this PC (e.g. after
  copying `settings.json`): the header shows a warning and "Sync now" is off.
- **Commit after a sync (T20):** two counters (edits, key slots) are bumped under the session lock; when they moved
  during a sync, the result is applied with core `rebase` (merge plus the S4/S5 slot rules), not a plain `merge`.
- **Flows (T21):** commands `sync_join`, `sync_resolve_key_change`, `sync_replace_local`; error codes
  `sync_remote_missing`, `sync_required`. Everything that talks to GitHub (worker sync, key-less push, join,
  key-change resolution, replace, online rotation) holds one async mutex. While a key change waits for the
  password, setting up Windows Hello is held back (it would wrap the old key).
- **Restore (T22):** `list_backups` reports a `kind` per backup (`current`, `olderKey`, `otherVault`,
  `unreadable`); backups of another vault cannot be restored. The list is its own Settings section.

## Manual test checklist (v0.4.0)

Use a **private** test repository and two data directories (two PCs when available; otherwise two data
directories on one PC: `$env:XPASS_DATA_DIR = "$env:TEMP\xpass-a"` and `...\xpass-b`). Only one dev instance can
run at a time (single-instance plugin, Vite port 5173), so run the devices one after the other and quit each one
from the tray. Both directories hold the same vault and therefore share the Windows Hello key `xpass-<vault_id>`:
turn Hello off in both for the rotation checks. Create the fine-grained PAT yourself and type it only into xPass.

Setup and settings
- [ ] Test connection: correct token → OK; wrong token → "token invalid"; repo not shared with the token →
      "not found"; public repo → warning + confirm.
- [ ] Token never visible again after saving; `settings.json`, `sync-state.json`, the vault file, dev console
      and stderr contain no token (search for the token prefix `github_pat_`).
- [ ] Remove token → sync off, status "not configured"; Credential Manager entry gone.

Basic loop
- [ ] A: create vault, configure sync → first push creates `xpass/vault.xpv` in the repo.
- [ ] B: "Use my vault from GitHub" → password → same entries; Emergency Kit not shown; Hello enroll offered.
- [ ] Edit on A → within ~10 s a commit appears; B gets it on unlock / "Sync now" / within 5 min.
- [ ] No commit when nothing changed (timer ticks, unlock without edits).
- [ ] Header indicator and tray tooltip follow the state; tray "Sync now" works.

Conflicts and offline
- [ ] Both offline (disconnect network): A and B edit the same entry and add different entries → online → both
      converge; the older edit of the shared entry is in its history (D1).
- [ ] Delete on A, edit of another entry on B → no resurrection.
- [ ] Both press "Sync now" at the same time → both converge (one 409/422 retry).
- [ ] Edit while offline, quit the app, start again offline → edit still there → online → pushed.
- [ ] Lock / quit right after an edit → the change is pushed (key-less push) when the remote did not move.

Credentials across devices
- [ ] Change password on A → B syncs silently, shows "password changed on another device"; B's next unlock
      needs the new password; B's Windows Hello still works.
- [ ] Enable Hello on A and on B → both keep working after several syncs (S4).
- [ ] Rotate on A (online) → B shows *Needs password* → B's offline edits survive → Hello on B re-enroll offered.
- [ ] Rotate on A while offline → refused with a clear message, nothing changed.
- [ ] Point B at a path holding another vault → "different vault" explanation; "replace this PC's vault"
      keeps a backup and works.

Errors
- [ ] Revoke / expire the token → "token expired" banner, edits stay local, new token → sync resumes.
- [ ] Token without Contents write → first push reports missing write access.

Restore
- [ ] Restore a backup from before some edits → restored state wins on both devices after sync.

Baseline
- [ ] App fully usable offline (unlock, edit, copy) with sync configured.
- [ ] Release build: CSP still without network sources, capabilities unchanged, devtools off.

## Out of scope

Restoring from GitHub commit history and rewriting history after a compromise (orphan branch + force push),
other backends (folder, OneDrive, WebDAV — the `RemoteStore` trait leaves room), GitHub App / OAuth device flow
instead of a PAT, sync of settings between devices, several vaults per app, device names / per-device Hello
slots in the shared header, compression (D4), sync status history UI.

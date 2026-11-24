# v0.5.0 Plan: Import / Export

Status: **done** (2026-11-24). Parent: [PLAN.md §4 v0.5](PLAN.md#v05--import--export-size-m-2-weeks).

## Goal

Let people move into xPass and out of it without retyping anything:

- **Import** a CSV exported by Bitwarden, Chrome (or Edge / any Chromium browser), KeePassXC, or any other
  manager (generic mapping), with a preview and duplicate detection before anything is written.
- **Export** the vault as CSV (plaintext, behind a warning and a confirm) or as an encrypted **KDBX4** file that
  KeePassXC / KeePass 2 can open.
- **Password history UI**: the per-entry history the vault has kept since v0.1 (edits, conflict copies D1,
  restores S14) becomes visible, revealable, copyable and restorable.

The security baseline stays: no new Tauri plugins, no capability or CSP change, every file is opened, parsed and
written in Rust, and the WebView never receives an imported or exported file's plaintext content.

## What already exists

- `xpass-core`: `Entry` with `urls`, `tags`, `favorite`, `fields: [CustomField { name, value, secret }]`,
  `history: [HistoryItem { at, password, username?, note? }]` (last 20, canonical order, notes `conflict` /
  `restore`), tombstones, per-entry LWW merge that unions history (D1), D2 clock.
- `src-tauri`: `AppState::edit` (one D2 timestamp per edit, atomic write, rolling backups, sync trigger
  debounced 10 s), `entries::save` (old credentials → history), `authorize` / `verify_password` with the unlock
  throttle, Hello `sign` on the vault's challenge, `shell.rs` calling `ShellExecuteW` directly instead of the
  opener plugin, `windows` 0.62 / `windows-sys` 0.61 already in the tree.
- Frontend: `EntryDetail.historyCount` and `fields` are sent but not shown; Settings has General, Security,
  Sync, Restore, About sections; `PasswordPrompt` component; `ConfirmDialog`.
- Not there: any way to pick a file (the Emergency Kit uses `window.print()`), any custom-field UI.

## Decisions

- **I1 — Native file dialogs from Rust, no plugin.** A small `file_dialog.rs` uses the `windows` crate
  (`IFileOpenDialog` / `IFileSaveDialog`, COM STA on a dedicated thread, owner = main window HWND so the dialog
  is modal) and returns a `PathBuf` to Rust only. The frontend never sees or sends a path; it only gets a
  display name (file name, no directory). Rejected: `tauri-plugin-dialog` + `fs` (widens capabilities and puts
  file contents in the WebView), `tauri-plugin-dialog` from Rust only (adds a plugin and JS-invokable commands
  that must stay denied), `rfd` (MIT, fine, kept as fallback if the COM code proves fragile). Same pattern as
  `shell.rs`. Filters: `*.csv`, `*.kdbx`; the save dialog proposes `xpass-export-YYYY-MM-DD.csv|kdbx` in the
  Documents folder, with overwrite confirmation by the dialog.
- **I2 — Parsing and writing in a new pure crate `crates/xpass-interop`.** CSV presets, mapping, duplicate
  detection, CSV writer and KDBX writer, no Tauri / OS code, fully unit-tested. Kept out of `xpass-core` so the
  security-critical core does not pull in `csv`, `keepass` and their trees. Depends on `xpass-core` for the model.
- **I3 — The WebView never gets the plaintext.** Import: Rust reads the file, parses it, holds the result as a
  *pending import* in `AppState` (zeroized on drop) and sends the UI a **preview without secrets**: detected
  preset, headers, row count, and per row `title`, `username`, `urlHost`, `hasPassword`, duplicate status,
  warnings (no passwords, no notes, no custom-field values). Export: Rust builds and writes the file; the UI gets
  `{ fileName, count }`. The pending import is dropped on cancel, on commit, on lock and after 10 min.
- **I4 — Presets** (detected from the header row, user can override):

  | Preset | Header (detection) | Mapping |
  |---|---|---|
  | Bitwarden | `folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp` | `name`→title, `login_uri` (comma-separated)→urls, `folder`→tag, `favorite=1`→favorite, `fields` (`name: value` lines)→custom fields, `login_totp`→secret field `TOTP`; `type=note`→entry with notes only; other types skipped with a warning |
  | Chrome / Edge | `name,url,username,password,note` | `name` (or URL host when empty)→title, `note`→notes |
  | KeePassXC | `Group,Title,Username,Password,URL,Notes,TOTP,Icon,Last Modified,Created` | `Group` (without leading `Root/`)→tag, `TOTP`→secret field `TOTP`, `Created`→`created_at`, `Icon` ignored |
  | xPass | header written by our own CSV export (I8) | lossless round trip of the exported columns |
  | Generic | any header row | user maps title / username / password / URLs / notes / tags / favorite to columns; first guess from common header names (`name`, `login`, `user`, `pass`, `website`, `uri`, `comment`, `folder`, `group`, …); unmapped columns can go to notes or custom fields (off by default) |

  Input rules: UTF-8 with or without BOM (other encodings → `import_encoding`, ask to re-export as UTF-8);
  delimiter `,`, `;` or tab (sniffed from the header line); quoted multi-line notes; max **10 MB** and **10 000
  rows**; rows without title, username, password, URL and notes are skipped and counted. Title falls back to the
  URL host, then to the username.
- **I5 — Duplicate rules.** Match key = (first URL's host without `www.` — or the lower-cased trimmed title when
  there is no URL, lower-cased trimmed username). Against live (non-tombstoned) entries and within the file:
  - **exact duplicate**: same key and same password → skipped (always);
  - **possible duplicate**: same key, different password → per-import choice, default **skip**; alternatives
    **import as new entry** or **update the existing entry** (the old credentials go to its history with note
    `import`, like a normal password change);
  - otherwise **new**. The preview shows counts per class and the user can untick single rows.
- **I6 — Timestamps and sync.** The whole import is **one `AppState::edit`**: one D2 timestamp `ts`, one atomic
  write (the rolling backup before it is the undo path, pointed to from the result screen), one sync push.
  New entries get a fresh UUID, `updated_at = ts`, `created_at` = the source's creation date when the preset has
  one (KeePassXC) and it is not in the future, else `ts`. Updated entries get `updated_at = ts`. Because ids are
  new, other devices just receive new entries; an updated entry merges by LWW with the D1 history copy as for any
  edit. No change to merge or format.
- **I7 — Re-authentication before any export.** `export_vault` takes `auth: { password } | { hello }` and checks
  it inside the command: password through `verify_password` (same throttle as unlock), Hello through a new
  `verify_hello` (sign the vault's Hello challenge, check it opens the Hello slot; only when the slot exists).
  A session unlocked with the recovery key follows the `authorize` rule (no second prompt). No "recently
  authenticated" window: every export asks. The order is confirm → auth → save dialog → write; if the vault is
  locked while the dialog is open, nothing is written.
- **I8 — CSV export format.** Header `title,username,password,urls,notes,tags,favorite,created,updated,fields`
  (`urls` and `tags` newline-separated inside the quoted cell, dates ISO 8601 UTC, `fields` as a JSON array
  of `{ name, value, secret }` so the secret flag and multi-line values survive a round trip; secret fields
  included). History and tombstones are not exported. No formula escaping (prefixing `'`
  would corrupt passwords that start with `=`, `+`, `-`, `@`); the warning says not to open the file in a
  spreadsheet app. The preset `xPass` imports it back.
- **I9 — KDBX4 export with the `keepass` crate** (0.17, MIT, feature `save_kdbx4`; its tree — RustCrypto
  ciphers, `rust-argon2`, `quick-xml`, `flate2`, `chrono`, `secrecy` — is MIT / Apache-2.0; checked again with
  `cargo tree` + licences in the PR, no AGPL/GPL allowed). The user sets an **export password** (new-password form
  with strength meter and confirm; not defaulting to the master password). File: KDBX 4, ChaCha20 or AES-256,
  Argon2id (64 MiB, 25 iterations ≈ 1 s on a desktop CPU, p = 2). Mapping: one group `xPass`; Title, UserName, Password
  (protected), URL = first URL, more URLs as `KP2A_URL_1…` (KeePassXC convention), Notes, Tags (`;`),
  favorite → tag `favorite`, custom fields → string fields (`secret` → protected), history → KDBX entry history
  (one old version per item, `LastModificationTime = at`), creation / modification times kept. Verified by a
  round-trip test that opens the result with the same crate, and by hand in KeePassXC.
- **I10 — Handling plaintext files.** The CSV is written straight to the chosen path (temp file in the same
  folder + rename; never `%TEMP%`, never a second copy), then the result screen repeats: "This file is not
  encrypted. Move it where you need it and delete it." After an import, the result screen offers **Delete the
  source CSV** (Rust removes exactly the file it opened; confirm dialog; normal delete — the text says SSDs cannot
  be wiped reliably and to empty the Recycle Bin if the file came from there). Buffers holding file bytes and
  parsed rows are `Zeroizing`; known limit: the `csv` crate's internal buffers are not zeroized (noted in the
  threat model in v0.6).
- **I11 — Password history UI.** Entry view gets a **Password history (n)** section, newest first: date,
  username when it differs from the current one, label for `conflict` / `restore` / `import`, masked password with
  reveal (auto-hide 15 s) and copy, and **Use this password** (an ordinary save: the current password moves to
  history). New `FieldRef::History { index, at }` — `at` guards against the list changing under the UI (sync
  merge) → `not_found`. Deleting history items is not offered: the merge unions history, so a deleted item would
  come back from another device. The same task shows **custom fields read-only** (imports create them: TOTP,
  Bitwarden fields), with copy / reveal through the existing `FieldRef::Field`.
- **No new Tauri plugin, no capability or CSP change.** New dependencies: `csv` 1.4 (Unlicense/MIT) and
  `keepass` 0.17 (MIT) in `xpass-interop`; extra `windows` features (`Win32_System_Com`, `Win32_UI_Shell_Common`)
  in `src-tauri`.

## IPC additions

| Group | Commands |
|---|---|
| Import | `import_open(preset?)` → preview or `null` (dialog canceled), `import_set_mapping(preset, mapping?)` → preview, `import_commit({ duplicates: skip \| new \| update, excluded: [row] })` → `{ added, updated, skipped }`, `import_cancel`, `import_delete_source` |
| Export | `export_vault({ format: csv \| kdbx, auth, kdbxPassword? })` → `{ fileName, count }` or `null` (dialog canceled) |
| Entries | `list_history(id)` → `[{ index, at, username?, note?, hasPassword }]`; `copy_field` / `reveal_secret` accept `FieldRef::History`; `use_history_password(id, index, at)` |

New error codes: `import_too_large`, `import_invalid_csv`, `import_encoding`, `import_empty`, `import_expired`
(pending import gone: lock or timeout), `dialog_failed`. File errors reuse `io`.

## Tasks

Each task is one feature branch off an up-to-date `develop` and one PR into `develop`. T28 does not depend on the
others and can go first.

### T24 — `xpass-interop`: CSV import, duplicates, CSV writer (`feature/t24-interop-csv`)

**Add** `crates/xpass-interop` (workspace member, MIT, `publish = false`)
- `csv_import`: decode (UTF-8/BOM, delimiter sniff, size/row limits), `Preset` detection, `Mapping`
  (generic + header guessing), `ImportedEntry` (zeroizing) with warnings per row.
- `dedup`: match key, classes exact / possible / new, against a `&Payload` and within the file (I5).
- `apply_import(payload, rows, options, ts)`: adds / updates entries per I5–I6, history note `import`
  (add `HISTORY_NOTE_IMPORT` to `xpass-core::model`).
- `csv_export`: writer for I8.
- Fixtures `tests/fixtures/{bitwarden,chrome,keepassxc,xpass,generic-semicolon,bad-encoding}.csv` with
  **synthetic data only** (`example.com` hosts, passwords like `Sample-Pass-01`).
- Tests: each preset parses its fixture; header detection; quoted multi-line notes; BOM; `;` delimiter; limits;
  empty rows; Bitwarden `fields` / `note` type / unknown type; KeePassXC group and dates; duplicate classes incl.
  within-file duplicates; update keeps old password in history; export → import round trip is lossless for the
  exported columns.

**Done when:** `cargo test --workspace` green; `cargo tree -p xpass-interop` licences listed in the PR.

### T25 — Native file dialogs and export re-auth (`feature/t25-dialogs-reauth`)

**Add / change** (`src-tauri`)
- `file_dialog.rs` (I1): `pick_open(window, filters)` / `pick_save(window, default_name, filters)` on a COM STA
  thread, canceled → `None`; non-Windows stub.
- `AppState::reauthorize(Auth)`: password (`verify_password`) or Hello (`verify_hello`, new; uses `hello_busy`)
  (I7). Unit tests with the fake Hello backend and a temp app dir.
- No UI yet beyond what T26/T27 need.

**Done when:** CI green; manual: open and save dialogs are modal over the window and return to the app.

### T26 — Import flow (`feature/t26-import`)

**Add / change**
- `state/import.rs`: pending import (I3), dropped on lock / commit / cancel / 10 min; commands from the IPC table;
  commit through `AppState::edit` (I6); `import_delete_source` only for the path of the pending/last import.
- Frontend: Settings → new **Import & export** section; import wizard in a modal: pick file → preset (auto,
  changeable) → generic mapping table when needed → preview list (title, username, host, duplicate badge,
  untick rows) with counts → duplicate choice → Import → result (`added / updated / skipped`, "Restore a
  previous version" hint, "Delete the source CSV"). Entry list reloads. i18n en/vi.
- Tests (temp app dir): import commit writes one backup + one edit; lock drops the pending import; possible
  duplicate update stamps history; sync trigger fired once.

**Done when:** CI green; manual import checklist passes with every fixture.

### T27 — CSV and KDBX4 export (`feature/t27-export`)

**Add / change**
- `xpass-interop::kdbx_export` with `keepass` (`save_kdbx4`) per I9; round-trip test (open with the crate,
  compare titles / usernames / passwords / URLs / tags / fields / history / times).
- Command `export_vault` (I7 → I1 → I8/I9 → I10), temp file + rename in the target folder.
- Frontend: in **Import & export**: "Export as CSV" → warning dialog (plaintext, do not open in Excel, delete
  after use) with an "I understand" checkbox → password / Hello prompt → save dialog → result. "Export as KeePass
  (KDBX)" → export password form → auth → save dialog → result. i18n en/vi.

**Done when:** CI green; exported KDBX opens in KeePassXC with the export password; CSV re-imports with the xPass
preset.

### T28 — Password history and custom fields in the entry view (`feature/t28-history-ui`)

**Add / change**
- `entries.rs`: `list_history`, `FieldRef::History { index, at }` for copy / reveal, `use_history_password`
  (I11).
- `EntryView`: collapsible **Password history (n)** (newest first, labels, reveal / copy / use), read-only
  **Custom fields** (copy; reveal for secret ones). i18n en/vi.
- Tests: history ref with stale `at` → `not_found`; use-history moves the current password into history and
  stamps a D2 timestamp; secret custom field is never in `EntryDetail`.

**Done when:** CI green; manual history checks pass.

### T29 — Docs and release prep (`docs/t29-v0.5-docs`)

- `README`: "Import / export" section (how to export from Bitwarden, Chrome, KeePassXC; what is and is not
  imported; plaintext CSV warning; KDBX compatibility).
- `docs/PLAN.md`: v0.5 status and IPC table; risks (plaintext export, CSV parser input).
- `CHANGELOG.md` 0.5.0 section; this plan → **done** with an "As built" section.

## As built

Differences from the decisions and task texts above, found while implementing:

- **Re-auth (T25, I7):** `AppState::reauthorize(Auth)` with `Auth = { method: password, password } | { method:
  hello }`. The Hello check is `UnlockedVault::check_hello` in `xpass-core` (signs `hello_challenge`, opens the
  Hello slot, changes nothing). A re-created Hello credential marks Hello broken, as at unlock. A recovery-key session
  is not asked again in Rust, but the UI still shows the password field (it does not know the session kind).
- **File dialogs (T25, I1):** `file_dialog.rs` runs `IFileOpenDialog` / `IFileSaveDialog` on its own COM STA
  thread, modal to the main window; files are not added to Recent. Cancel → `null`; failure → `dialog_failed`. Two
  `#[ignore]` tests open and close the real dialogs (`cargo test -p xpass --lib file_dialog -- --ignored`).
- **Preview (T26, I3):** `headers` is sent only for the generic preset (a file without a header row would otherwise
  put its first data row in the WebView). The 10-minute limit is also checked by the idle watcher every 10 s, so
  parsed rows do not stay in memory. `import_delete_source` works on the pending or last committed import; that
  path is forgotten on lock. Possible-duplicate badges in the preview are computed without the unticked rows'
  effect; the commit recomputes them, so the result counts are right.
- **Generic mapping (T26, I4):** `site` is the last URL guess (`Name` + `Site` → title + website; `Site` + `URL` →
  title + website). Mapping errors return `invalid_input` (`field: "mapping"`).
- **Import wizard (T26):** the wizard cancels the pending import from its close actions, not from an effect cleanup
  (React StrictMode ran the cleanup right after mounting in dev and cancelled every import; fixed in T27).
- **KDBX (T27, I9):** fixed Argon2id cost 64 MiB / 25 passes / p = 2 (10 passes measured 0.4 s in a release build)
  instead of tuning at runtime; ChaCha20 outer and inner cipher. A custom field named like a standard field gets
  ` (2)`. Unit tests use a 1 MiB / 1 pass KDF. New dependencies: `keepass` 0.17, `chrono`, `rust-argon2` (MIT /
  Apache-2.0).
- **Export (T27, I7–I10):** `export_vault(format, auth, kdbxPassword)` takes separate arguments. The export password
  must pass the same strength check as the master password (`weak_password` otherwise), and it is checked before
  re-auth. The file is built from a payload snapshot after the save dialog; the lock check runs again just before
  the write.
- **History (T28, I11):** `list_history` sends the username only when it differs from the current one. The entry
  view reveals one secret at a time (password, secret field or old password), keyed by entry version, so a save or
  sync hides it. "Use this password" asks for confirmation.
- **Dialogs (T28):** `Modal` keeps the title fixed and scrolls its body (the import wizard was squeezed and could not
  scroll).

## Manual test checklist (v0.5.0)

Use a **test vault** (`$env:XPASS_DATA_DIR = "$env:TEMP\xpass-v05"`) and only synthetic data: the fixtures from
T24, plus files exported from Bitwarden / Chrome / KeePassXC test profiles holding a few **fake** entries created
for this purpose. Never import or export a real vault while testing; delete exported files afterwards.

Import
- [ ] Bitwarden fixture and a real Bitwarden export of fake entries: preset detected; folder → tag, favorite,
      URIs, custom fields, TOTP as secret field, secure note imported; card/identity rows (if any) skipped with a
      warning.
- [ ] Chrome (and Edge) export: preset detected; empty name → host as title.
- [ ] KeePassXC export: group → tag, created date kept, TOTP as secret field.
- [ ] Generic `;`-separated file: mapping table, header guesses, manual change, preview follows.
- [ ] Non-UTF-8 file → clear "re-export as UTF-8" message; > 10 MB file → refused; not a CSV → refused.
- [ ] Import the same file twice → second run shows all rows as exact duplicates, nothing added.
- [ ] Possible duplicate (same site and user, new password): skip / new / update each behave; update puts the
      old password in history with label "import".
- [ ] Untick rows → those are not imported; result counts match.
- [ ] Preview never shows passwords or notes; dev console / IPC log contains no password from the file.
- [ ] Lock while the wizard is open → pending import gone, unlock and retry works.
- [ ] Undo: "Restore a previous version" brings back the vault from before the import.
- [ ] "Delete the source CSV" removes the file (confirm shown).
- [ ] With sync on: import → one commit on GitHub; second device receives all entries.

Export
- [ ] CSV export: warning + checkbox, wrong password → refused (throttled like unlock), Hello works when
      enabled, cancel in the save dialog → nothing written.
- [ ] CSV file content: header as in I8, multi-line notes intact, tombstoned entries absent; re-import with the
      xPass preset into an empty test vault → same entries.
- [ ] KDBX export: opens in KeePassXC with the export password; titles, users, passwords, URLs (extra ones as
      `KP2A_URL_n`), notes, tags, custom fields (protected when secret), history and dates present.
- [ ] Lock during the save dialog → nothing written.
- [ ] No plaintext copy in `%TEMP%` or `%APPDATA%\xpass` after a CSV export.

Password history
- [ ] Change a password twice → history shows both old values, newest first, reveal auto-hides after 15 s, copy
      works and clears from the clipboard.
- [ ] "Use this password" → becomes current, previous current moves into history.
- [ ] Conflict copy (two devices, see v0.4 checklist) and restored entries show their labels.
- [ ] Imported custom fields visible; secret ones masked until revealed.

Baseline
- [ ] Release build: capabilities file and CSP unchanged, no new Tauri plugin, devtools off.

## Out of scope

Importing KDBX / 1Password (`.1pux`) / LastPass / Bitwarden JSON or encrypted exports (CSV only in v0.5),
non-UTF-8 encodings, attachments, passkeys, Bitwarden cards / identities, KDBX key files or YubiKey, scheduled or
automatic exports, encrypted CSV, editing custom fields (later: custom fields UI), TOTP codes (later), deleting
history items, secure wipe of files, drag-and-drop of files onto the window (would hand the path through the
WebView).

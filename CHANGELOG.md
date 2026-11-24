# Changelog

All notable changes to xPass. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/).

## [0.5.0]

Import / export and password history.

### Added

- Settings → **Import & export** → **Import from CSV**: Bitwarden, Chrome / Edge, KeePassXC and xPass files are
  detected from the header row; any other CSV is imported by choosing which column holds each field. A preview
  shows every row (without passwords) with its duplicate status before anything is saved. Rows already in the vault
  are skipped; for rows with the same site and username but another password, choose skip, import as new, or
  update the existing entry (the old password goes to its history). The whole import is one change (one backup,
  one sync), and the result screen offers to delete the source CSV.
- **Export as KeePass (KDBX)**: an encrypted KDBX 4 file with its own export password, readable by KeePassXC and
  KeePass 2, including extra websites, tags, custom fields and password history.
- **Export as CSV**: a plaintext file behind a warning and an "I understand" checkbox; it imports back into xPass.
- Every export asks for the master password or Windows Hello, then shows the Windows save dialog.
- Entry view: **Password history (n)** with the date, the username of that time when it differed, labels for sync
  conflicts, restores and imports, reveal (hides after 15 s), copy (cleared from the clipboard like the password)
  and **Use this password**.
- Entry view: custom fields, read-only, with copy and reveal for secret ones (for example a TOTP secret from an
  import).

### Changed

- Dialogs taller than the window scroll their content instead of squeezing it.

### Security

- Files are chosen in the native Windows dialog and read, parsed and written in Rust; the file path and the file
  contents never reach the WebView. No new Tauri plugin, capability or CSP change.
- Exports are written next to the target (temporary file and rename), never to `%TEMP%`; nothing is written if the
  vault locks before the write.

## [0.4.0]

GitHub sync: one vault on several PCs. This completes the MVP.

### Added

- Sync through a file in a private GitHub repository, set up in Settings → Sync: repository, branch and path, a
  fine-grained token (stored in Windows Credential Manager, never shown again) and **Test connection**, which also
  warns about a public repository. A guide explains how to create the token.
- **Use my vault from GitHub** on the first screen: downloads the vault, opens it with the master password and turns
  sync on; the existing Emergency Kit keeps working and Windows Hello can be set up for the new PC.
- Offline-first sync: changes are saved locally and synced 10 s after the last edit, every 5 minutes, on unlock,
  on lock and on quit; retries with backoff when offline. Concurrent edits merge per entry, keeping the older
  version of an entry edited on two PCs in its history.
- Sync status in the header (click to sync now), in Settings and in the tray tooltip; **Sync now** in the tray menu.
- Banners with the fix for problems that need you: expired or under-privileged token, vault key changed on another
  PC (enter the master password; local changes are merged), a different vault at the configured path (use another
  path or replace this PC's vault, keeping a backup). A one-time notice when the master password was changed on
  another PC.
- Settings → **Restore a previous version**: restores one of the last 10 local versions as a new change, so it also
  wins on the other PCs. Versions from before a key change ask for the password of that time.

### Changed

- Rotating the vault key with sync on needs a connection: the new file is uploaded first, and nothing changes if
  that fails. The other PCs then ask for the master password once.
- Windows Hello stays set up per PC; a synced file never turns it off on another PC.

## [0.3.0]

Unlock with Windows Hello.

### Added

- Windows Hello unlock (face, fingerprint or PIN). Turn it on in Settings → Security with the master password; the
  unlock screen then offers Windows Hello first and opens its prompt once per lock, as soon as the window is shown.
  The master password form stays available, and the password is still needed to turn Hello on and for sensitive
  changes.
- Each vault has its own Hello key in the PC's TPM; it only wraps the vault key, which never leaves the Rust side.
- If Windows Hello is reset or changed on the PC, xPass explains it, unlocks with the password and offers to set
  Hello up again. Rotating the vault key turns Hello off and offers the same.

### Changed

- GitHub Actions workflows run on Node 24.

## [0.2.0]

Custom title bar and UI polish.

### Added

- Self-drawn title bar that follows the light / dark / system theme: drag to move, double-click to maximize,
  minimize / maximize-restore / close buttons; close still hides to the tray and can lock the vault. App actions
  (generator, settings, lock, theme, language) are icon buttons with tooltips. The snap layouts flyout on the
  maximize button is not available (`Win+Z` and drag-to-edge snapping still work).

### Changed

- The app name is shown as **xPass** (window, tray, texts). The installer, executable and data folder keep the
  `xpass` name, so 0.2.0 upgrades 0.1.0 in place.
- The status bar is separated from the content by a line.

### Fixed

- The new master password form no longer jumps up and down while typing.

## [0.1.0]

First release: a local-only vault on Windows.

### Added

- Vault file format v1: XChaCha20-Poly1305, VaultKey wrapped by Argon2id (master password) and HKDF (recovery key)
  key slots, authenticated header, golden test vector.
- Setup with a strength-checked master password and a printable Emergency Kit (recovery key) confirmed by re-typing
  two groups; unlock with the password or the recovery key, followed by a forced new password.
- Entries: list, search, favorites, create/edit/delete (tombstones), password history on change, open websites in
  the default browser, keyboard shortcuts.
- Secure copy excluded from clipboard history and cloud clipboard, cleared after a timeout, on lock and on exit;
  reveal that hides again after 15 s.
- Password generator: characters or EFF passphrases with an entropy estimate.
- Auto-lock on Windows lock, display off / sleep (including Modern Standby), inactivity and, optionally, minimize;
  the unlock screen shows why the vault locked.
- Settings: theme, language, auto-lock and clipboard timeouts, close to tray, change master password, new Emergency
  Kit, rotate the vault key.
- Tray icon (Open / Lock / Quit), single instance, atomic saves with the last 10 backups, wrong-password throttling.
- Per-entry merge, HLC-lite clock and tombstone purge in `xpass-core`, ready for sync in v0.3.

[0.5.0]: https://github.com/bs135/xpass/releases/tag/v0.5.0
[0.4.0]: https://github.com/bs135/xpass/releases/tag/v0.4.0
[0.3.0]: https://github.com/bs135/xpass/releases/tag/v0.3.0
[0.2.0]: https://github.com/bs135/xpass/releases/tag/v0.2.0
[0.1.0]: https://github.com/bs135/xpass/releases/tag/v0.1.0

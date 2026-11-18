# Changelog

All notable changes to xPass. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/).

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

[0.3.0]: https://github.com/bs135/xpass/releases/tag/v0.3.0
[0.2.0]: https://github.com/bs135/xpass/releases/tag/v0.2.0
[0.1.0]: https://github.com/bs135/xpass/releases/tag/v0.1.0

# Changelog

All notable changes to xPass. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/).

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

[0.1.0]: https://github.com/bs135/xpass/releases/tag/v0.1.0

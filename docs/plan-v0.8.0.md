# v0.8.0 Plan: TOTP, custom fields, password health

Status: **approved** (2026-11-25). Parent: [PLAN.md §4 v0.8 → v1.0](PLAN.md#v08--v10--release-outline).

## Goal

The last feature milestone before 1.0 (user's decision, 2026-11-25):

- **TOTP codes**: show and copy the current one-time code of an entry. The code is computed in Rust; the secret
  never reaches the WebView.
- **Custom fields**: add, edit, reorder and delete them in the entry form (today they are read-only, v0.5).
- **Password health**: list entries with a weak or reused password.
- **1.0 documents**: `docs/FORMAT.md` (frozen vault format v1 / payload schema 1) and `SECURITY.md`.

Not in v0.8 or 1.0: **auto-type** (user's decision). After 1.0 the user runs xPass day to day for a while, fixes the
bugs that turn up and collects improvements. Those notes shape whatever comes after 1.0 (see U12).

The baseline stays: **no format or schema change**, no new capability, CSP unchanged, no new network access.

## What already exists

- `xpass-core::model::CustomField { name, value, secret }` in `Entry.fields`, stored since schema 1. `CustomField`
  has **no** `extra` map: a key it does not know is dropped when an older app saves the entry again. Every
  decision below therefore keeps the stored shape exactly as it is.
- Imports (v0.5) put one-time password secrets in a **secret** custom field named `TOTP` (`xpass-interop`
  `convert::TOTP_FIELD`): `otpauth://totp/…` URIs from Bitwarden, raw base32 secrets from KeePassXC. The KDBX
  export writes them as a protected string field `TOTP`; the CSV export writes `fields` as JSON.
- `src-tauri/src/entries.rs`: `EntryDetail.fields: Vec<FieldView { name, value: Option<String>, secret }>` (no
  value for secret fields); `FieldRef::Field { index }` for `copy_field` / `reveal_secret`; `EntryInput` has **no**
  fields, and `save` keeps them as they are.
- `EntryView.tsx` lists custom fields read-only with reveal / copy; `EntryForm.tsx` has no field editor.
- `strength.rs`: zxcvbn 3.1 in Rust (`password_strength`), inputs capped at 100 characters, `MIN_SCORE` 3 for the
  master password.
- Merge (D1) is per entry, last writer wins on the whole entry, history excluded: custom field edits sync like
  any other edit. Password history holds only password and username (`HistoryItem`).
- `hmac` 0.12 is already in `Cargo.lock` (through `hkdf`), with `sha1` and `sha2` as direct dependencies of
  `xpass-core`.
- The format is described only in `docs/spikes/vault-format.md`, which is out of date (its payload example has
  `url` instead of `urls` and no `fields`, `history` or `extra`). The golden vector `tests/vault.rs`
  `golden_vector_v1` freezes the bytes. `THREAT_MODEL.md §7` says how to report a vulnerability; there is no
  `SECURITY.md`.

## Decisions

- **V1 — Scope.** v0.8 = TOTP (T45, T47), custom field editing (T46), password health (T48), 1.0 documents (T49),
  release docs (T50). **v1.0** = whatever the user decides after v0.8 (U12); auto-type is out of 1.0.

- **V2 — TOTP lives in a custom field, no schema change.** A field is a TOTP field when it is **secret** and its
  value is an `otpauth://totp/…` URI, or when its name is `TOTP` / `OTP` (any case) and its value is a base32
  secret. A new TOTP is stored the way imports already store it: a secret field named `TOTP` whose value is a
  normalized `otpauth://totp/<label>?secret=…&issuer=…` URI (plus `algorithm`, `digits` and `period` only when they
  are not the defaults). Older versions see an ordinary secret field. KDBX and CSV exports keep it as they do
  today; the KDBX export **also** writes KeePassXC's `otp` attribute (the same URI), so KeePassXC shows the code.

- **V3 — TOTP math in `xpass-core` (`otp.rs`), no new crate.** RFC 4226 / 6238 with `hmac` 0.12 (already in the
  lockfile; added as a direct dependency) over SHA-1, SHA-256 or SHA-512; 6–8 digits; period 15–120 s (default 30).
  The parser accepts `otpauth://totp/` URIs and bare base32 (spaces, dashes and lower case allowed, padding
  optional); secrets of 10–128 bytes after decoding. It refuses `hotp` (it needs a counter that changes on
  every use: a write per code, which would clash with sync) and Steam's alphabet. Test vectors are those of
  RFC 6238 Appendix B. The parser gets a fuzz target (`otp_parse`, added to the weekly schedule) because its
  input comes from imports and paste.

- **V4 — The secret stays in Rust; codes are shown.** New command `totp_code(id, index)` →
  `{ code, period, remainingMs }`. The secret is decoded, used and dropped (`Zeroizing`) on each call. The entry view
  shows the code (grouped `123 456`) with a countdown and asks again when it expires, only while the entry is open
  and the vault unlocked. `copy_field` gains `FieldRef::Totp { index }`: it copies the **current code**, not the
  secret, with the same clipboard rules (no history, clear after the timeout). Revealing the secret stays as it is
  for any secret field (`reveal_secret`) so it can be moved to another app. Codes are not hidden behind a reveal
  button: a code is useful for a few seconds and the entry is already open.

- **V5 — Adding a TOTP.** In the entry form, **Add one-time password** opens a small dialog: paste an
  `otpauth://` URI or a base32 secret, optional issuer / account (pre-filled from the URI or the entry), then a
  preview of the current code before saving, so a mistyped secret shows up at once. No QR scanning (no camera or
  screen capture in the app; out of scope). The dialog sends the secret once, like a password; Rust moves it into
  `Zeroizing` on entry.

- **V6 — Custom field editing: secret values never need a reveal.** `EntryInput` gains
  `fields: Option<Vec<FieldInput>>`. `None` keeps the stored fields, so older UI calls and other commands keep
  working. `Some` replaces the list in the given order. Each `FieldInput` has `name`, `secret` and a value:
  - `{ set: "…" }`: a new value;
  - `{ keep: { index, name } }`: keep the stored value of the field that was at `index` when the form was loaded. The
    `name` must still match, otherwise `not_found` (the entry changed under the form, e.g. by a sync merge).

  So the form can rename, reorder, flip `secret` or delete a secret field without its value ever reaching the
  WebView. The form shows a secret value only after an explicit reveal (as today), and turns it into `set` only if
  the user changes it. Limits: at most 50 fields per entry, a name of at most 100 characters, a value of at most
  10 000 characters; an empty name is refused. Duplicate names are allowed (imports have them).

- **V7 — No history for custom fields.** `HistoryItem` cannot hold them without a schema change, and an older
  version would drop the new key (see "What already exists"). Changing or deleting a **secret** field therefore
  asks for confirmation ("the old value is not kept in the history; local backups keep the last 10 versions").
  Accepted limit, listed in THREAT_MODEL §5 and the README.

- **V8 — Password health in Rust, on demand.** New command `password_health()` →
  `{ weak: [ids], reused: [[ids]], checkedAt }`. Each list holds entry ids only, never passwords:
  - weak: zxcvbn score < 3 (the master-password threshold). The entry's title, username and URL hosts are passed
    as user inputs, so a password made from them scores low;
  - reused: the same password (exact match) in two or more live entries. The comparison stays in Rust memory and
    the copies are wiped (`Zeroizing`) after it; empty passwords are ignored.

  It runs on a blocking thread and is computed only when the health view opens, never stored. With 2 000 entries,
  zxcvbn takes roughly 1–2 s. The view shows "checking…" and the result is cached per vault revision until the
  next change. Breach checks against an online service (HIBP) stay out: they would send password hashes
  (k-anonymity prefixes) out of the app, which v0.8 does not do.

- **V9 — Health UI.** The entry list gets a **Password health** filter next to Favorites. It shows the counts and
  two groups (Weak, Reused together with the entries sharing each password). A click opens the entry, where the
  password field shows the same badge. No score is shown in the list itself: zxcvbn is too slow to run on every
  render.

- **V10 — `docs/FORMAT.md`: format v1 written from the code.** It covers:
  - the header byte by byte, slot kinds and parameters, KDF bounds (`KdfParams::validate`), HKDF info strings,
    the AEAD and AAD bindings;
  - the recovery key encoding and checksum, and the Hello challenge;
  - payload schema 1 with every key and its default (`fields`, `history`, `extra` included), tombstones, the
    merge rules (D1–D3) and the blob-SHA sync contract;
  - compatibility rules: unknown payload and entry keys survive (`extra`), `CustomField` and `HistoryItem` do
    not, so they must not grow keys within schema 1; what needs `version` 2 or `schema` 2;
  - the golden vector as the reference file.

  The spike document gets a note pointing to `FORMAT.md` as the source of truth. "Frozen" means a change after
  1.0 needs a new version number, a migration and a new golden vector.

- **V11 — `SECURITY.md`.** It covers:
  - supported versions: the latest release only, as before 1.0, since the in-app update makes moving forward
    easy; it may grow to a supported range at 1.0;
  - how to report: GitHub private vulnerability reporting, which the **user** turns on (Settings → Security →
    Private vulnerability reporting); I do not change repository settings;
  - what is in scope (the THREAT_MODEL boundaries) and what is not (A5 malware as the user, self-signed
    SmartScreen warnings);
  - best-effort response times for a one-person project;
  - how fixes are released.

  `THREAT_MODEL.md §7` and the README point to it.

- **V12 — After v0.8 (not planned here).** The user decides on 1.0. Options: release 1.0 directly, or first run
  the third-party review of v0.7 U14 (scope: xpass-core crypto, format and sync, the IPC boundary, the update
  chain). After 1.0, a period of real use: bugs found are fixed in patch releases (`fix:` → `x.y.z+1`, in-app
  update), and improvements go into a notes list for the next plan.

- **V13 — Threat model.** It gets these changes:
  - new limit: a TOTP secret in the same vault as the password turns two factors into one for anyone who opens
    the vault. This is the user's choice per entry, and the README says so;
  - TOTP codes reach the WebView (short-lived); TOTP secrets do not, except on an explicit reveal;
  - custom field edits are not in the history (V7);
  - health results carry ids only;
  - §4 rows for `otp.rs` (RFC vectors, fuzz) and for health (no passwords over IPC).

## IPC additions

| Group | Commands |
|---|---|
| Entries | `save_entry` input gains `fields?: FieldInput[]` (V6); `get_entry` adds `totp: boolean` to each `FieldView` |
| Secrets | `totp_code(id, index)` → `{ code, period, remainingMs }`; `copy_field` with `{ kind: "totp", index }` copies the current code |
| Tools | `totp_preview(uri)` → `{ code, period, remainingMs, issuer, account }` for the add dialog (V5); `password_health()` → `{ weak, reused, checkedAt }` |

New error codes: `totp_invalid` (unparsable or unsupported secret, HOTP), `field_limit` (V6 limits). No new events.

## Tasks

Each task is one feature branch off an up-to-date `develop` and one PR into `develop`. T45 comes before T47. T46
comes before T47 (the add dialog lives in the field editor). T48 and T49 are independent. T50 is last.

### T45 — TOTP core (`feature/t45-totp-core`)

- `xpass-core/src/otp.rs`: parse (URI, base32), normalize to a URI, `code_at(secret, unix_s)` → code and
  remaining time; `is_totp_field(&CustomField)` (V2). Direct dependency `hmac = "0.12"`.
- Tests: RFC 6238 Appendix B vectors (SHA-1 / 256 / 512, 8 digits), RFC 4226 vectors, URI round trips, bad input
  (HOTP, bad base32, short secret, digits / period out of range), imported fixtures (Bitwarden URI, KeePassXC
  base32).
- Fuzz target `otp_parse` (seeds from the fixtures) and its stable replay test; `fuzz.yml` schedule includes it.
- `cargo deny` stays green.

**Done when:** CI green; vectors pass; the new fuzz target runs in a manual Fuzz run.

### T46 — Custom field editing (`feature/t46-custom-fields`)

- `EntryInput.fields: Option<Vec<FieldInput>>` with `set` / `keep { index, name }` (V6); limits, `field_limit`.
  `Debug` stays redacted.
- Entry form: a field list (name, value, secret toggle, reveal for stored secret values, move up / down,
  delete), confirmation when a secret value is changed or deleted (V7). i18n en/vi.
- Tests: keep / set / reorder / rename / flip secret / delete; a stale `keep` → `not_found`; limits; `None` keeps
  the fields; sync merge of two field edits (LWW per entry); `Debug` without values.

**Done when:** CI green; manual: add, edit, reorder and delete fields on an imported entry, with no secret value in
the WebView unless revealed (devtools check in a debug build).

### T47 — TOTP in the app (`feature/t47-totp-app`)

- `totp_code`, `totp_preview`, `FieldRef::Totp`; `FieldView.totp`; `totp_invalid`.
- Entry view: code with countdown and copy; Add one-time password dialog in the field editor (V5); KDBX export
  also writes `otp` (V2). i18n en/vi.
- Tests: code for a known secret at a fixed time (injected clock); copy puts the code, not the secret, on the
  clipboard; locked → `locked`; a non-TOTP field → `totp_invalid`; KDBX export has `otp`.

**Done when:** CI green; manual: codes match an authenticator app for a test account (Bitwarden-imported URI and a
pasted KeePassXC secret); the copied code is cleared after the timeout; a KDBX export opened in KeePassXC shows the
code.

### T48 — Password health (`feature/t48-password-health`)

- `src-tauri/src/health.rs` + `password_health` (V8), cached per vault revision; list filter and groups (V9);
  badge in the entry view. i18n en/vi.
- Tests: weak with user inputs, reused groups, tombstones and empty passwords ignored, no password in the
  serialized result, cache cleared by an edit or a sync.
- Measure the time with 2 000 synthetic entries in a release build; if it is over 2 s, compute in chunks and
  report progress.

**Done when:** CI green; manual on a test vault with weak and reused passwords: the counts are right, a fix
removes the entry from the list after saving.

### T49 — 1.0 documents (`docs/t49-format-security`)

- `docs/FORMAT.md` (V10), checked against `format.rs`, `vault.rs`, `recovery.rs`, `model.rs`, `merge.rs` and
  the golden vector; a note in `docs/spikes/vault-format.md`.
- `SECURITY.md` (V11); links from README and `THREAT_MODEL.md §7`.
- Before: the user turns on private vulnerability reporting (V11), or the document says to use the address the
  user gives.

**Done when:** a review pass finds no difference between `FORMAT.md` and the code (each section names the source
file); links work on GitHub.

### T50 — Docs and release prep (`docs/t50-v0.8-docs`)

- `THREAT_MODEL.md` (V13); README (Features: TOTP, custom fields, health; the "two factors in one vault" note);
  `PLAN.md` (v0.8 status, §3.5 IPC, §1 "Later" without TOTP / health / custom fields, auto-type marked as
  after 1.0, outline for 1.0 from V12).
- `CHANGELOG.md` `[Unreleased]` → 0.8.0; this plan → **done** with "As built"; `npm run release:prepare` →
  `chore(release): 0.8.0`.

## Manual test checklist (v0.8.0)

Use a test vault (`$env:XPASS_DATA_DIR = "$env:TEMP\xpass-v08"`) with synthetic entries.

TOTP
- [ ] Codes equal those of an authenticator app for the same secret (SHA-1 / 6 digits / 30 s, and one SHA-256 or
      8-digit secret); countdown and refresh at the period boundary.
- [ ] Imported Bitwarden URI and KeePassXC base32 secret both show codes; HOTP and a bad secret give a clear error.
- [ ] Copy puts the code on the clipboard (not in `Win+V` history), cleared after the timeout; lock stops the codes.
- [ ] KDBX export opened in KeePassXC shows the code.

Custom fields
- [ ] Add, rename, reorder, delete; flip secret; edit a secret value only after reveal; confirmation on secret
      change / delete.
- [ ] No secret field value in the WebView without a reveal (debug devtools: `get_entry` response).
- [ ] Two PCs edit fields of the same entry: newer edit wins, nothing else lost.
- [ ] An entry edited in v0.8 still opens and saves in v0.7.0 without losing its fields.

Password health
- [ ] Weak and reused entries listed correctly; fixing one updates the list; no password in the `password_health`
      response.
- [ ] Time with a large vault (2 000 entries) acceptable, UI responsive.

Release
- [ ] v0.7.0 NSIS and MSI installs update in-app to v0.8.0 (first real update from a release; v0.7 checklist).
- [ ] `verify-updater` and `publish` jobs pass without manual steps (#58).

## Out of scope

Auto-type (not in 1.0), HOTP and Steam codes, QR code scanning, breach checks against online services, history of
custom fields, attachments, card / identity entry types, a format or schema change, the third-party review (V12).

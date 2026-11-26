# v1.0.0 Plan: First stable release

Status: **done** (2026-11-26). Parent: [PLAN.md §4 v1.0](PLAN.md#v10--first-stable-release).

## Goal

Release the v0.8.0 feature set as **1.0.0**, the first stable version, and start the period of real use.
No code change: 1.0.0 is v0.8.0 with the version number, the documents and the support promise that come with "stable".

## Decisions

- **W1 — No third-party review before 1.0 (user's decision, 2026-11-26).** The review of v0.7 plan U14 stays
  possible later; what it would need is already in place: the frozen format spec ([FORMAT.md](FORMAT.md)), the
  [threat model](THREAT_MODEL.md), [SECURITY.md](../SECURITY.md) with private vulnerability reporting, SBOMs and
  build provenance per release. Until a review happens, xPass rests on its own tests, fuzzing and self-review
  (THREAT_MODEL §6). README and SECURITY.md say so plainly.
- **W2 — Scope of 1.0.** Exactly the features of v0.8.0: local encrypted vault, Windows Hello, Emergency Kit, GitHub
  sync, import / export, password history, TOTP codes, custom fields, password health, signed in-app update, signed
  builds. **Not in 1.0:** auto-type (user's decision), a publicly trusted code-signing certificate, Touch ID /
  other platforms.
- **W3 — Versioning from 1.0.** Semantic Versioning in earnest: `fix` / `perf` → patch, `feat` → minor, a breaking
  change → major (`release:prepare` already does this). For the vault, "breaking" follows
  [FORMAT.md §9](FORMAT.md#9-compatibility-rules): anything an existing build would misread needs a new format
  `version` or payload `schema`, which makes it a major release.
- **W4 — Support.** The latest release only, as before; the in-app update makes moving forward a click.
  SECURITY.md states it for 1.x.
- **W5 — After 1.0.** The user runs xPass day to day. Bugs found are fixed in patch releases (`1.0.x`) through the
  normal flow (feature branch → `develop` → release PR → in-app update); improvements are collected as GitHub issues
  (security problems through private vulnerability reporting, never in a public issue) and shape the next plan.

## Tasks

One branch `docs/v1.0-release`, one PR into `develop`, then the release PR `develop` → `main`.

- Tick the manual checklist items of v0.7 and v0.8 that the v0.8.0 release proved (in-app update from a published
  release, unattended release run).
- README, PLAN, THREAT_MODEL, SECURITY.md: status 1.0, scope, versioning, support, no review yet.
- CHANGELOG `[Unreleased]` → 1.0.0; `node scripts/release-prepare.mjs 1.0.0` (explicit: the commits since v0.8.0 are
  docs only) → `chore(release): 1.0.0`.

## Manual test checklist (v1.0.0)

- [ ] Release run passes unattended; v1.0.0 is Latest with all assets, checksums and attestations.
- [ ] v0.8.0 updates in-app to v1.0.0; About shows 1.0.0; vault and settings intact.

## Out of scope

Any code change, the third-party review (W1), auto-type, new platforms.

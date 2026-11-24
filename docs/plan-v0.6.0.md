# v0.6.0 Plan: Hardening

Status: **approved** (2026-11-24). Parent: [PLAN.md §4 v0.6 → v1.0](PLAN.md#v06--v10--hardening--release-size-m-2-weeks).

## Goal

Make the security claims of xPass written down, checked and harder to break, without new features beyond an
update check:

- A **threat model** and a **security self-review** against [IDEA.md §3](IDEA.md#3-design-issues), with every
  finding either fixed in this milestone or recorded as an accepted limit.
- **Supply-chain checks** in CI: Rust advisories, licences (no AGPL / GPL), crate sources, npm advisories.
- **Fuzzing** of the vault file parser (the only code that reads attacker-controlled bytes before any MAC check).
- **Memory hygiene**: key material kept out of the pagefile, fewer plaintext copies at the IPC / serde boundaries.
- **Update check** (D10): Rust asks GitHub Releases, the WebView never talks to the network, nothing is downloaded.
- **Release process** that works with branch protection on `main`; installer notes for unsigned builds.
- Clean-up: remove `spikes/`.

The baseline stays: no new Tauri plugin, no capability change, CSP unchanged (`connect-src` stays IPC only).

## What already exists

- `xpass-core::format::parse`: bounds-checked `Reader`, slot count 1..=8, no duplicate slot kinds, fixed wrapped-key
  length; `KdfParams::validate` caps Argon2 at 2 GiB / t ≤ 20 / p ≤ 16 when reading a file. The header is the AAD of
  the payload, each slot's params are the AAD of its wrapped key. Tests: every single-byte flip is detected, KDF
  downgrade rejected, `parser_never_panics_on_garbage` (random inputs, stable toolchain).
- `zeroize`: `Key = Zeroizing<[u8; 32]>`; `Payload` / `Entry` / `CustomField` / `HistoryItem` zeroize on drop
  (`extra` and ids/timestamps skipped); decrypted plaintext and the JSON buffer in `save` are `Zeroizing<Vec<u8>>`;
  PAT and recovery key are `Zeroizing<String>`; pending import rows zeroized.
- Gaps found while reading the code (input for T30 / T32):
  - **Secrets arrive over IPC as plain `String`**: `unlock_password`, `create_vault`, `change_password`,
    `rotate_vault_key`, `new_recovery_key`, `restore_backup`, `hello_enable`, `sync_join`,
    `sync_resolve_key_change`, `sync_replace_local`, `export_vault` (`kdbx_password`), `Auth::Password`,
    `EntryInput.password`, and `password_strength` (called on every keystroke of the master-password form).
  - `UnlockedVault::save` serializes into a growing `Vec` (`serde_json::to_vec`): each reallocation frees a partial
    plaintext copy without wiping it.
  - The VaultKey lives inline in `UnlockedVault` (moved and cloned with it, e.g. `persist_with` clones the vault on
    every edit), so it cannot be page-locked where it is; copies are wiped on drop but may be paged out before that.
  - `serde_json` deserialization uses an internal scratch buffer for escaped strings that is not wiped; the `csv`
    crate's buffers are not wiped (noted in v0.5).
- CI (`ci.yml`, `windows-latest`): oxlint, vitest, `tsc` + vite build, `cargo fmt`, clippy `-D warnings`,
  `cargo test --workspace`. No dependency audit. `release.yml`: `mathieudutour/github-tag-action` computes the
  version from Conventional Commits, **pushes a `chore(release): x.y.z [skip ci]` commit to `main`**, tags, and
  `tauri-action` builds NSIS + MSI and publishes the release.
- `tauri.conf.json`: bundle targets `nsis`, `msi`; no NSIS options set (default per-machine prompt / WebView2
  bootstrapper). README "Install" already mentions SmartScreen in one sentence.
- `spikes/` (three standalone crates) is excluded from the workspace and from oxlint; still referenced by
  `Cargo.toml` (`exclude`), `.oxlintrc.json`, README (layout), and two code comments. `docs/spikes/*.md` (the
  write-ups) are referenced from PLAN / IDEA / code and stay.
- Settings → About shows the version; no update check, no `check_update` command yet (listed in PLAN §3.5).

## Decisions

- **H1 — Scope split.** v0.6 = threat model + self-review, CI supply-chain checks, fuzz target, memory hardening,
  update check, release process, installer notes, `spikes/` removal, docs. **Later (v0.7 → v1.0):** signed
  auto-update (`tauri-plugin-updater` + update signing key), Authenticode code signing (certificate cost), optional
  git-history rewrite after a suspected compromise (IDEA §3.3), excluding the process from Windows Error Reporting
  dumps, continuous / scheduled fuzzing, SBOM, a third-party review before 1.0.
- **H2 — `cargo deny` replaces `cargo audit`.** `cargo deny check` reads the same RustSec advisory database as
  `cargo audit` and also checks licences, banned crates and sources, so running both would report every advisory
  twice. `deny.toml`:
  - `[graph] targets = ["x86_64-pc-windows-msvc"]` (the only shipped target; keeps Linux/macOS-only crates out of
    the licence check), `all-features = false`;
  - `[licenses] allow` = MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib,
    Unicode-3.0, Unlicense, CC0-1.0, BSL-1.0, MPL-2.0, CDLA-Permissive-2.0 (exact list = what `cargo deny list`
    shows today, reviewed in the PR); everything else — AGPL, GPL, LGPL, SSPL, unknown — fails;
  - `[advisories]` unmaintained / unsound / yanked = deny, with a commented `ignore` list (each entry needs a
    reason and a review date);
  - `[sources]` crates.io only; `[bans] multiple-versions = "warn"` (Tauri pulls duplicates we cannot fix).
- **H3 — `npm audit` scope.** `npm audit --audit-level=high` over all dependencies (dev included: vite, the Tauri
  CLI and the TypeScript toolchain run at build time and can inject code into the bundle). Moderate / low are
  printed, not failing. Fixes go through `npm audit fix` without `--force`, or an `overrides` entry with a comment.
- **H4 — Where the audits run.** A new job `audit` in `ci.yml` on `ubuntu-latest` (fast, no Windows minutes):
  `EmbarkStudios/cargo-deny-action@v2` + `npm ci --ignore-scripts && npm audit`. It runs on every PR / push like the
  rest of CI, plus a weekly `schedule` (Monday) so a new advisory shows up without a code change. A scheduled
  failure does not block anything by itself; it surfaces as a red run on `develop`.
- **H5 — Fuzzing: `cargo-fuzz` on demand, not on every CI run.** `crates/xpass-core/fuzz/` (own crate, excluded
  from the workspace, nightly + libFuzzer). Targets:
  - `parse` — `format::parse(data)`, then re-encode the header and check it round-trips byte-for-byte to
    `header_bytes` (catches parser / encoder disagreement, which would break the AAD);
  - `unlock_hello` — `UnlockedVault::unlock(data, Unlock::Hello { signature: fixed })` and `open_sibling` from a
    fixed small vault: exercises slot selection, AAD building and both AEAD opens. The **password path is not
    fuzzed** (a valid header asks Argon2 for up to 2 GiB per input), it is covered by unit tests;
  - `recovery_decode` — `recovery::decode(str)` on arbitrary UTF-8.

  Seed corpus: files written by the tests (one vault per slot combination), checked in under `fuzz/corpus/`.
  Runs: locally (`cargo +nightly fuzz run parse -- -max_total_time=600`; MSVC is supported by cargo-fuzz, WSL is
  the fallback) and a `fuzz.yml` workflow with `workflow_dispatch` only (inputs: target, minutes) on
  `ubuntu-latest` — `xpass-core` has no OS code, so Linux fuzzes the same parser; crashes are uploaded as an
  artifact. **Every CI run** keeps a cheap guard on stable: a test that runs `parse` + `unlock_hello` over every file
  in `fuzz/corpus/` and `fuzz/regressions/` (crash inputs are added there once fixed).
- **H6 — Page-locked key material (`VirtualLock`).** New `xpass-core::secret::LockedKey`: a 32-byte key in its own
  heap allocation (`Box`, stable address), locked with the `region` crate (MIT; `VirtualLock` on Windows, `mlock`
  elsewhere — keeps `xpass-core` free of `windows` bindings) on creation, zeroized and unlocked on drop. Locking is
  best effort: a failure is logged once and the key is still zeroized. Used for the VaultKey in `UnlockedVault`
  (clone = new locked box). KEKs (password / recovery / Hello) stay `Zeroizing` stack arrays: they live for one
  unwrap call. **Not locked:** the decrypted payload (`String`s spread over the heap; locking them would mean a
  custom allocator) — recorded as an accepted limit. Hibernation writes locked pages to `hiberfil.sys` anyway; the
  threat model says so and points to "lock on sleep" (already on).
- **H7 — Secrets at the IPC boundary.** Every command argument that carries a password, recovery key, export
  password or entry password becomes `Zeroizing<String>` (`zeroize` with the `serde` feature), including
  `EntryInput.password`, `Auth::Password`, `password_strength`, `sync_set_token` (PAT) and `copy_text` (found in
  the T30 review). The copy inside Tauri's IPC request buffer and in
  the WebView cannot be wiped — accepted limit, documented. Outgoing secrets (`reveal_secret`,
  `generate_password`, recovery kit) are already `Zeroizing` on the Rust side until serialized; same limit.
- **H8 — No reallocation copies when saving.** `UnlockedVault::save` serializes twice: first into a counting
  writer to get the exact length, then into a `Zeroizing<Vec<u8>>` with that capacity, so the plaintext JSON is
  never reallocated. Same for `xpass-interop` CSV export buffer if it grows a `Vec` (checked in T32).
- **H9 — Update check (D10).** `check_update` command in `src-tauri/src/update.rs`: `GET
  https://api.github.com/repos/bs135/xpass/releases/latest` with the existing `reqwest` client settings
  (rustls, user agent `xpass/<version>`, 10 s timeout, no token, no cookies), response capped at 256 KB, parses
  `tag_name` / `html_url` / `published_at`, compares semver with `CARGO_PKG_VERSION` (pre-releases and drafts
  ignored). Returns `{ current, latest, newer, url }`; `url` is only accepted if it starts with
  `https://github.com/bs135/xpass/releases/`. UI: Settings → About gets **Check for updates** and, when newer, a
  line with the version and **Open release page** (opened by Rust via `shell.rs`, never a WebView link). Optional
  setting **Check for updates at startup** (one request, at most once per 24 h, result shown as a small badge on
  the Settings icon): **default off** — the manual button needs no setting, and xPass stays silent on the network
  unless sync is on. Nothing is downloaded or installed; no new capability, CSP unchanged.
- **H10 — Release process without pushing to `main`.** The version bump moves **before** the merge into `main`:
  - `scripts/release-prepare.mjs` (Node, no deps): computes the next version from Conventional Commits since the
    last `v*` tag (same rules as today: `feat` → minor, `fix` / `perf` → patch, `!` / `BREAKING CHANGE` → minor
    while < 1.0), or takes it as an argument; writes `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`,
    `Cargo.lock` (`cargo update -p xpass --offline`) and turns `CHANGELOG.md`'s "Unreleased" heading into the
    version. Committed on `develop` as `chore(release): x.y.z` inside the milestone's docs PR.
  - `ci.yml` job `version` on PRs into `main`: the four files agree, the version is greater than the latest
    `v*` tag.
  - `release.yml`: reads the version from `package.json`; if the tag `vX.Y.Z` already exists → nothing to do;
    otherwise creates the tag through `mathieudutour/github-tag-action` with `custom_tag` (tag via the API, no
    branch push; still builds the release notes from commits) and builds / publishes as today. The
    `chore(release)` push and `[skip ci]` disappear, and `develop` == `main` after a release without a back-merge.
  - The user re-enables the ruleset **"Protect Production/Main"** after T34 is merged (I do not change repo
    settings). If the ruleset also restricts tag creation, `v*` tags need a separate rule or none.
- **H11 — Installer.** Keep **NSIS and MSI** (MSI is what managed / Intune installs want). NSIS:
  `installMode: "currentUser"` (no UAC prompt; the vault is per user anyway), `languages: ["English",
  "Vietnamese"]` with the language selector, WebView2 `downloadBootstrapper` (default, keeps the installer small).
  The release publishes `SHA256SUMS.txt` for the assets (unsigned builds: the checksum is the integrity check).
  README "Install": SmartScreen steps with what the warning means, how to check the checksum
  (`Get-FileHash`), per-user install location, where data lives and that uninstall keeps `%APPDATA%\xpass`.
- **H12 — Self-review output.** `docs/THREAT_MODEL.md` holds assets, trust boundaries, attackers (stolen locked
  device, leaked GitHub repo / PAT, malicious CSV, compromised npm / crate dependency, malware while unlocked,
  someone at the unlocked PC), what is protected and what is not, and the **accepted limits** list (JS strings,
  IPC buffers, serde scratch buffers, `csv` buffers, payload not page-locked, hibernation, git history and its
  metadata, unsigned builds, PAT in Credential Manager readable by user-level malware). The **self-review** is a
  checklist table in the same file (IDEA §3.1–3.7 + v0.4/v0.5 additions) with status per item and a link to the
  code or test that proves it; findings not fixed in v0.6 get a row in "Accepted limits" or a later-milestone note.

## IPC additions

| Group | Commands |
|---|---|
| App | `check_update()` → `{ current, latest, newer, url }`; `open_release_page()` (opens the URL from the last check); settings field `checkUpdatesOnStartup` (default `false`) |

Changed signatures only (no frontend change): secret arguments become `Zeroizing<String>` (H7). New error codes:
`update_failed` (network / HTTP / parse; no detail from the response body).

## Tasks

Each task is one feature branch off an up-to-date `develop` and one PR into `develop`. T30–T33 and T35–T36 do not
depend on each other; T34 should be merged before the release PR; T37 is last.

### T30 — Threat model and self-review (`docs/t30-threat-model`)

- `docs/THREAT_MODEL.md` per H12, written against the code as of `develop` (not the plans).
- Self-review checklist with status per item; every gap either points to the v0.6 task that fixes it (T31–T36) or
  goes to "Accepted limits" / "Later".
- README "Security model (summary)" links to it.

**Done when:** the user has read it and agrees with the accepted-limits list.

### T31 — Supply-chain checks in CI (`ci/t31-deny-audit`)

- `deny.toml` per H2; job `audit` in `ci.yml` per H3 / H4 (+ weekly schedule).
- Fix or justify whatever the first run reports (each `ignore` with a reason and a date).
- `CONTRIBUTING`-style note in README "Development": how to run `cargo deny check` and `npm audit` locally.

**Done when:** the `audit` job is green on the PR; a deliberately added GPL crate fails it (checked on a throwaway
commit, not merged).

### T32 — Memory hardening (`feature/t32-locked-keys`)

- `xpass-core::secret::LockedKey` with `region` (H6); `UnlockedVault.vault_key` uses it; unit tests: locked flag
  reported where the OS allows it, zeroized on drop, clone gets its own allocation.
- `save` without reallocation copies (H8).
- Secret command arguments as `Zeroizing<String>` (H7); `EntryInput.password` as `Option<Zeroizing<String>>`.
- `cargo tree -p xpass-core` licence of `region` listed in the PR; T31's `cargo deny` stays green.

**Done when:** CI green; manual: unlock, edit, sync, lock still work; the app runs with the default working-set
size (no `VirtualLock` failure logged).

### T33 — Fuzz targets (`test/t33-fuzz`)

- `crates/xpass-core/fuzz` with `parse`, `unlock_hello`, `recovery_decode` and a seed corpus (H5); workspace
  `exclude` updated.
- Stable corpus-replay test in `xpass-core/tests/`.
- `.github/workflows/fuzz.yml` (`workflow_dispatch`, target + minutes inputs, crash artifact).
- README "Development": how to fuzz locally.

**Done when:** each target ran ≥ 30 min locally or in `fuzz.yml` without a crash (or crashes fixed with a
regression input); CI green.

### T34 — Release process for a protected `main` (`ci/t34-release-bump`)

- `scripts/release-prepare.mjs` + `npm run release:prepare`; `version` job in `ci.yml`; `release.yml` per H10.
- `docs/PLAN.md` §7 Workflow updated (bump in the release PR, workflow only tags).
- Checked without a real release: the script on a scratch branch (computes 0.6.0 from the commits since
  `v0.5.0`), and the `release.yml` logic reviewed against the v0.5.0 run log.

**Done when:** CI green; the steps for the user to re-enable "Protect Production/Main" are in the PR description.
The real proof is the v0.6.0 release itself.

### T35 — Update check (`feature/t35-update-check`)

- `update.rs` + `check_update` / `open_release_page` commands, setting `checkUpdatesOnStartup` (H9).
- Settings → About: button, result line, startup toggle; badge on the Settings entry when a newer version was
  found at startup. i18n en/vi.
- Tests: semver comparison (pre-release, equal, older, malformed tag), URL allow-list, response size cap, setting
  default `false`, startup check throttled to once per 24 h.

**Done when:** CI green; manual: against the real GitHub API, a build at 0.5.0 shows "up to date" and a build
with the version lowered to 0.4.0 shows v0.5.0 as new; offline → clear error; no request at startup with the
setting off (checked with a proxy or Resource Monitor).

### T36 — Installer and spikes clean-up (`chore/t36-installer-spikes`)

- `tauri.conf.json` NSIS options (H11); `release.yml` adds `SHA256SUMS.txt` to the release.
- README "Install" rewritten per H11.
- Remove `spikes/`; drop the `exclude` / oxlint ignore entries; code comments point to `docs/spikes/*.md`; README
  layout updated. `docs/spikes/` stays.

**Done when:** CI green; manual: a local `npm run tauri build` NSIS installer installs per user without UAC,
shows the language selector, upgrades an installed v0.5.0 in place, uninstall keeps `%APPDATA%\xpass`.

### T37 — Docs and release prep (`docs/t37-v0.6-docs`)

- `docs/PLAN.md`: v0.6 status, §3.5 IPC, §5 fuzz row, §6 risks (supply chain, update check), v0.7 → v1.0 outline
  from H1.
- `CHANGELOG.md` 0.6.0; this plan → **done** with an "As built" section; `npm run release:prepare` → commit
  `chore(release): 0.6.0`.

## As built

Differences from the decisions and task texts above, found while implementing:

- **T31 (H2):** `[bans] multiple-versions = "allow"` instead of `"warn"` (Tauri / windows duplicates are not
  fixable and only add noise). The licence allow-list is exactly what the graph uses today (no
  `Apache-2.0 WITH LLVM-exception`, no `CDLA-Permissive-2.0`; cargo-deny warns about unused entries). First run:
  `url` 2.5.9 and `cookie` 0.18.3 for two advisories; `chrono` (unmaintained, only via `keepass`) ignored with a
  review date. Separate fix PR: `release.yml` refreshes `Cargo.lock` with `cargo update --workspace` and checks
  that all version files agree before tagging (v0.5.0 had shipped with `xpass 0.4.0` in the lock).
- **T32 (H6):** `LockedKey` keeps the key in a page-aligned 4 KiB heap allocation of its own, so unlocking one key
  never unlocks a page shared with another clone; with any other page size it does not lock. `region` 4 (MIT,
  same `windows-sys` 0.61 as the app). `UnlockedVault::key_locked()`; `VaultSession::new` logs once when locking
  failed.
- **T32 (H7):** no signature change was needed. Every command already moves secret arguments into `Zeroizing`
  on entry (the move wipes the very buffer serde produced), and `Auth` / `EntryInput` are `ZeroizeOnDrop`; the gap
  listed under "What already exists" was wrong. What T32 fixed instead: `Debug` of `EntryInput` and `Generated`
  printed the password (never logged, now redacted), and the password generator built passwords in growing
  `String`s (reallocation copies) — now sized up front.
- **T32 (H8):** `serialize_payload` counts the JSON size with a byte-counting writer, then serializes into an
  exact-capacity `Zeroizing<Vec<u8>>`; debug builds assert the capacity never changed. `xpass-interop`'s CSV
  export pre-sizes its buffer from an estimate, so it can still grow — left as is: the plaintext CSV file it
  writes is the bigger exposure.

## Manual test checklist (v0.6.0)

Use a test vault (`$env:XPASS_DATA_DIR = "$env:TEMP\xpass-v06"`) with synthetic entries.

Memory / IPC
- [ ] Create, unlock (password, recovery, Hello), edit, change password, rotate, export, sync join: all work as in
      v0.5.
- [ ] No "VirtualLock failed" line in the log on a normal start.

Update check
- [ ] About → Check for updates: current version shown; newer version (simulate by building with a lower
      version) shows the version and **Open release page** opens the browser on the GitHub release.
- [ ] Offline → `update_failed` message, app unaffected.
- [ ] Startup check off (default) → no request to `api.github.com` at start with sync off; on → one request, badge
      shown when newer.

Installer
- [ ] NSIS installs per user without UAC; language selector; upgrade over v0.5.0 keeps the vault and settings.
- [ ] MSI installs; uninstall of either keeps `%APPDATA%\xpass`.
- [ ] SmartScreen steps in README match what Windows shows; `Get-FileHash` matches `SHA256SUMS.txt`.

Release process
- [ ] Release PR develop → main has the `chore(release): 0.6.0` commit and a green `version` job.
- [ ] After merge: tag `v0.6.0` and the release appear; **no** commit pushed to `main` by the workflow.
- [ ] User re-enables "Protect Production/Main"; a direct push to `main` is refused.

Baseline
- [ ] Release build: capabilities file and CSP unchanged, no new Tauri plugin, devtools off.

## Out of scope

Signed auto-update and the update signing key, code-signing certificate, git-history rewrite, WER dump exclusion,
scheduled fuzzing, SBOM, third-party audit (all v0.7 → v1.0, H1); page-locking the decrypted payload (custom
allocator); wiping Tauri / WebView IPC buffers; new features (TOTP, auto-type, custom-field editing).

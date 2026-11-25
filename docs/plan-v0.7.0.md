# v0.7.0 Plan: Trusted updates

Status: **approved** (2026-11-25). Parent: [PLAN.md §4 v0.7 → v1.0](PLAN.md#v07--v10--release-outline).

## Goal

Close the "later" rows of the v0.6 self-review ([THREAT_MODEL.md §6](THREAT_MODEL.md#6-security-self-review-v06)
R4, R28, R29) that do not need money or an outside party:

- **Signed in-app update**: xPass downloads a release, checks its signature against a public key built into the app
  and installs it, only after the user says so. Replaces the notify-only check of v0.6 (H9).
- **No crash dumps with secrets**: keep the process out of Windows Error Reporting (WER) dumps.
- **Remove old vaults from the sync repository's history** after a suspected compromise (IDEA §3.3).
- **Release supply chain**: SBOM and build-provenance attestations per release, scheduled fuzzing.
- **Authenticode signing with a self-signed certificate** (user's decision, U8): every binary carries a signature
  whose thumbprint users can check. It does **not** remove the SmartScreen warning.

A third-party review is a v1.0 item.

The baseline stays: **no capability change, CSP unchanged** (`connect-src` stays IPC only). One new Tauri plugin
(`tauri-plugin-updater`), used from Rust only.

## What already exists

- `src-tauri/src/update.rs` (v0.6 T35): `check_update` / `update_status` / `open_release_page`, event
  `update://status`. Rust calls `api.github.com/repos/bs135/xpass/releases/latest` (rustls, HTTPS only, 256 KB
  cap, no token), compares `x.y.z` with `CARGO_PKG_VERSION`, opens only `https://github.com/bs135/xpass/releases/…`.
  Settings `checkUpdatesOnStartup` (default **off**, at most once per 24 h) and `lastUpdateCheckAt`. Nothing is
  downloaded.
- `lib.rs`: plugins = `single-instance` only. `RunEvent::Exit` → `lock_watch::lock_app(LockReason::Exit)` then
  `sync_worker::push_on_exit` (blocking, 8 s): an update that ends the process some other way must do both itself.
- `capabilities/default.json`: event listen/unlisten and window controls only. CSP in `tauri.conf.json` as in v0.6.
- `tauri.conf.json` bundle: `nsis` + `msi`, NSIS `currentUser`, English / Vietnamese with selector, WebView2
  `downloadBootstrapper`. No `createUpdaterArtifacts`, no `plugins` section.
- `release.yml`: on a push to `main`, tag `v<package.json version>` if missing (H10), `tauri-action@v0` builds and
  publishes NSIS + MSI with the CHANGELOG section as body, then `SHA256SUMS.txt` from the uploaded `*.exe` / `*.msi`.
  `workflow_dispatch` builds artifacts only. No signing secrets, no environment.
- `fuzz.yml`: `workflow_dispatch` only (target, minutes), `ubuntu-latest`, nightly + ASan, crash artifacts.
  Corpus is not kept between runs. `ci.yml` replays seeds and regressions on every run; `audit` job weekly.
- `deny.toml`: windows-msvc graph only, permissive licence list, crates.io only.
- `xpass-core::secret::LockedKey`: the VaultKey in its own page-aligned 4 KiB `VirtualLock`ed allocation — a known
  address and size, so it can also be excluded from WER reports.
- Sync: `RemoteStore` (`get` / `put` with compare-and-swap on the **blob** SHA) in `xpass-core::sync`;
  `GitHubStore` in `src-tauri/src/github.rs` (Contents API, fine-grained PAT with Contents R/W). `sync-state.json`
  keeps blob SHAs, not commit SHAs, so rewriting commits does not change what a device compares against.
- Repository `bs135/xpass` is **public** and **MIT** (relevant for SignPath Foundation and free attestations).
- Users on v0.6.x have no updater: they install v0.7.0 by hand once (their v0.6 check already points them to it).

## Decisions

- **U1 — Scope split.** v0.7 = signed in-app update (T38, T39), WER exclusion (T40), sync history rewrite (T41),
  SBOM + provenance + scheduled fuzzing (T42), self-signed Authenticode (T43), docs (T44). **Later (optional):** a
  publicly trusted certificate if SmartScreen ever matters (U8 table). **v1.0:** third-party review and what it needs (U14). Features (TOTP, auto-type, custom-field
  editing, password health) stay in "Later" of PLAN §1.
- **U2 — Updater from Rust only.** Add `tauri-plugin-updater` 2 with `default-features = false` and only the
  rustls feature (no native-tls, no zip/tar: Windows updates are the installers themselves). The plugin is
  initialised in `lib.rs` and called from `update.rs` through `UpdaterExt`; **no `updater:*` permission is added**,
  so the WebView cannot call the plugin's own commands (checked in T39: `invoke('plugin:updater|check')` is
  refused). No `process` plugin: the installer restarts the app. CSP unchanged. `cargo deny` must stay green (new
  crates listed in the PR, e.g. `minisign-verify`).
- **U3 — Manifest and endpoint.** `tauri-action` writes `latest.json` (version, notes, date, per-platform URL +
  signature) and uploads it with `.sig` files to the release. The only endpoint is
  `https://github.com/bs135/xpass/releases/latest/download/latest.json` (drafts and pre-releases are never
  "latest"). On top of the signature check, `update.rs` accepts a download URL only under
  `https://github.com/bs135/xpass/releases/download/v<version>/` and a version that is plain `x.y.z` and newer than
  the running one (no downgrade, no pre-release). `updaterJsonPreferNsis: true`; T39 checks that an MSI install
  picks the `-msi` entry (the plugin chooses by the bundle type built into the binary) and an NSIS install the
  NSIS one. Windows `installMode: "passive"` (progress window, no questions). Notes from `latest.json` are shown
  as plain text, capped at 4 000 characters.
- **U4 — Update signing key (the user's).** Minisign key pair from `npx tauri signer generate`, **created by the
  user on their own PC, with a password**. I never generate, read, print or commit the private key or its
  password. The user:
  1. runs `npx tauri signer generate -w "$env:USERPROFILE\.tauri\xpass-updater.key"` and keeps an offline backup
     of the key file and its password (losing either = no more in-app updates; users would reinstall by hand once);
  2. creates a GitHub **Environment `release`** (Settings → Environments): deployment branches = `main` only,
     optional "Required reviewers" = themself; adds the environment secrets `TAURI_SIGNING_PRIVATE_KEY` (content of
     the key file) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`;
  3. pastes the **public** key (`.key.pub`, not secret) into the chat or the PR; T38 puts it in `tauri.conf.json`.

  Key rotation: a new public key can only reach users through an update signed with the old key; if the old key
  is lost or leaked, users must reinstall by hand (README says so). Fork PRs never see the secrets (only
  `release.yml` on `main` uses the environment).
- **U5 — Release pipeline.** `createUpdaterArtifacts: true` lives in a new `src-tauri/tauri.release.conf.json`
  merged only by `release.yml` (`tauri-action` `args: --config src-tauri/tauri.release.conf.json`), so local
  `npm run tauri build`, `ci.yml` and the `workflow_dispatch` build keep working without the key. The
  `build-and-release` job gets `environment: release`; a first step fails the release with a clear message when
  the signing secrets are empty. After upload, a step checks that the release has `latest.json`, one `.sig` per
  installer, and that `latest.json` names this version and only `github.com/bs135/xpass/releases/download/` URLs.
  `SHA256SUMS.txt` keeps covering `*.exe` / `*.msi` (plus SBOMs from T42).
- **U6 — Install flow: always user-confirmed.** Settings → About: **Check for updates** (as today) → when newer,
  the version, date, notes and **Install and restart** (plus **Open release page**). Install:
  1. download with progress (`update://progress` `{ downloaded, total }`); the plugin verifies the signature
     before anything runs — a bad signature → `update_signature` error, nothing installed;
  2. a confirm dialog: "xPass will lock, push unsynced changes and restart";
  3. `lock_app(LockReason::Update)` (new reason), `push_locked` with the 8 s bound, clipboard cleared;
  4. `install` → the NSIS installer (passive) replaces the app and starts it again.

  Nothing is downloaded or installed without a click. The startup check stays opt-in (default off) and only shows
  the badge; it never downloads. The Rust check replaces the GitHub API call of v0.6 (one source of truth:
  `latest.json`); `open_release_page` keeps working with `…/releases/tag/v<version>`. Errors carry no response
  content. Timeout 15 s for the manifest; downloads have no total timeout but fail on 30 s of silence.
- **U7 — Migration and testing.** v0.6.x → v0.7.0 is a manual install (README and CHANGELOG say so). The first
  real in-app update is v0.7.0 → the next release (v0.7.1 or v0.8.0); its manual checklist item is ticked then.
  Before the release, T39 is tested locally without the real key: **debug builds only** read
  `XPASS_UPDATE_ENDPOINT` and `XPASS_UPDATE_PUBKEY` (like `XPASS_DATA_DIR`; release builds ignore them), a
  throwaway key pair made in a scratch folder signs a locally built installer, and `latest.json` is served from
  `http://localhost` (the localhost URL is allowed only in debug builds). The throwaway key never enters the repo.
- **U8 — Authenticode: self-signed certificate (decided by the user, 2026-11-25).**
  - **What it gives:** every shipped binary (`xpass.exe`, the NSIS installer and uninstaller, the MSI) carries an
    Authenticode signature with an RFC 3161 timestamp. Anyone can check that a file, **also an installed
    `xpass.exe` later on**, was signed by our key: `Get-AuthenticodeSignature` → `SignerCertificate.Thumbprint`
    equals the thumbprint published in the README, in `docs/codesign/xpass-codesign.cer` and in every release
    body. The timestamp keeps signatures valid after the certificate expires.
  - **What it does not give:** no trust chain. Windows shows the signature as not trusted (`Status` =
    `UnknownError`, "terminated in a root certificate which is not trusted") and SmartScreen still says "Unknown
    publisher" / "Windows protected your PC". The README keeps the SmartScreen steps.
  - **Users are never asked to import the certificate** into "Trusted Root Certification Authorities" or "Trusted
    Publishers": whoever holds that private key could then sign anything that their Windows trusts. The README
    says so; checking the thumbprint is the supported way. (The user may choose to trust it on their own PCs; that
    is their call, not documented as a step.)
  - **Certificate (the user's, like U4):** created by the user on their own PC, never by me, e.g.
    `New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=bs135 (xPass code signing)" -KeyAlgorithm RSA
    -KeyLength 3072 -HashAlgorithm SHA256 -KeyExportPolicy Exportable -CertStoreLocation Cert:\CurrentUser\My
    -NotAfter (Get-Date).AddYears(5)` (EKU = code signing only), then `Export-PfxCertificate` with a password into
    a file they keep offline. Secrets in the `release` environment: `WINDOWS_CERTIFICATE` (base64 of the PFX) and
    `WINDOWS_CERTIFICATE_PASSWORD`. The user gives me only the **public** part: the `.cer` from
    `Export-Certificate` and its thumbprint. I never open, read or commit the PFX or its password.
    **Done 2026-11-25:** `CN=bs135 (xPass code signing)`, RSA 3072, EKU Code Signing only, valid to 2031-11-25,
    thumbprint `4D3E068AED69DE2C88578FC2CA46439E78E73D7D`; secrets set in the `release` environment; public
    certificate at `%USERPROFILE%\.xpass-signing\xpass-codesign.cer` (copied into `docs/codesign/` by T43).
  - **In the build:** `release.yml` imports the PFX into the runner's `CurrentUser\My` store before
    `tauri-action` and removes it afterwards (`if: always()`); `tauri.release.conf.json` sets
    `bundle.windows.certificateThumbprint`, `digestAlgorithm: "sha256"` and a `timestampUrl` (RFC 3161,
    `tsp: true`). Tauri signs inside the build, **before** the updater signature is computed (U4), so `.sig` files
    and `SHA256SUMS.txt` cover the signed bytes. A post-build step checks with `Get-AuthenticodeSignature` that
    every installer and the extracted `xpass.exe` carry exactly the expected thumbprint. Local and
    `workflow_dispatch` builds stay unsigned.
  - **Rotation / loss:** a new certificate = a new thumbprint, announced in the release notes and README with the
    date; old releases keep their old, timestamped signature. A leaked PFX: publish the fact, rotate, and remind
    users that the thumbprint alone was never a trust root (checksums + attestations, T42, still hold).

  For reference, if a publicly trusted certificate is wanted later (prices approximate, check when deciding):

  | Option | Cost | SmartScreen | CI fit | Notes |
  |---|---|---|---|---|
  | SignPath Foundation (OSS) | free | builds reputation over time | GitHub Action, per-release signing request | Needs a public OSI-licensed repo built on GitHub Actions (xpass: yes) and acceptance by SignPath; the certificate names SignPath Foundation, not bs135. |
  | Certum Open Source Code Signing | ~€30–70 / year + card or SimplySign | builds over time | hard (SimplySign needs an interactive login) | Issued to the individual; identity check. |
  | Commercial OV (Sectigo, SSL.com, DigiCert …) | ~$200–500 / year + cloud HSM fee | builds over time | good with a cloud HSM (e.g. eSigner, KeyLocker) | Keys must be on an HSM / token since June 2023. |
  | EV | ~$300–700 / year | no instant reputation since 2024 | as OV | Little gain over OV for us. |
  | Azure Trusted Signing | ~$10 / month | builds over time | very good (`signCommand`) | Identity validation for individuals is limited to a few countries; probably not available to the user — verify. |

  Switching later only changes the certificate source in T43's steps (`certificateThumbprint` or `signCommand`);
  a post-build signer such as SignPath would mean re-running `tauri signer sign` on the signed installers and
  regenerating `latest.json`.
- **U9 — Crash dumps (WER).** At startup, on Windows (new `src-tauri/src/crash_dump.rs`, `windows-sys` feature
  `Win32_System_ErrorReporting`):
  - `WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP)` — WER reports for this process carry no heap;
  - `WerRegisterExcludedMemoryBlock` for each `LockedKey` page (registered on creation, unregistered on drop;
    exposed by `xpass-core` as address + length only on Windows, through the app, keeping `xpass-core` free of WER);
  - `WerAddExcludedApplication("xpass.exe", FALSE)` (per user, HKCU) — no WER report at all; the NSIS
    uninstaller removes the value again (Tauri NSIS `installerHooks`).

  For the whole process lifetime, not only while unlocked: a locked xPass has nothing to debug from a dump either,
  and switching at runtime would race with a crash. Not covered: an administrator's `LocalDumps` policy if it
  ignores the exclusion, a debugger or Task Manager "Create dump file" (A5), WebView2's own crash reports
  (Crashpad in the WebView2 user-data folder) — T40 checks whether a documented WebView2 option turns them off
  (keeping Tauri's default browser arguments); if not, it is an accepted limit. Each call is best effort: a
  failure is logged once, the app runs.
- **U10 — Sync history rewrite.** Settings → Sync → **Remove old versions from GitHub** (re-auth required, online,
  sync idle with no local changes; offered on the rotation result screen too, since it only helps after a
  rotation). Through the Git Data API with the existing PAT (Contents R/W covers refs):
  1. read the branch ref → commit → its **tree** (all files of the branch, not only the vault);
  2. create a commit with that tree and **no parent** ("xpass: history removed");
  3. `PATCH /git/refs/heads/<branch>` with `force: true`;
  4. read the vault file back and check its blob SHA equals the local `base_sha`.

  If another device pushed between 1 and 3, its commit is dropped from the branch, but its own local file still
  has the edits: its next sync sees a remote that moved "back", merges (LWW, tombstones) and pushes again — the
  same path as a rolled-back remote (A8); a `MemoryStore` test proves no edit is lost. Branch protection or a
  ruleset that refuses force updates → `history_rewrite_refused`, nothing changed. The dialog says what it cannot
  do: GitHub may keep old objects reachable by SHA for a while, forks / clones / caches keep theirs, and a full
  purge needs GitHub Support; other files' history in that branch is removed too (shown when the tree has more than
  the vault file).
- **U11 — Scheduled fuzzing.** `fuzz.yml` gets `schedule` (weekly, Sunday) with all targets × 15 min; the
  working corpus is kept between runs with `actions/cache` (key per target and run, restore by prefix), so
  coverage grows. A crash makes the run red and uploads the input as today; fixes add it to
  `fuzz/regressions/`. Manual dispatch keeps its inputs.
- **U12 — SBOM and build provenance.** In `release.yml`, after the build:
  - SBOMs in CycloneDX JSON: Rust from `cargo cyclonedx` for the `xpass` package and the Windows target; npm from
    `npm sbom --sbom-format cyclonedx --omit dev` (what is bundled into the frontend). Uploaded as
    `xpass_<version>_sbom-rust.cdx.json` / `…_sbom-npm.cdx.json` and listed in `SHA256SUMS.txt`;
  - `actions/attest-build-provenance` for the installers and `actions/attest-sbom` linking each SBOM to them
    (free for public repos; job permissions `id-token: write`, `attestations: write`). Users can check a download
    with `gh attestation verify <file> --repo bs135/xpass`. README "Check the download" explains it.
- **U13 — Threat model changes.** A10 becomes "fake or tampered update": the signature (U4) and the URL / version
  checks (U3) stop it; the user still confirms. New **A11 — update signing key and GitHub account both
  compromised**: a signed malicious update reaches every user who clicks Install. Reduced by: key password,
  environment secret limited to `main`, protected `main` (ruleset), optional required reviewer, nothing installs
  without a click. Rows R4, R28, R29 → OK (or limit, with what U9 does not cover); R26 notes the self-signed
  signature. Accepted limits: "Unsigned builds" becomes "**Self-signed builds**: the signature proves which key
  signed a file only to someone who checks the thumbprint; Windows and SmartScreen do not trust it" (checksums and
  attestations stay); "Update check trusts GitHub" is replaced by A11. New attacker note: whoever has the PFX and
  its password can sign binaries with our thumbprint — same protection as the update key (environment secret,
  offline backup, rotation per U8).
- **U14 — Before 1.0 (not v0.7).** A third-party review needs: a frozen format v1 spec (`docs/spikes/vault-format.md`
  promoted to `docs/FORMAT.md`), `SECURITY.md` with supported versions, private vulnerability reporting turned on
  by the user, the threat model up to date, and a scope (xpass-core crypto + format + sync, IPC boundary, update
  chain). Planned in the v1.0 plan, together with the choice of reviewer (e.g. a community audit, OSTIF, or a paid
  firm).

## IPC additions

| Group | Commands |
|---|---|
| App | `check_update()` → `{ current, latest, newer, url, notes, date, checkedAt }` (from `latest.json`); `install_update()` (download → verify → lock → push → install; event `update://progress` `{ downloaded, total }`); `update_status`, `open_release_page` unchanged |
| Sync | `sync_rewrite_history(auth)` → `{ otherFiles: bool }` (re-auth like export) |

New lock reason `update`. New error codes: `update_signature` (signature or URL / version check failed),
`update_failed` (network / HTTP / parse, as today), `history_rewrite_refused` (branch protected / ruleset),
`history_rewrite_failed`.

## Tasks

Each task is one feature branch off an up-to-date `develop` and one PR into `develop`. **T38 needs the user's key
first** (U4 steps 1–3). T39 needs T38. **T43 needs the user's certificate** (U8) and goes after T38 (both change
`release.yml` and `tauri.release.conf.json`). T40, T41 and T42 do not depend on anything and can go in any order.
T44 is last.

### T38 — Update signing in the release pipeline (`ci/t38-updater-artifacts`)

- Before: the user creates the key pair, the `release` environment and its two secrets, and gives me the public
  key (U4).
- `tauri.conf.json`: `plugins.updater` with `pubkey`, the endpoint (U3) and `windows.installMode: "passive"`.
- `src-tauri/tauri.release.conf.json` with `bundle.createUpdaterArtifacts: true` (U5).
- `release.yml`: `environment: release`, secret presence check, `args: --config …`, `updaterJsonPreferNsis: true`,
  post-upload check of `latest.json` and `.sig` files (U5).
- README "Development" → "Releasing": what the secrets are, that they are the user's, key backup and rotation.

**Done when:** CI green; a `workflow_dispatch` build still works without secrets; a local
`npm run tauri build -- --config src-tauri/tauri.release.conf.json` run by the user with the real key in their own
shell produces the `.sig` files. The real proof is the v0.7.0 release (`latest.json` + `.sig` assets).

### T39 — In-app update (`feature/t39-updater`)

- `tauri-plugin-updater` per U2 (features, no capability); `update.rs` rewritten on top of it: check from
  `latest.json`, URL / version checks (U3), `install_update` with progress, lock reason `update`, lock + push before
  install (U6); debug-only endpoint / public-key override (U7). The startup throttle and opt-in setting stay; the
  256 KB response cap of v0.6 goes with the API call (the plugin fetches `latest.json` itself — the PR says
  whether it caps the size; if not, accepted: the file comes only from our release).
- Settings → About: version, date, notes (plain text), **Install and restart**, progress bar, errors; confirm
  dialog. i18n en/vi.
- Tests: version / URL acceptance (newer, equal, older, pre-release, foreign host, other tag path, `http`),
  notes cap, startup check still opt-in and daily, the lock-and-push order before install (fake installer hook).
- PR lists the new crates and `cargo deny` result; capabilities file and CSP unchanged in the diff.

**Done when:** CI green; manual (U7, local, throwaway key): a debug build at 0.6.99 finds 0.7.0, shows the notes,
installs it after confirm, the vault was locked and the pending edit pushed before the installer started, the new
version starts; a tampered installer (one byte changed) or a `latest.json` signed with another key is refused
with nothing installed; `invoke('plugin:updater|check')` from the devtools console of a debug build is refused.

### T40 — Keep secrets out of crash dumps (`feature/t40-wer-exclusion`)

- `crash_dump.rs` per U9; `LockedKey` page registered / unregistered; NSIS uninstall hook removes the
  `ExcludedApplications` value.
- WebView2 crash reporting: check the documented options; adopt one only if it keeps Tauri's default browser
  arguments, otherwise record it as an accepted limit.
- Debug-only `XPASS_CRASH_TEST=1` that faults on purpose while unlocked (for the manual test only).

**Done when:** CI green; manual: with the debug crash trigger, no report or dump with heap appears under
`%LOCALAPPDATA%\CrashDumps` or `%PROGRAMDATA%\Microsoft\Windows\WER\ReportArchive`; uninstall removes the
registry value; normal start logs no WER failure.

### T41 — Remove old versions from the sync repository (`feature/t41-history-rewrite`)

- `GitHubStore::rewrite_history` (Git Data API, U10) — kept out of the `RemoteStore` trait, which stays
  `get` / `put`; `state/sync.rs` checks the preconditions (online, idle, clean, re-auth).
- Settings → Sync button + dialog (what it does and does not do, other files warning); link on the rotation result
  screen. i18n en/vi.
- Tests: preconditions refused (locked, dirty, offline, sync off); request sequence against a mock HTTP layer
  (parentless commit with the same tree, forced ref update, read-back check); `xpass-core` sync test: a remote that
  moves back to an older blob after a concurrent push loses no edit.

**Done when:** CI green; manual on a private test repo with two PCs (or two data dirs): after a rotation and the
rewrite, the branch has one commit, the old commit SHAs give 404 on the branch view, both devices keep syncing and
keep their edits; a ruleset blocking force pushes → clear refusal, nothing changed.

### T42 — SBOM, provenance and scheduled fuzzing (`ci/t42-sbom-provenance-fuzz`)

- `release.yml`: SBOMs, attestations, permissions, SHA256SUMS additions (U12).
- `fuzz.yml`: weekly schedule, defaults for scheduled runs, corpus cache (U11).
- README "Check the download": `gh attestation verify`, SBOM files.

**Done when:** CI green; one manual `fuzz.yml` run with the cache restores the corpus of the previous run; the
SBOM and attestation steps are checked with `workflow_dispatch` where possible (attestations need a real release:
the proof is v0.7.0, `gh attestation verify` on its installers passes).

### T43 — Self-signed Authenticode signing (`ci/t43-self-signed-signing`)

- Before: the user creates the certificate, exports the PFX, adds `WINDOWS_CERTIFICATE` /
  `WINDOWS_CERTIFICATE_PASSWORD` to the `release` environment and gives me the `.cer` and the thumbprint (U8).
- `docs/codesign/xpass-codesign.cer` (public certificate) and its thumbprint in the README.
- `tauri.release.conf.json`: `certificateThumbprint`, `digestAlgorithm`, `timestampUrl`, `tsp`.
- `release.yml`: secret presence check, PFX import before `tauri-action` and removal in an `if: always()` step,
  post-build `Get-AuthenticodeSignature` check (installers + the `xpass.exe` they contain), thumbprint line in the
  release body.
- README "Check the download": how to check the thumbprint, what the "not trusted" status means, why not to import
  the certificate; SmartScreen section stays.

**Done when:** CI green; `workflow_dispatch` build still works (unsigned) without the secrets; the user signs a
local build once with the certificate from their own store (`--config src-tauri/tauri.release.conf.json`) and
`Get-AuthenticodeSignature` shows the expected thumbprint and a timestamp. The real proof is v0.7.0.

### T44 — Docs and release prep (`docs/t44-v0.7-docs`)

- `THREAT_MODEL.md` per U13 (A10, A11, §4 rows for update, WER and code signing, §5, §6 R4 / R26 / R28 / R29);
  README (Features, Update section: how in-app update works, the one manual install from v0.6.x, what to do if the
  update key ever changes); `PLAN.md` (v0.7 status, §3.5 IPC, §5, §6 risks: update key, history rewrite); v0.8 / v1.0 outline
  from U1 / U8 / U14.
- `CHANGELOG.md` 0.7.0; this plan → **done** with "As built"; `npm run release:prepare` → `chore(release): 0.7.0`.

## Manual test checklist (v0.7.0)

Use a test vault (`$env:XPASS_DATA_DIR = "$env:TEMP\xpass-v07"`) with synthetic entries and a private test repo.

Update (local, before the release — U7)
- [ ] Debug build at 0.6.99 + throwaway key: check finds 0.7.0, notes shown as plain text, install after confirm,
      vault locked and pending edit pushed before the installer, app restarts at 0.7.0 with the vault intact.
- [ ] Tampered installer / wrong key / URL outside `bs135/xpass/releases/download/` / older version → refused,
      nothing installed.
- [ ] Offline → `update_failed`, app unaffected. Startup check off (default) → no request at start.
- [ ] WebView cannot call `plugin:updater|*`.

Update (real, after the release)
- [ ] v0.7.0 release has both installers, their `.sig`, `latest.json`, SBOMs, `SHA256SUMS.txt`;
      `gh attestation verify` passes for both installers.
- [ ] v0.6.1 → v0.7.0 by hand over an existing install keeps vault and settings.
- [ ] **Next release** (v0.7.1 / v0.8.0): v0.7.0 NSIS install and v0.7.0 MSI install each update in-app.

Crash dumps
- [ ] Debug crash trigger while unlocked → no dump / report with heap; uninstall removes the WER exclusion value.

Code signing
- [ ] v0.7.0 installers, the installed `xpass.exe` and the uninstaller: `Get-AuthenticodeSignature` shows the
      published thumbprint and a timestamp; status "not trusted" as documented.
- [ ] SmartScreen behaviour matches the README (still warns).
- [ ] The in-app update to the next release installs a signed build (thumbprint checked after the update).

History rewrite
- [ ] Rotate, then remove old versions: one commit left, both PCs keep syncing, no edit lost (concurrent edit on
      the second PC during the rewrite included).
- [ ] Force push blocked by a ruleset → refusal, nothing changed.

Baseline
- [ ] Release build: capabilities file and CSP unchanged, only `tauri-plugin-updater` added, devtools off.
- [ ] Scheduled `fuzz.yml` run green; `audit` job green with the new crates.

## Out of scope

A publicly trusted code-signing certificate and SmartScreen reputation (U8 table, only if the user decides so
later), asking users to trust the self-signed certificate, third-party review and its preparation
(U14 → v1.0), silent or automatic background installs, delta updates, an update channel for pre-releases,
page-locking the decrypted payload, wiping Tauri / WebView IPC buffers, new features (TOTP, auto-type, custom-field
editing, password health).

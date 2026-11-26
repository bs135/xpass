# xPass

Zero-knowledge personal password manager for Windows, built with Tauri (Rust + React).

> Status: **v0.8 — TOTP, custom fields and password health**, the last feature release before 1.0. The vault
> format is frozen ([docs/FORMAT.md](docs/FORMAT.md)). See [docs/PLAN.md](docs/PLAN.md).

![xPass: entry list and an entry with its password history](docs/screenshots/main.png)

<p align="center">
  <img src="docs/screenshots/unlock.png" alt="xPass unlock screen with Windows Hello and the master password" width="500">
</p>

## Features (v0.8)

- **Encrypted local vault** — one file, `%APPDATA%\com.bs135.xpass\vault.xpv`, with the last 10 versions kept in
  `backups\`.
- **Windows Hello unlock** — face, fingerprint or PIN instead of typing the master password; the password stays
  available and is needed to turn Hello on.
- **Master password + Emergency Kit** — a printable recovery key, confirmed at setup, unlocks the vault if the
  password is forgotten.
- **Entries** — title, username, password, websites, tags, notes, favorites, custom fields (add, rename,
  reorder, mark as secret); search and keyboard shortcuts.
- **One-time passwords (TOTP)** — an entry shows the current code of its TOTP secret with a countdown and copies
  it; add one by pasting the `otpauth://` link or the key a site shows when you turn on two-factor
  authentication. Imported TOTP secrets (Bitwarden, KeePassXC) work as they are. See
  [One-time passwords](#one-time-passwords).
- **Password health** — the shield button in the entry list shows entries with a weak password or one used in
  several entries.
- **Password history** — the last 20 passwords of each entry, newest first, with reveal, copy and **Use this
  password**; labels show which ones came from a sync conflict, a restore or an import.
- **Import / export** — import CSV files from Bitwarden, Chrome / Edge, KeePassXC or any password manager, with a
  preview and duplicate detection; export to an encrypted KeePass (KDBX 4) file or a plaintext CSV. See
  [Import and export](#import-and-export).
- **Secure copy** — copied secrets are excluded from clipboard history (`Win+V`) and cloud clipboard and are cleared
  after a timeout (default 20 s) unless you copied something else meanwhile.
- **Password generator** — random characters or EFF passphrases, with an entropy estimate.
- **Auto-lock** — on Windows lock, when the screen turns off or the PC sleeps, after inactivity (default 5 min),
  optionally when minimized; tray icon with Open / Lock / Quit.
- **Security settings** — change the master password, create a new Emergency Kit, rotate the vault key.
- **GitHub sync** — one vault on several PCs through a file in your private GitHub repository. Offline-first: every
  change is saved locally first and synced when GitHub is reachable; concurrent edits merge per entry, and the older
  version of an entry edited on two PCs is kept in its history. See [Sync with GitHub](#sync-with-github).
- **Restore a previous version** — put the entries back as they were in one of the last 10 local versions; with
  sync on, the restore reaches your other PCs too.
- **Signed in-app update** — Settings → About finds a newer release, shows its notes and installs it after you
  confirm; xPass checks the update's signature first. Optional check at startup (off by default) only shows a
  badge; nothing is downloaded without a click. See [Update](#update).
- **Remove old versions from GitHub** — after rotating the vault key, replace the sync branch with a single commit,
  so the old encrypted copies are no longer in its history. See [Remove old versions](#remove-old-versions).
- English and Vietnamese UI, light/dark theme, self-drawn title bar that follows the theme.

## Security model (summary)

- A random **VaultKey** encrypts the vault (XChaCha20-Poly1305). It is stored only wrapped by key-encryption keys:
  - master password → **Argon2id** (128 MiB, t=3, p=4);
  - recovery key (256 bits) → HKDF;
  - Windows Hello (optional) → HKDF of a signature made by a key in the PC's TPM, which needs Hello consent.
- The whole file header is authenticated: any modified byte is detected and reported, never silently accepted.
- Wrong passwords are throttled (1, 2, 4 … 30 s).
- All crypto and file handling is in Rust (`crates/xpass-core`). The WebView never holds the VaultKey and receives a
  password only when you reveal it or open the edit form.
- Strict Content Security Policy (no remote content, no `eval`), minimal Tauri capabilities, no file-system or shell
  plugins.
- Secrets in memory are zeroized when the vault locks or the app exits, and the vault key sits in a memory page
  kept out of the pagefile (best effort; see the threat model in [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md)).
- Dependencies are checked in CI (`cargo deny`, `npm audit`) and the vault file parser is fuzzed every week.
- Windows Error Reporting is told to leave xPass out: no crash report or dump with the unlocked vault in it.
- Updates and installers are signed (update key, self-signed Authenticode certificate); each release has checksums,
  SBOMs and build provenance attestations. See [Check the download](#check-the-download).
- **GitHub sync** uploads only the encrypted vault file. The token is kept in Windows Credential Manager and used
  only by the Rust side for requests to `api.github.com`; the WebView has no network access at all.
- **Import and export** run in Rust: files are picked in the native Windows dialog, read, parsed and written in
  Rust, and the WebView only gets a preview without passwords or notes (import) or the file name (export). Every
  export asks for the master password or Windows Hello first. A plaintext CSV export is as sensitive as the vault
  itself: see [Import and export](#import-and-export).
- **Not protected against:** malware running as your Windows user, or a stolen backup file plus a weak master
  password. Older backups keep opening with the credentials that were valid when they were written. Full list of
  what is and is not covered: [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

## One-time passwords

A TOTP secret is stored as a secret custom field named `TOTP` (an `otpauth://totp/…` link). The code is computed
in the Rust side; the secret itself stays hidden unless you reveal it in the edit form.

Keeping the TOTP secret in the same vault as the password is convenient, but anyone who opens the vault then has
**both** factors of that account. For accounts where that matters, keep the TOTP in a separate authenticator app.

HOTP (counter-based) and Steam codes are not supported. A KDBX export also writes the secret as KeePassXC's `otp`
attribute.

## Sync with GitHub

xPass can keep one vault in sync across your Windows PCs through a single file in a **private** GitHub repository.
There is no xPass server: each PC talks to GitHub directly with a token that can reach only that repository.

### 1. Create a repository and a token

1. Create a **private** repository on GitHub for the vault (an empty one is fine).
2. Open GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens** → Generate new
   token.
3. Repository access: **Only select repositories**, and pick the vault repository.
4. Permissions: **Contents: Read and write** (Metadata: Read-only is added automatically). Nothing else.
5. Choose an expiration date and generate the token. Copy it straight into xPass; you do not need to keep it
   anywhere else.

### 2. Set up the first PC

Settings → **Sync**: enter the repository (`owner/name` or its URL), optionally a branch (default: the repository's
default branch) and the path (default `xpass/vault.xpv`), paste the token and choose **Test connection**. Save is
enabled after a passing test; a public repository needs an explicit confirmation. The first sync creates the file.

### 3. Add another PC

Install xPass and choose **Use my vault from GitHub** on the first screen: the same repository, branch and path, a
token (the same one or a new one), then your master password. The vault is downloaded and opened; your existing
Emergency Kit keeps working, and you can turn Windows Hello on for this PC.

### How it behaves

- Changes are saved on the PC first and synced 10 s after the last edit, every 5 min while unlocked, when you unlock,
  and when the vault locks or the app quits (8 s at most). **Sync now** is in the header, in Settings and in the
  tray menu. Without a connection everything keeps working; changes are synced later.
- Edits of different entries on different PCs are merged. If the same entry was edited on two PCs, the newer edit
  wins and the other one is kept in the entry's history.
- A new master password set on one PC reaches the others with the next sync (they show a notice once).
  Windows Hello is set up per PC and keeps working.
- **Rotating the vault key** with sync on needs a connection: the new file reaches GitHub first. The other PCs then
  ask for the master password once and keep their unsynced changes.
- The header icon shows the state. Problems that need you have a banner with the fix: an expired token (enter a new
  one in Settings → Sync), a vault key changed on another PC (enter the master password), or a different vault at
  the configured path (use another path, or replace this PC's vault with the one on GitHub; the current file is
  kept as a backup).

### What is stored where

| Where | What |
|---|---|
| GitHub repository | The encrypted vault file only. Each sync that changes something is one commit with the message `xpass: update vault`. |
| Windows Credential Manager | The token (`xpass/github-token`). It is never shown again, written to a file or logged. |
| `%APPDATA%\com.bs135.xpass\settings.json` | Repository, branch and path (no token). |
| `%APPDATA%\com.bs135.xpass\sync-state.json` | The last synced file versions (SHA) and the last sync time. |

Anyone who can read the repository can see **when** you synced (commit times), the **size** of the vault file and
its history, but not your entries: the file is encrypted with your vault key. Old versions stay in the git history,
so a leaked old file can be attacked with the master password that was valid then; use a strong master password
and keep the repository private.

### Remove old versions

After you rotate the vault key (Settings → Security), the older versions in the repository history still open with
the old master password or recovery key. Settings → Sync → **Remove old versions** (also offered right after a
rotation) replaces the sync branch with a single commit holding the current files:

- this PC must be in sync with GitHub, with no changes waiting; you confirm with the master password or Windows Hello;
- your other PCs keep syncing; if one of them pushed at that very moment, its change comes back with its next sync;
- other files in the same branch stay, but lose their history too (the dialog lists them);
- if branch protection or a ruleset blocks force pushes on that branch, GitHub refuses and nothing changes.

What it cannot do: GitHub may still serve an old version to someone who knows its commit ID for a while, and forks,
clones and caches keep their copies. Removing every trace needs GitHub Support.

### Turn sync off or remove the token

Settings → Sync → **Turn off sync** deletes the token from Credential Manager and the sync settings; the vault on
this PC stays. Also revoke the token on GitHub (Settings → Developer settings → Personal access tokens) when you
no longer need it. To replace an expired token, choose **Change** → **Replace token**.

## Import and export

Both are in Settings → **Import & export**.

### Import a CSV file

1. Export your passwords as CSV from the old password manager:
   - **Bitwarden**: Tools → Export vault → File format **.csv** (not the encrypted or JSON formats).
   - **Chrome / Edge**: Settings → Passwords (Password Manager) → Export passwords.
   - **KeePassXC**: Database → Export → CSV file.
   - **Other managers**: any CSV export with a header row.
2. In xPass choose **Choose CSV file…**. The format is detected from the header row and can be changed; for other
   CSV files, pick which column holds the title, username, password, website, notes, tags and favorite, and what
   happens to the other columns (leave out, add to notes, or keep as custom fields).
3. Check the preview. Rows already in the vault (same site, username and password) are skipped. Rows with the same
   site and username but another password are *possible duplicates*: skip them (default), import them as new
   entries, or update the existing entries (the old password stays in their history). Untick rows you do not want.
4. **Import**. The whole import is one change: with sync on it reaches your other PCs at once, and Settings →
   Restore a previous version undoes it.
5. **Delete the source CSV** on the result screen. The file is not encrypted.

What is imported: title, username, password, websites (all of them), notes, tags (Bitwarden folders, KeePassXC
groups), favorites, Bitwarden custom fields, TOTP secrets (as a secret custom field named `TOTP`; xPass does not
show TOTP codes yet), Bitwarden secure notes, and the creation date from KeePassXC. Not imported: Bitwarden cards and
identities (counted as left out), attachments, passkeys, and encrypted or JSON exports. Files must be UTF-8 (with or
without BOM), comma, semicolon or tab separated, at most 10 MB and 10 000 rows.

### Export

- **KeePass (KDBX)**: an encrypted KDBX 4 file (ChaCha20, Argon2id 64 MiB) protected by a new *export password*.
  KeePassXC and KeePass 2 open it. Entries go into one group `xPass`; extra websites are stored as `KP2A_URL_1`, …
  (KeePassXC shows them), favorites as the tag `favorite`, secret custom fields as protected fields, and password
  history as entry history.
- **CSV**: a **plaintext** file, readable by anyone who gets it. Do not open it in Excel or another spreadsheet app
  (they can change values), do not save it in a folder that syncs to the cloud, and delete it (and empty the Recycle
  Bin) when done. Its columns are `title, username, password, urls, notes, tags, favorite, created, updated,
  fields`; xPass imports it back with the **xPass** format.

Every export asks for the master password (or Windows Hello) and then where to save the file. Deleted entries are
not exported, and neither is password history in the CSV.

## Update

From v0.7.0, Settings → About → **Check for updates** shows a newer version with its notes. **Install and restart**
downloads the installer, checks its signature against the update key built into xPass (and that it comes from this
repository's release for that version), then locks the vault, uploads unsynced changes and runs the installer,
which restarts xPass. A file that fails a check is never run. Your vault and settings stay as they are.

- **From v0.6.x:** those versions cannot install updates; install v0.7.0 by hand once (see [Install](#install)).
  Later versions arrive in the app.
- **If the update key ever changes** (lost or leaked), xPass cannot accept updates signed with the new key: the
  release notes and this README will say so, and that version must be installed by hand once.

## Install

Download one installer from [Releases](https://github.com/bs135/xpass/releases):

- `xpass_<version>_windows_x64-setup.exe` (NSIS, recommended): installs for your Windows user only, without an
  administrator prompt, into `%LOCALAPPDATA%\xpass`. The first install asks for English or Vietnamese; later
  installs and updates reuse that choice.
- `xpass_<version>_windows_x64.msi`: for managed installs (Intune, Group Policy).

The WebView2 runtime is downloaded by the installer if Windows does not have it yet (it ships with Windows 11).
Installing a newer version over an older one keeps your vault and settings.

### Check the download

Each release (from v0.6.0) has `SHA256SUMS.txt`; compare the installer's hash with the line for its file name before
running it:

```powershell
(Get-FileHash .\xpass_0.7.0_windows_x64-setup.exe -Algorithm SHA256).Hash.ToLower()
```

From v0.7.0 the installers, `xpass.exe` and the uninstaller also carry an Authenticode signature from a
**self-signed** xPass certificate. Windows does not know that certificate, so it reports the signature as not
trusted; what matters is **which** certificate signed the file. Check that the thumbprint is exactly
`4D3E068AED69DE2C88578FC2CA46439E78E73D7D` (also in [docs/codesign/xpass-codesign.cer](docs/codesign/xpass-codesign.cer)
and in every release's notes):

```powershell
(Get-AuthenticodeSignature .\xpass_0.7.0_windows_x64-setup.exe).SignerCertificate.Thumbprint
```

The same works later on the installed app (`%LOCALAPPDATA%\xpass\xpass.exe`). Do **not** import the certificate
into "Trusted Root Certification Authorities" or "Trusted Publishers": that would make Windows trust anything signed
with its key, not just xPass. If the thumbprint ever changes, the release notes and this README say so first.

From v0.7.0 each installer also has a **build provenance attestation**: a statement signed through GitHub that the
file was built by this repository's Release workflow from a specific commit. With the
[GitHub CLI](https://cli.github.com/):

```powershell
gh attestation verify .\xpass_0.7.0_windows_x64-setup.exe --repo bs135/xpass
```

The release also lists what is inside as CycloneDX SBOMs (`…_sbom-rust.cdx.json` for the Rust crates,
`…_sbom-npm.cdx.json` for the bundled npm packages), attested for the same installers and covered by
`SHA256SUMS.txt`.

### Windows SmartScreen

Because the certificate is self-signed, SmartScreen may show "Windows protected your PC" (Unknown publisher) on the
first run. It means Windows does not know the publisher, not that the file is harmful. After checking the hash and
the thumbprint, choose **More info** → **Run anyway**. Download xPass only from the Releases page above. Later
versions can be installed from Settings → About (in-app update, signature checked by xPass itself).

### Uninstall

Settings → Apps → Installed apps → xPass → Uninstall. By default your data is **not** removed: the vault, its
backups and settings stay in `%APPDATA%\com.bs135.xpass`, and the GitHub token stays in Windows Credential Manager
(`xpass/github-token`) unless you turn sync off before uninstalling.

The uninstaller also offers a checkbox to delete the application data. Ticking it **deletes the vault and all local
backups** in `%APPDATA%\com.bs135.xpass`; only do that if the vault is synced to GitHub or you no longer need it.

## Development

Requirements: Windows 10/11, Node.js 24+, Rust stable (MSVC), Visual Studio Build Tools (Desktop C++), WebView2.

```bash
npm install
npm run tauri dev
```

Use a throwaway vault while developing (debug builds only):

```powershell
$env:XPASS_DATA_DIR = "$env:TEMP\xpass-dev"; $env:XPASS_IDLE_SECS = "30"; npm run tauri dev
```

Checks (same as CI):

```bash
npm run lint && npm test && npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Dependency audit (CI job `audit`, also every Monday): Rust advisories, licences and sources with
[cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (rules in `deny.toml`: permissive licences only, every
ignored advisory has a reason), and npm advisories at level high or above:

```bash
cargo install --locked cargo-deny   # once
cargo deny --workspace check
npm audit --audit-level=high
```

Fuzzing (every Sunday and on demand, not part of the PR checks): `crates/xpass-core/fuzz` has `cargo-fuzz` targets for the vault file parser
(`parse`), Hello unlock and sibling open (`unlock_hello`), recovery-key input (`recovery_decode`) and TOTP secrets
(`otp_parse`). Every CI run
replays the checked-in seeds and regressions on stable (`cargo test`). The **Fuzz** workflow runs each target for
15 minutes every Sunday, keeping its corpus between runs in the Actions cache (a crash makes the run red and uploads
the input); longer runs: Actions → Fuzz → Run workflow, or nightly locally:

```bash
rustup toolchain install nightly --profile minimal
cargo install --locked cargo-fuzz
cd crates/xpass-core
cargo +nightly fuzz run parse fuzz/corpus/parse fuzz/seeds/parse fuzz/regressions/parse -- -max_total_time=600
```

On Windows the AddressSanitizer runtime comes with Visual Studio: put the folder holding
`clang_rt.asan_dynamic-x86_64.dll` (`VC\Tools\MSVC\<version>\bin\Hostx64\x64`) on `PATH` first, otherwise the
target exits with `STATUS_DLL_NOT_FOUND`. A crash input lands in `fuzz/artifacts/<target>/`; after the fix, copy
it to `fuzz/regressions/<target>/` so the
replay test keeps it covered. Seeds are regenerated with
`cargo run -p xpass-core --example fuzz_seeds --features fuzzing`.

Release build: `npm run tauri build` (NSIS and MSI installers in `target/release/bundle/`).

### Releasing

The version is bumped in the release PR (`npm run release:prepare`, see [PLAN.md §7](docs/PLAN.md#7-workflow));
merging it into `main` runs the **Release** workflow, which tags, builds and publishes.

Update signing (v0.7): the in-app updater only installs files signed with the project's update key. The key pair
belongs to the repository owner and never enters the repository:

- Private key and its password: GitHub environment **`release`** (Settings → Environments) as
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, plus an offline backup kept by the owner.
  Only the release build job uses that environment; pull requests never see it.
- Public key: `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` (base64 of the minisign `.pub` file).

`src-tauri/tauri.release.conf.json` turns on `createUpdaterArtifacts` and Authenticode signing for the release
build only, so local builds and the manual workflow run need no key.

Code signing (v0.7): a self-signed certificate (`CN=bs135 (xPass code signing)`, RSA 3072, code signing only, valid
until 2031-11-25), also owned by the repository owner. The PFX and its password are the `release` environment secrets
`WINDOWS_CERTIFICATE` (base64) and `WINDOWS_CERTIFICATE_PASSWORD`; the public certificate is
`docs/codesign/xpass-codesign.cer` and its thumbprint is `bundle.windows.certificateThumbprint` in
`tauri.release.conf.json`. The release job imports the PFX into the runner's certificate store only for the build,
tauri signs (with a timestamp) before computing the updater signatures, and `scripts/verify-authenticode.ps1` checks
every installer and the exe files inside the NSIS one. The release is created as a draft and published only after
these checks and the updater check pass; if a check fails, fix the cause and re-run the failed jobs. A release gets a `.sig` per installer and `latest.json` (the manifest the
app reads); the `verify-updater` job then checks them against the committed public key with
`scripts/verify-updater-artifacts.sh`. To sign a local build with the real key, set the two variables in your own
shell (never in a file in the repository) and run
`npm run tauri build -- --config src-tauri/tauri.release.conf.json`.

If the private key is ever lost or leaked, the public key in the app cannot be changed by an update signed with
another key: publish a release with a new key pair and tell users to install it by hand once.

Layout:

```text
crates/xpass-core/   vault format, crypto, model, merge (no UI / OS code)
crates/xpass-interop/ CSV import / export and KDBX export (no UI / OS code)
src-tauri/           Tauri app: commands and Windows integration
src/                 React + TypeScript frontend
```

## Documentation

- [docs/IDEA.md](docs/IDEA.md) — concept review and design decisions
- [docs/PLAN.md](docs/PLAN.md) — implementation plan and milestones
- [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) — assets, attackers, accepted limits and the security self-review
- [docs/FORMAT.md](docs/FORMAT.md) — vault file format v1, payload schema 1 and the sync contract (frozen for 1.0)
- [SECURITY.md](SECURITY.md) — supported versions and how to report a vulnerability
- [docs/plan-v0.1.0.md](docs/plan-v0.1.0.md) — v0.1 tasks and manual test checklist
- [docs/plan-v0.2.0.md](docs/plan-v0.2.0.md) — v0.2 tasks (custom title bar) and manual test checklist
- [docs/plan-v0.3.0.md](docs/plan-v0.3.0.md) — v0.3 tasks (Windows Hello unlock) and manual test checklist
- [docs/plan-v0.4.0.md](docs/plan-v0.4.0.md) — v0.4 tasks (GitHub sync, restore) and the two-device test checklist
- [docs/plan-v0.5.0.md](docs/plan-v0.5.0.md) — v0.5 tasks (import / export, password history) and test checklist
- [docs/plan-v0.6.0.md](docs/plan-v0.6.0.md) — v0.6 tasks (hardening, update check, release process) and test checklist
- [docs/plan-v0.7.0.md](docs/plan-v0.7.0.md) — v0.7 tasks (signed updates, code signing, crash dumps, history rewrite, SBOM) and test checklist
- [docs/spikes/](docs/spikes) — feasibility spikes (Windows Hello, vault format, GitHub sync)
- [CHANGELOG.md](CHANGELOG.md)

## License

[MIT](LICENSE). The passphrase generator uses the [EFF large wordlist](https://www.eff.org/dice) (CC BY 3.0 US).
KDBX export uses the [`keepass`](https://crates.io/crates/keepass) crate (MIT).

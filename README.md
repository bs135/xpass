# xPass

Zero-knowledge personal password manager for Windows, built with Tauri (Rust + React).

> Status: **v0.4 — GitHub sync; the MVP is complete**. Import / export (v0.5) is next. See
> [docs/PLAN.md](docs/PLAN.md).

## Features (v0.4)

- **Encrypted local vault** — one file, `%APPDATA%\com.bs135.xpass\vault.xpv`, with the last 10 versions kept in
  `backups\`.
- **Windows Hello unlock** — face, fingerprint or PIN instead of typing the master password; the password stays
  available and is needed to turn Hello on.
- **Master password + Emergency Kit** — a printable recovery key, confirmed at setup, unlocks the vault if the
  password is forgotten.
- **Entries** — title, username, password, websites, tags, notes, favorites; search and keyboard shortcuts.
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
- Secrets in memory are zeroized when the vault locks or the app exits (best effort; see the threat model in
  [docs/IDEA.md](docs/IDEA.md)).
- **GitHub sync** uploads only the encrypted vault file. The token is kept in Windows Credential Manager and used
  only by the Rust side for requests to `api.github.com`; the WebView has no network access at all.
- **Not protected against:** malware running as your Windows user, or a stolen backup file plus a weak master
  password. Older backups keep opening with the credentials that were valid when they were written.

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

### Turn sync off or remove the token

Settings → Sync → **Turn off sync** deletes the token from Credential Manager and the sync settings; the vault on
this PC stays. Also revoke the token on GitHub (Settings → Developer settings → Personal access tokens) when you
no longer need it. To replace an expired token, choose **Change** → **Replace token**.

## Install

Download the installer (`.exe` or `.msi`) from [Releases](https://github.com/bs135/xpass/releases). Builds are not
code-signed yet, so Windows SmartScreen may warn on first start ("More info" → "Run anyway").

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

Release build: `npm run tauri build` (NSIS and MSI installers in `target/release/bundle/`).

Layout:

```text
crates/xpass-core/   vault format, crypto, model, merge (no UI / OS code)
src-tauri/           Tauri app: commands and Windows integration
src/                 React + TypeScript frontend
spikes/              feasibility spikes (reference only)
```

## Documentation

- [docs/IDEA.md](docs/IDEA.md) — concept review and design decisions
- [docs/PLAN.md](docs/PLAN.md) — implementation plan and milestones
- [docs/plan-v0.1.0.md](docs/plan-v0.1.0.md) — v0.1 tasks and manual test checklist
- [docs/plan-v0.2.0.md](docs/plan-v0.2.0.md) — v0.2 tasks (custom title bar) and manual test checklist
- [docs/plan-v0.3.0.md](docs/plan-v0.3.0.md) — v0.3 tasks (Windows Hello unlock) and manual test checklist
- [docs/plan-v0.4.0.md](docs/plan-v0.4.0.md) — v0.4 tasks (GitHub sync, restore) and the two-device test checklist
- [docs/spikes/](docs/spikes) — feasibility spikes (Windows Hello, vault format, GitHub sync)
- [CHANGELOG.md](CHANGELOG.md)

## License

[MIT](LICENSE). The passphrase generator uses the [EFF large wordlist](https://www.eff.org/dice) (CC BY 3.0 US).

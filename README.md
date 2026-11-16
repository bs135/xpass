# xPass

Zero-knowledge personal password manager for Windows, built with Tauri (Rust + React).

> Status: **v0.1 — local vault**. Custom title bar (v0.2), Windows Hello (v0.3) and GitHub sync (v0.4) are next. See [docs/PLAN.md](docs/PLAN.md).

## Features (v0.1)

- **Encrypted local vault** — one file, `%APPDATA%\com.bs135.xpass\vault.xpv`, with the last 10 versions kept in
  `backups\`.
- **Master password + Emergency Kit** — a printable recovery key, confirmed at setup, unlocks the vault if the
  password is forgotten.
- **Entries** — title, username, password, websites, tags, notes, favorites; search and keyboard shortcuts.
- **Secure copy** — copied secrets are excluded from clipboard history (`Win+V`) and cloud clipboard and are cleared
  after a timeout (default 20 s) unless you copied something else meanwhile.
- **Password generator** — random characters or EFF passphrases, with an entropy estimate.
- **Auto-lock** — on Windows lock, when the screen turns off or the PC sleeps, after inactivity (default 5 min),
  optionally when minimized; tray icon with Open / Lock / Quit.
- **Security settings** — change the master password, create a new Emergency Kit, rotate the vault key.
- English and Vietnamese UI, light/dark theme.

## Security model (summary)

- A random **VaultKey** encrypts the vault (XChaCha20-Poly1305). It is stored only wrapped by key-encryption keys:
  - master password → **Argon2id** (128 MiB, t=3, p=4);
  - recovery key (256 bits) → HKDF.
- The whole file header is authenticated: any modified byte is detected and reported, never silently accepted.
- Wrong passwords are throttled (1, 2, 4 … 30 s).
- All crypto and file handling is in Rust (`crates/xpass-core`). The WebView never holds the VaultKey and receives a
  password only when you reveal it or open the edit form.
- Strict Content Security Policy (no remote content, no `eval`), minimal Tauri capabilities, no file-system or shell
  plugins.
- Secrets in memory are zeroized when the vault locks or the app exits (best effort; see the threat model in
  [docs/IDEA.md](docs/IDEA.md)).
- **Not protected against:** malware running as your Windows user, or a stolen backup file plus a weak master
  password. Older backups keep opening with the credentials that were valid when they were written.

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
- [docs/spikes/](docs/spikes) — feasibility spikes (Windows Hello, vault format, GitHub sync)
- [CHANGELOG.md](CHANGELOG.md)

## License

[MIT](LICENSE). The passphrase generator uses the [EFF large wordlist](https://www.eff.org/dice) (CC BY 3.0 US).

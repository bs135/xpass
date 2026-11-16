# xpass

Zero-knowledge personal password manager for Windows, built with Tauri (Rust + React). The vault is encrypted
locally and synced through a private GitHub repository.

> Status: in development (v0.1 — local vault). See [docs/PLAN.md](docs/PLAN.md).

## Documentation

- [docs/IDEA.md](docs/IDEA.md) — concept review and design decisions
- [docs/PLAN.md](docs/PLAN.md) — implementation plan and milestones
- [docs/plan-v0.1.0.md](docs/plan-v0.1.0.md) — current milestone tasks
- [docs/spikes/](docs/spikes) — feasibility spikes (Windows Hello, vault format, GitHub sync)

## Development

Requirements: Windows 10/11, Node.js 24+, Rust stable (MSVC), Visual Studio Build Tools (Desktop C++), WebView2.

```bash
npm install
npm run tauri dev
```

Checks (same as CI):

```bash
npm run lint && npm test && npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Layout:

```text
crates/xpass-core/   vault format, crypto, model, merge, sync engine (no UI / OS code)
src-tauri/           Tauri app: commands and Windows integration
src/                 React + TypeScript frontend
spikes/              feasibility spikes (reference only)
```

## License

[MIT](LICENSE)

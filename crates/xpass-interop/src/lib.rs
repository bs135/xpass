//! xpass import / export (v0.5, `docs/plan-v0.5.0.md`).
//!
//! Pure Rust, no Tauri or OS code: the app reads and writes files and calls into this
//! crate, so every format detail is unit-tested on any platform. Kept out of
//! `xpass-core` so the security-critical core does not depend on CSV/KDBX libraries.
//!
//! Import pipeline (all in Rust; the WebView only gets a preview without secrets):
//!
//! 1. [`decode::decode`]: bytes → [`decode::Table`] (UTF-8 check, delimiter sniffing,
//!    size and row limits).
//! 2. [`preset::detect`]: header row → [`preset::Preset`] (Bitwarden, Chrome, KeePassXC,
//!    xPass, Generic); [`preset::guess_mapping`] for generic files.
//! 3. [`convert::convert`]: table + preset (+ mapping) → [`convert::ImportRow`]s.
//! 4. [`dedup::classify`]: duplicate status of each row against the vault and earlier rows.
//! 5. [`apply::apply_import`]: adds / updates entries in a payload with one timestamp.
//!
//! Export: [`csv_export::export_csv`] writes the xPass CSV layout (imported back with
//! [`preset::Preset::XPass`]).
//!
//! Known limit: the `csv` crate's internal buffers and `String`s created while parsing
//! or writing are not zeroized; callers drop them as soon as possible.

pub mod apply;
pub mod convert;
pub mod csv_export;
pub mod decode;
pub mod dedup;
pub mod error;
pub mod preset;
pub mod text;
pub mod time;

pub use error::ImportError;

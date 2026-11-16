//! xpass core: vault file format, crypto, data model and merge (the sync engine comes in v0.3).
//!
//! This crate has no UI or OS dependencies so the security-critical code can be
//! unit-tested on any platform. Layout and rationale: `docs/spikes/vault-format.md`.
//!
//! - [`format`]: binary envelope (header + key slots + ciphertext), hand-written parser.
//! - [`crypto`]: Argon2id, HKDF and XChaCha20-Poly1305 primitives.
//! - [`recovery`]: printable recovery key with checksum.
//! - [`model`]: decrypted payload (entries with merge-ready metadata).
//! - [`vault`]: high-level operations (create, unlock, save, change password, rotate).
//! - [`merge`]: per-entry LWW merge with conflict history (D1).
//! - [`clock`]: HLC-lite timestamps (D2).
//! - [`maintenance`]: tombstone purge (D3).

pub mod clock;
pub mod crypto;
pub mod error;
pub mod format;
pub mod maintenance;
pub mod merge;
pub mod model;
pub mod recovery;
pub mod vault;

pub use error::{Error, Result};
pub use vault::{Unlock, UnlockedVault};

/// Current vault file format version.
pub const FORMAT_VERSION: u16 = format::VERSION;

//! xpass core: vault file format, crypto and data model (merge and sync come later).
//!
//! This crate has no UI or OS dependencies so the security-critical code can be
//! unit-tested on any platform. Layout and rationale: `docs/spikes/vault-format.md`.
//!
//! - [`format`]: binary envelope (header + key slots + ciphertext), hand-written parser.
//! - [`crypto`]: Argon2id, HKDF and XChaCha20-Poly1305 primitives.
//! - [`recovery`]: printable recovery key with checksum.
//! - [`model`]: decrypted payload (entries with merge-ready metadata).
//! - [`vault`]: high-level operations (create, unlock, save, change password, rotate).

pub mod crypto;
pub mod error;
pub mod format;
pub mod model;
pub mod recovery;
pub mod vault;

pub use error::{Error, Result};
pub use vault::{Unlock, UnlockedVault};

/// Current vault file format version.
pub const FORMAT_VERSION: u16 = format::VERSION;

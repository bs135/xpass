//! Spike: xpass vault file format v1.
//!
//! Layout and rationale are documented in `docs/spikes/vault-format.md`.
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
pub use vault::{UnlockedVault, Unlock};

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Error {
    /// File is not an xpass vault or is truncated / malformed.
    #[error("invalid vault file: {0}")]
    Format(&'static str),
    /// Format version newer than this build understands.
    #[error("unsupported vault format version {0}")]
    UnsupportedVersion(u16),
    /// Wrong password / recovery key / Hello KEK, or the slot was tampered with.
    #[error("wrong credentials")]
    BadCredentials,
    /// Payload or header was modified (AEAD tag mismatch).
    #[error("vault data was modified or corrupted")]
    Tampered,
    /// No key slot of the requested kind.
    #[error("no key slot of this kind")]
    NoSuchSlot,
    /// Recovery key has a typo (checksum mismatch) or bad encoding.
    #[error("invalid recovery key")]
    BadRecoveryKey,
    /// KDF parameters outside accepted bounds.
    #[error("KDF parameters out of bounds")]
    BadKdfParams,
    /// Payload JSON could not be (de)serialized. The message never contains payload data.
    #[error("payload error: {0}")]
    Payload(String),
    /// The other file belongs to a different vault (vault_id mismatch).
    #[error("file belongs to a different vault")]
    DifferentVault,
    /// The other file's key slots differ and our VaultKey does not decrypt it:
    /// the VaultKey was rotated on another device. Unlock with credentials instead.
    #[error("vault key was rotated on another device")]
    KeyChanged,
}

pub type Result<T> = std::result::Result<T, Error>;

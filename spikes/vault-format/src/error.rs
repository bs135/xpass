use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// File is not an xpass vault or is truncated / malformed.
    Format(&'static str),
    /// Format version newer than this build understands.
    UnsupportedVersion(u16),
    /// Wrong password / recovery key / Hello KEK, or the slot was tampered with.
    BadCredentials,
    /// Payload or header was modified (AEAD tag mismatch).
    Tampered,
    /// No key slot of the requested kind.
    NoSuchSlot,
    /// Recovery key has a typo (checksum mismatch) or bad encoding.
    BadRecoveryKey,
    /// KDF parameters outside accepted bounds.
    BadKdfParams,
    /// Payload JSON could not be (de)serialized.
    Payload(String),
    /// The other file belongs to a different vault (vault_id mismatch).
    DifferentVault,
    /// The other file's key slots differ and our VaultKey does not decrypt it:
    /// the VaultKey was rotated on another device. Unlock with credentials instead.
    KeyChanged,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format(m) => write!(f, "invalid vault file: {m}"),
            Error::UnsupportedVersion(v) => write!(f, "unsupported vault format version {v}"),
            Error::BadCredentials => write!(f, "wrong credentials"),
            Error::Tampered => write!(f, "vault data was modified or corrupted"),
            Error::NoSuchSlot => write!(f, "no key slot of this kind"),
            Error::BadRecoveryKey => write!(f, "invalid recovery key"),
            Error::BadKdfParams => write!(f, "KDF parameters out of bounds"),
            Error::Payload(m) => write!(f, "payload error: {m}"),
            Error::DifferentVault => write!(f, "file belongs to a different vault"),
            Error::KeyChanged => write!(f, "vault key was rotated on another device"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

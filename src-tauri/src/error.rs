//! Errors returned to the frontend.
//!
//! Serialized as `{ "code": "<snake_case>", ...fields }` so the UI can map stable codes
//! to translated messages. Messages and fields never contain secret data or file paths.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum AppError {
    #[error("no vault has been created yet")]
    NoVault,
    #[error("a vault already exists")]
    VaultExists,
    #[error("the vault is locked")]
    Locked,
    #[error("entry not found")]
    NotFound,
    #[error("could not open the link")]
    OpenFailed,
    #[error("the clipboard is not available")]
    Clipboard,
    #[error("the password is too weak")]
    WeakPassword,
    #[error("invalid input: {field}")]
    InvalidInput { field: &'static str },
    #[error("wrong credentials")]
    BadCredentials,
    #[error("invalid recovery key")]
    BadRecoveryKey,
    #[error("too many failed attempts, retry in {retry_after_ms} ms")]
    Throttled { retry_after_ms: u64 },
    #[error("the vault file was modified or is corrupted")]
    Tampered,
    #[error("not a valid vault file")]
    InvalidFile,
    #[error("unsupported vault format version {version}")]
    UnsupportedVersion { version: u16 },
    #[error("the vault has no key slot of this kind")]
    NoSuchSlot,
    #[error("the file belongs to a different vault")]
    DifferentVault,
    #[error("the vault key was rotated on another device")]
    KeyChanged,
    #[error("file system error: {kind}")]
    Io { kind: String },
    #[error("internal error")]
    Internal,
}

pub type AppResult<T> = Result<T, AppError>;

impl From<xpass_core::Error> for AppError {
    fn from(e: xpass_core::Error) -> Self {
        use xpass_core::Error as E;
        match e {
            E::BadCredentials => Self::BadCredentials,
            E::BadRecoveryKey => Self::BadRecoveryKey,
            E::Tampered => Self::Tampered,
            // Out-of-bounds KDF params can only come from a damaged or hostile file.
            E::Format(_) | E::BadKdfParams | E::Payload(_) => Self::InvalidFile,
            E::UnsupportedVersion(version) => Self::UnsupportedVersion { version },
            E::NoSuchSlot => Self::NoSuchSlot,
            E::DifferentVault => Self::DifferentVault,
            E::KeyChanged => Self::KeyChanged,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io {
            kind: e.kind().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_to_stable_codes() {
        assert_eq!(
            serde_json::to_value(AppError::BadCredentials).unwrap(),
            json!({"code": "bad_credentials"})
        );
        assert_eq!(
            serde_json::to_value(AppError::Throttled { retry_after_ms: 1000 }).unwrap(),
            json!({"code": "throttled", "retryAfterMs": 1000})
        );
        assert_eq!(
            serde_json::to_value(AppError::UnsupportedVersion { version: 2 }).unwrap(),
            json!({"code": "unsupported_version", "version": 2})
        );
    }

    #[test]
    fn core_errors_map_without_details() {
        assert_eq!(
            AppError::from(xpass_core::Error::Format("bad magic")),
            AppError::InvalidFile
        );
        assert_eq!(
            AppError::from(xpass_core::Error::Payload("secret?".into())),
            AppError::InvalidFile
        );
    }
}

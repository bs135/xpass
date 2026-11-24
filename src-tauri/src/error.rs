//! Errors returned to the frontend.
//!
//! Serialized as `{ "code": "<snake_case>", ...fields }` so the UI can map stable codes
//! to translated messages. Messages and fields never contain secret data or file paths.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Error, Serialize, PartialEq, Eq)]
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
    #[error("Windows Hello is not available on this PC")]
    HelloUnavailable,
    #[error("the Windows Hello key for this vault no longer exists or has changed")]
    HelloNotFound,
    #[error("Windows Hello was cancelled")]
    HelloCanceled,
    #[error("Windows Hello is locked after too many attempts")]
    HelloDeviceLocked,
    #[error("Windows Hello is already waiting for the user")]
    HelloBusy,
    #[error("Windows Hello failed")]
    HelloFailed,
    #[error("Windows Credential Manager is not available")]
    SecretStore,
    #[error("sync is not set up")]
    SyncNotConfigured,
    #[error("GitHub is not reachable")]
    SyncOffline,
    #[error("the GitHub token is invalid or expired")]
    SyncUnauthorized,
    #[error("the GitHub token lacks a permission")]
    SyncForbidden,
    #[error("GitHub rate limit reached")]
    SyncRateLimited { retry_after_s: Option<u64> },
    #[error("repository not found or not shared with the token")]
    SyncRepoNotFound,
    #[error("sync failed")]
    SyncFailed,
    #[error("no vault at this path on GitHub")]
    SyncRemoteMissing,
    #[error("this action needs a successful sync first")]
    SyncRequired,
    #[error("the import file is too large")]
    ImportTooLarge { max_mb: usize, max_rows: usize },
    #[error("the import file is not a valid CSV file")]
    ImportInvalidCsv { line: Option<u64> },
    #[error("the import file is not UTF-8 text")]
    ImportEncoding,
    #[error("the import file has no data rows")]
    ImportEmpty,
    #[error("the import was cancelled or timed out")]
    ImportExpired,
    #[error("the file dialog could not be shown")]
    DialogFailed,
    #[error("the update check failed")]
    UpdateFailed,
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

impl From<xpass_core::sync::StoreError> for AppError {
    fn from(e: xpass_core::sync::StoreError) -> Self {
        use xpass_core::sync::StoreError as E;
        match e {
            E::Offline => Self::SyncOffline,
            E::Unauthorized => Self::SyncUnauthorized,
            E::Forbidden => Self::SyncForbidden,
            E::RateLimited { retry_after_s } => Self::SyncRateLimited { retry_after_s },
            E::RepoNotFound => Self::SyncRepoNotFound,
            E::Conflict | E::InvalidResponse | E::Http(_) => Self::SyncFailed,
        }
    }
}

impl From<xpass_interop::ImportError> for AppError {
    fn from(e: xpass_interop::ImportError) -> Self {
        use xpass_interop::{
            decode::{MAX_FILE_BYTES, MAX_ROWS},
            ImportError as E,
        };
        match e {
            E::TooLarge { .. } | E::TooManyRows { .. } => Self::ImportTooLarge {
                max_mb: MAX_FILE_BYTES / (1024 * 1024),
                max_rows: MAX_ROWS,
            },
            E::Encoding => Self::ImportEncoding,
            E::InvalidCsv { line } => Self::ImportInvalidCsv { line },
            E::Empty => Self::ImportEmpty,
            E::NoSuchColumn { .. } | E::NothingMapped => Self::InvalidInput { field: "mapping" },
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
    fn store_errors_map_to_sync_codes() {
        use xpass_core::sync::StoreError;
        assert_eq!(AppError::from(StoreError::Unauthorized), AppError::SyncUnauthorized);
        assert_eq!(
            serde_json::to_value(AppError::from(StoreError::RateLimited { retry_after_s: Some(9) })).unwrap(),
            json!({"code": "sync_rate_limited", "retryAfterS": 9})
        );
        assert_eq!(AppError::from(StoreError::Http(502)), AppError::SyncFailed);
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

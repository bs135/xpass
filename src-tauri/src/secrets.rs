//! The GitHub token (fine-grained PAT) in Windows Credential Manager (S11).
//!
//! The token is typed once in Settings, validated, stored here and never sent back
//! to the WebView, written to `settings.json`, logged or put into an error. Rust
//! reads it only to build the `Authorization` header of a GitHub request.

use std::sync::{Mutex, PoisonError};

use zeroize::Zeroizing;

use crate::{
    error::{AppError, AppResult},
    paths::Paths,
};

/// Credential Manager target (generic credential) of the release app.
pub const TOKEN_TARGET: &str = "xpass/github-token";

/// Where the token lives. The app uses the Credential Manager; tests use memory.
pub trait TokenStore: Send + Sync {
    fn get(&self) -> AppResult<Option<Zeroizing<String>>>;
    fn set(&self, token: &str) -> AppResult<()>;
    /// Removing a token that does not exist is not an error.
    fn delete(&self) -> AppResult<()>;
}

/// Trims and checks a token typed by the user. Accepts fine-grained (`github_pat_…`)
/// and classic (`ghp_…`) tokens without depending on the prefix: printable ASCII,
/// no whitespace, 20–255 characters.
pub fn normalize_token(input: &str) -> AppResult<Zeroizing<String>> {
    let token = input.trim();
    let ok = (20..=255).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_graphic());
    if ok {
        Ok(Zeroizing::new(token.to_owned()))
    } else {
        Err(AppError::InvalidInput { field: "token" })
    }
}

/// Target name for this data directory. Debug builds pointed at a throwaway
/// directory (`XPASS_DATA_DIR`) get their own credential, so dev vaults never touch
/// the real token.
pub fn target_for(paths: &Paths, data_dir_overridden: bool) -> String {
    if !data_dir_overridden {
        return TOKEN_TARGET.to_owned();
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    paths.dir().hash(&mut h);
    format!("{TOKEN_TARGET}/dev-{:016x}", h.finish())
}

/// Windows Credential Manager via `keyring` (generic credential, current user).
pub struct CredentialManager {
    target: String,
}

impl CredentialManager {
    pub fn new(target: String) -> Self {
        Self { target }
    }

    fn entry(&self) -> AppResult<keyring::Entry> {
        keyring::Entry::new_with_target(&self.target, "xpass", "github-token").map_err(|_| AppError::SecretStore)
    }
}

impl TokenStore for CredentialManager {
    fn get(&self) -> AppResult<Option<Zeroizing<String>>> {
        match self.entry()?.get_password() {
            Ok(token) => Ok(Some(Zeroizing::new(token))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(AppError::SecretStore),
        }
    }

    fn set(&self, token: &str) -> AppResult<()> {
        self.entry()?.set_password(token).map_err(|_| AppError::SecretStore)
    }

    fn delete(&self) -> AppResult<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(AppError::SecretStore),
        }
    }
}

/// In-memory store (tests, and the default until the app installs the real one).
#[derive(Default)]
pub struct MemoryTokens(Mutex<Option<Zeroizing<String>>>);

impl TokenStore for MemoryTokens {
    fn get(&self) -> AppResult<Option<Zeroizing<String>>> {
        Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).clone())
    }

    fn set(&self, token: &str) -> AppResult<()> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Zeroizing::new(token.to_owned()));
        Ok(())
    }

    fn delete(&self) -> AppResult<()> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_trimmed_and_checked() {
        let t = normalize_token("  github_pat_11AAAAAAA0abcdefghijklmnop  \n").unwrap();
        assert_eq!(t.as_str(), "github_pat_11AAAAAAA0abcdefghijklmnop");
        assert!(
            normalize_token("ghp_0123456789abcdefghij").is_ok(),
            "classic tokens too"
        );
        for bad in [
            "",
            "short",
            "has space inside the token value",
            "ghp_ünïcode_0123456789abc",
        ] {
            assert_eq!(
                normalize_token(bad).unwrap_err(),
                AppError::InvalidInput { field: "token" },
                "{bad:?}"
            );
        }
        assert!(normalize_token(&"x".repeat(256)).is_err());
    }

    #[test]
    fn dev_data_dirs_get_their_own_target() {
        let a = Paths::new("C:/tmp/xpass-a");
        let b = Paths::new("C:/tmp/xpass-b");
        assert_eq!(target_for(&a, false), TOKEN_TARGET);
        assert_ne!(target_for(&a, true), TOKEN_TARGET);
        assert_ne!(target_for(&a, true), target_for(&b, true));
        assert!(target_for(&a, true).starts_with("xpass/github-token/dev-"));
    }

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryTokens::default();
        assert_eq!(s.get().unwrap(), None);
        s.set("abc").unwrap();
        assert_eq!(s.get().unwrap().as_deref().map(String::as_str), Some("abc"));
        s.delete().unwrap();
        s.delete().unwrap();
        assert_eq!(s.get().unwrap(), None);
    }
}

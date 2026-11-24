//! Re-authentication before a sensitive action (v0.5 plan I7: every export asks).
//!
//! Password: checked against the password slot with the unlock throttle. Windows
//! Hello: the vault's Hello challenge is signed (Hello dialog) and the signature must
//! open the Hello slot; it is dropped right after. A session unlocked with the
//! recovery key follows the `authorize` rule and is not asked again.

use serde::Deserialize;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::*;

/// How the user confirms a sensitive action.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum Auth {
    Password { password: String },
    Hello,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password { .. } => f.write_str("Auth::Password"),
            Self::Hello => f.write_str("Auth::Hello"),
        }
    }
}

impl AppState {
    /// Confirms the user before a sensitive action. Blocks on the Hello dialog
    /// (call from a blocking thread) without holding the session lock.
    pub fn reauthorize(&self, auth: &Auth) -> AppResult<()> {
        match auth {
            Auth::Password { password } => {
                let guard = lock(&self.0.session);
                let session = guard.as_ref().ok_or(AppError::Locked)?;
                self.authorize(session, Some(password))
            }
            Auth::Hello => self.reauthorize_hello(),
        }
    }

    fn reauthorize_hello(&self) -> AppResult<()> {
        let _busy = self.0.hello_busy.try_lock().map_err(|_| AppError::HelloBusy)?;
        let (vault_id, challenge) = {
            let guard = lock(&self.0.session);
            let session = guard.as_ref().ok_or(AppError::Locked)?;
            if session.recovered() {
                return Ok(());
            }
            let vault = session.vault();
            (vault.vault_id(), vault.hello_challenge()?)
        };
        let signature = match self.0.hello.sign(&hello::credential_name(vault_id), &challenge) {
            Ok(signature) => signature,
            Err(HelloError::NotFound) => {
                lock(&self.0.hello_flags).broken = true;
                return Err(AppError::HelloNotFound);
            }
            Err(e) => return Err(e.into()),
        };

        let guard = lock(&self.0.session);
        // Locked (or another vault opened) while the dialog was open.
        let session = guard
            .as_ref()
            .filter(|s| s.vault().vault_id() == vault_id)
            .ok_or(AppError::Locked)?;
        match session.vault().check_hello(&signature) {
            Ok(()) => Ok(()),
            // The credential was re-created: the slot is dead, like at unlock.
            Err(xpass_core::Error::BadCredentials) => {
                lock(&self.0.hello_flags).broken = true;
                Err(AppError::HelloNotFound)
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hello::fake::FakeHello, paths::Paths};

    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    const PW: &str = "correct horse battery staple";

    fn unlocked() -> (tempfile::TempDir, AppState, FakeHello, String) {
        let tmp = tempfile::tempdir().unwrap();
        let fake = FakeHello::default();
        let clipboard = SecureClipboard::new(crate::clipboard::fake::FakeClipboard::default());
        let state = AppState::new(
            Paths::new(tmp.path().join("data")),
            TEST_KDF,
            clipboard,
            Box::new(fake.clone()),
        );
        let rec = state.create_vault(PW).unwrap().to_string();
        (tmp, state, fake, rec)
    }

    fn password(pw: &str) -> Auth {
        Auth::Password { password: pw.into() }
    }

    #[test]
    fn password_reauth_is_checked_and_throttled() {
        let (_tmp, state, _, _) = unlocked();
        state.reauthorize(&password(PW)).unwrap();
        assert_eq!(
            state.reauthorize(&password("nope")).unwrap_err(),
            AppError::BadCredentials
        );
        assert_eq!(
            state.reauthorize(&password("nope")).unwrap_err(),
            AppError::BadCredentials
        );
        // Failures count like wrong unlocks: after the second one a retry waits.
        assert!(matches!(
            state.reauthorize(&password(PW)).unwrap_err(),
            AppError::Throttled { .. }
        ));
    }

    #[test]
    fn reauth_needs_an_unlocked_vault() {
        let (_tmp, state, _, _) = unlocked();
        state.lock();
        assert_eq!(state.reauthorize(&password(PW)).unwrap_err(), AppError::Locked);
        assert_eq!(state.reauthorize(&Auth::Hello).unwrap_err(), AppError::Locked);
    }

    #[test]
    fn hello_reauth() {
        let (_tmp, state, fake, _) = unlocked();
        assert_eq!(state.reauthorize(&Auth::Hello).unwrap_err(), AppError::NoSuchSlot);

        state.enable_hello(Some(PW)).unwrap();
        let prompts = fake.prompts();
        state.reauthorize(&Auth::Hello).unwrap();
        assert_eq!(fake.prompts(), prompts + 1, "every re-auth shows the Hello dialog");

        fake.fail_next(HelloError::Canceled);
        assert_eq!(state.reauthorize(&Auth::Hello).unwrap_err(), AppError::HelloCanceled);
        assert!(!state.hello_info().broken);

        let name = hello::credential_name(state.local_vault_id().unwrap());
        fake.recreate(&name);
        assert_eq!(state.reauthorize(&Auth::Hello).unwrap_err(), AppError::HelloNotFound);
        assert!(state.hello_info().broken);
    }

    #[test]
    fn recovery_session_is_not_asked_again() {
        let (_tmp, state, _, rec) = unlocked();
        state.lock();
        state.unlock_recovery(&rec).unwrap();
        state.reauthorize(&password("anything")).unwrap();
        state.reauthorize(&Auth::Hello).unwrap();
    }

    #[test]
    fn auth_debug_hides_the_password() {
        assert_eq!(format!("{:?}", password(PW)), "Auth::Password");
        let parsed: Auth = serde_json::from_str(r#"{"method":"password","password":"x"}"#).unwrap();
        assert!(matches!(parsed, Auth::Password { .. }));
        let parsed: Auth = serde_json::from_str(r#"{"method":"hello"}"#).unwrap();
        assert!(matches!(parsed, Auth::Hello));
    }
}

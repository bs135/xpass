//! Windows Hello as a key source (ported from the spike, `docs/spikes/windows-hello.md`).
//!
//! A TPM-backed Hello credential (`KeyCredentialManager`, RSA-2048, PKCS#1 v1.5)
//! signs a challenge bound to the vault id and the Hello slot salt. Signatures are
//! deterministic, so `xpass-core` turns the signature into a stable KEK with HKDF.
//! The signature never leaves Rust; callers drop it right after unlocking or
//! wrapping.
//!
//! Every call that can show the Hello dialog blocks until the user answers, so it
//! must run on a blocking thread (`spawn_blocking`), never on the UI or IPC thread.
//! While it waits, a watcher thread brings the dialog to the foreground: desktop
//! apps often get it behind their own window. `KeyCredentialManager` has no
//! HWND-owned interop (only `UserConsentVerifier` does, and it gives no key), so
//! the watcher is the only option.

use zeroize::Zeroizing;

use crate::error::AppError;

/// Why a Hello operation did not produce a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloError {
    /// Windows Hello is not set up or not supported on this PC.
    Unavailable,
    /// The credential does not exist (Hello was reset, the PIN removed, or the
    /// credential deleted). The Hello KEK is lost for good.
    NotFound,
    /// The user cancelled the dialog or chose to use a password.
    Canceled,
    /// Too many failed attempts; Windows locked Hello for now.
    DeviceLocked,
    /// Any other failure.
    Failed,
}

impl From<HelloError> for AppError {
    fn from(e: HelloError) -> Self {
        match e {
            HelloError::Unavailable => AppError::HelloUnavailable,
            HelloError::NotFound => AppError::HelloNotFound,
            HelloError::Canceled => AppError::HelloCanceled,
            HelloError::DeviceLocked => AppError::HelloDeviceLocked,
            HelloError::Failed => AppError::HelloFailed,
        }
    }
}

pub type HelloResult<T> = Result<T, HelloError>;

/// Hello operations, abstracted for tests. Calls may block on user verification.
pub trait HelloBackend: Send + Sync + 'static {
    /// Whether Windows Hello is set up and usable (no prompt).
    fn is_supported(&self) -> bool;
    /// Opens the credential `name`, creating it if missing (Hello prompt on create).
    fn open_or_create(&self, name: &str) -> HelloResult<()>;
    /// Signs `data` with the existing credential `name` (Hello prompt).
    fn sign(&self, name: &str, data: &[u8]) -> HelloResult<Zeroizing<Vec<u8>>>;
    /// Deletes the credential `name`. Deleting a missing credential is not an error.
    fn delete(&self, name: &str) -> HelloResult<()>;
}

/// Credential name for a vault. Unpackaged apps share the credential namespace per
/// user, so the name is specific to the vault (it survives a VaultKey rotation).
pub fn credential_name(vault_id: uuid::Uuid) -> String {
    format!("xpass-{vault_id}")
}

/// The real Windows Hello backend (a stub that reports "unavailable" elsewhere).
pub fn system() -> Box<dyn HelloBackend> {
    #[cfg(windows)]
    {
        Box::new(win::WinHello)
    }
    #[cfg(not(windows))]
    {
        Box::new(Unsupported)
    }
}

#[cfg(not(windows))]
struct Unsupported;

#[cfg(not(windows))]
impl HelloBackend for Unsupported {
    fn is_supported(&self) -> bool {
        false
    }
    fn open_or_create(&self, _name: &str) -> HelloResult<()> {
        Err(HelloError::Unavailable)
    }
    fn sign(&self, _name: &str, _data: &[u8]) -> HelloResult<Zeroizing<Vec<u8>>> {
        Err(HelloError::Unavailable)
    }
    fn delete(&self, _name: &str) -> HelloResult<()> {
        Ok(())
    }
}

#[cfg(windows)]
mod win {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    use windows::{
        core::{Array, HSTRING},
        Security::{
            Credentials::{KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus},
            Cryptography::CryptographicBuffer,
        },
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow};
    use zeroize::Zeroizing;

    use super::{HelloBackend, HelloError, HelloResult};

    /// Window class of the Windows Hello / Windows Security credential dialog.
    const DIALOG_CLASS: &str = "Credential Dialog Xaml Host";
    /// Stop looking for the dialog after this long (the call itself may take longer).
    const FOCUS_DEADLINE: Duration = Duration::from_secs(10);
    const FOCUS_POLL: Duration = Duration::from_millis(100);

    pub struct WinHello;

    impl HelloBackend for WinHello {
        fn is_supported(&self) -> bool {
            KeyCredentialManager::IsSupportedAsync()
                .and_then(|op| op.join())
                .unwrap_or(false)
        }

        fn open_or_create(&self, name: &str) -> HelloResult<()> {
            let name = HSTRING::from(name);
            let open = KeyCredentialManager::OpenAsync(&name)
                .and_then(|op| op.join())
                .map_err(|_| unavailable_or_failed())?;
            match open.Status().map_err(|_| HelloError::Failed)? {
                KeyCredentialStatus::Success => return Ok(()),
                KeyCredentialStatus::NotFound => {}
                other => return Err(map_status(other)),
            }
            let op = KeyCredentialManager::RequestCreateAsync(&name, KeyCredentialCreationOption::FailIfExists)
                .map_err(|_| HelloError::Failed)?;
            let created = with_focus(|| op.join()).map_err(|_| HelloError::Failed)?;
            match created.Status().map_err(|_| HelloError::Failed)? {
                KeyCredentialStatus::Success | KeyCredentialStatus::CredentialAlreadyExists => Ok(()),
                other => Err(map_status(other)),
            }
        }

        fn sign(&self, name: &str, data: &[u8]) -> HelloResult<Zeroizing<Vec<u8>>> {
            let credential = open_existing(name)?;
            let buffer = CryptographicBuffer::CreateFromByteArray(data).map_err(|_| HelloError::Failed)?;
            let op = credential.RequestSignAsync(&buffer).map_err(|_| HelloError::Failed)?;
            let result = with_focus(|| op.join()).map_err(|_| HelloError::Failed)?;
            match result.Status().map_err(|_| HelloError::Failed)? {
                KeyCredentialStatus::Success => {
                    let signed = result.Result().map_err(|_| HelloError::Failed)?;
                    let mut out = Array::<u8>::new();
                    CryptographicBuffer::CopyToByteArray(&signed, &mut out).map_err(|_| HelloError::Failed)?;
                    let signature = Zeroizing::new(out.to_vec());
                    // `Array` frees its buffer without wiping it; clear it ourselves.
                    out.iter_mut().for_each(|b| *b = 0);
                    Ok(signature)
                }
                other => Err(map_status(other)),
            }
        }

        fn delete(&self, name: &str) -> HelloResult<()> {
            match KeyCredentialManager::DeleteAsync(&HSTRING::from(name)).and_then(|op| op.join()) {
                Ok(()) => Ok(()),
                // Deleting a missing credential fails with "element not found".
                Err(_) if open_status(name) == Some(KeyCredentialStatus::NotFound) => Ok(()),
                Err(_) => Err(HelloError::Failed),
            }
        }
    }

    fn open_existing(name: &str) -> HelloResult<KeyCredential> {
        let open = KeyCredentialManager::OpenAsync(&HSTRING::from(name))
            .and_then(|op| op.join())
            .map_err(|_| unavailable_or_failed())?;
        match open.Status().map_err(|_| HelloError::Failed)? {
            KeyCredentialStatus::Success => open.Credential().map_err(|_| HelloError::Failed),
            other => Err(map_status(other)),
        }
    }

    fn open_status(name: &str) -> Option<KeyCredentialStatus> {
        KeyCredentialManager::OpenAsync(&HSTRING::from(name))
            .and_then(|op| op.join())
            .and_then(|r| r.Status())
            .ok()
    }

    /// A failing call usually means Hello is not set up; tell the two apart.
    fn unavailable_or_failed() -> HelloError {
        if WinHello.is_supported() {
            HelloError::Failed
        } else {
            HelloError::Unavailable
        }
    }

    fn map_status(status: KeyCredentialStatus) -> HelloError {
        match status {
            KeyCredentialStatus::NotFound => HelloError::NotFound,
            KeyCredentialStatus::UserCanceled | KeyCredentialStatus::UserPrefersPassword => HelloError::Canceled,
            KeyCredentialStatus::SecurityDeviceLocked => HelloError::DeviceLocked,
            _ => HelloError::Failed,
        }
    }

    /// Runs a blocking Hello call while a watcher thread brings the credential
    /// dialog to the foreground. Our window is in the foreground when the user
    /// starts the action, so `SetForegroundWindow` is allowed.
    fn with_focus<T>(call: impl FnOnce() -> windows::core::Result<T>) -> windows::core::Result<T> {
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let done = done.clone();
            thread::Builder::new()
                .name("xpass-hello-focus".into())
                .spawn(move || focus_dialog(&done))
                .ok()
        };
        let result = call();
        done.store(true, Ordering::Relaxed);
        if let Some(watcher) = watcher {
            let _ = watcher.join();
        }
        result
    }

    fn focus_dialog(done: &AtomicBool) {
        let class: Vec<u16> = DIALOG_CLASS.encode_utf16().chain(Some(0)).collect();
        let deadline = Instant::now() + FOCUS_DEADLINE;
        while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
            // SAFETY: `class` is a NUL-terminated UTF-16 string that outlives the call.
            let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
            if !hwnd.is_null() {
                // SAFETY: `hwnd` came from `FindWindowW`; a stale handle only makes the call fail.
                let ok = unsafe { SetForegroundWindow(hwnd) } != 0;
                if cfg!(debug_assertions) {
                    eprintln!("xpass: Hello dialog found, SetForegroundWindow = {ok}");
                }
                if ok {
                    return;
                }
            }
            thread::sleep(FOCUS_POLL);
        }
        if cfg!(debug_assertions) && !done.load(Ordering::Relaxed) {
            eprintln!("xpass: Hello dialog not focused within {FOCUS_DEADLINE:?}");
        }
    }
}

/// Real Windows Hello calls that never show a dialog. Ignored by default because
/// they depend on the machine: `cargo test -p xpass hello::win_tests -- --ignored`.
#[cfg(all(test, windows))]
mod win_tests {
    use super::*;

    #[test]
    #[ignore = "needs a Windows machine; run manually"]
    fn missing_credential_is_not_found_without_a_prompt() {
        let backend = system();
        let name = credential_name(uuid::Uuid::new_v4());
        let expected = if backend.is_supported() {
            HelloError::NotFound
        } else {
            HelloError::Unavailable
        };
        assert_eq!(backend.sign(&name, b"challenge").unwrap_err(), expected);
        assert_eq!(backend.delete(&name), Ok(()));
    }
}

#[cfg(test)]
pub mod fake {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use rand::RngCore;
    use zeroize::Zeroizing;

    use super::{HelloBackend, HelloError, HelloResult};

    #[derive(Default)]
    struct State {
        unsupported: bool,
        /// Credential name → private "key" bytes.
        credentials: HashMap<String, [u8; 16]>,
        /// Returned by the next `open_or_create` / `sign` call instead of a result.
        fail_next: Option<HelloError>,
        prompts: usize,
    }

    /// In-memory Hello for tests; clones share state. The "signature" is
    /// deterministic per credential and challenge, like the real one.
    #[derive(Clone, Default)]
    pub struct FakeHello(Arc<Mutex<State>>);

    impl FakeHello {
        pub fn set_supported(&self, supported: bool) {
            self.0.lock().unwrap().unsupported = !supported;
        }

        pub fn fail_next(&self, error: HelloError) {
            self.0.lock().unwrap().fail_next = Some(error);
        }

        /// Simulates a Hello reset (PIN removed): all credentials are gone.
        pub fn reset(&self) {
            self.0.lock().unwrap().credentials.clear();
        }

        /// Simulates the credential being re-created with a new key.
        pub fn recreate(&self, name: &str) {
            let mut key = [0u8; 16];
            rand::thread_rng().fill_bytes(&mut key);
            self.0.lock().unwrap().credentials.insert(name.to_owned(), key);
        }

        pub fn has(&self, name: &str) -> bool {
            self.0.lock().unwrap().credentials.contains_key(name)
        }

        pub fn names(&self) -> Vec<String> {
            self.0.lock().unwrap().credentials.keys().cloned().collect()
        }

        /// Number of Hello dialogs shown so far.
        pub fn prompts(&self) -> usize {
            self.0.lock().unwrap().prompts
        }
    }

    impl HelloBackend for FakeHello {
        fn is_supported(&self) -> bool {
            !self.0.lock().unwrap().unsupported
        }

        fn open_or_create(&self, name: &str) -> HelloResult<()> {
            let mut s = self.0.lock().unwrap();
            if s.unsupported {
                return Err(HelloError::Unavailable);
            }
            if s.credentials.contains_key(name) {
                return Ok(());
            }
            s.prompts += 1;
            if let Some(e) = s.fail_next.take() {
                return Err(e);
            }
            let mut key = [0u8; 16];
            rand::thread_rng().fill_bytes(&mut key);
            s.credentials.insert(name.to_owned(), key);
            Ok(())
        }

        fn sign(&self, name: &str, data: &[u8]) -> HelloResult<Zeroizing<Vec<u8>>> {
            let mut s = self.0.lock().unwrap();
            if s.unsupported {
                return Err(HelloError::Unavailable);
            }
            let key = *s.credentials.get(name).ok_or(HelloError::NotFound)?;
            s.prompts += 1;
            if let Some(e) = s.fail_next.take() {
                return Err(e);
            }
            Ok(Zeroizing::new([&key[..], data].concat()))
        }

        fn delete(&self, name: &str) -> HelloResult<()> {
            self.0.lock().unwrap().credentials.remove(name);
            Ok(())
        }
    }
}

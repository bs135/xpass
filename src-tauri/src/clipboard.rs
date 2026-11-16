//! Secure copy: secrets are put on the clipboard excluded from clipboard history
//! (`Win+V`), cloud clipboard and clipboard monitors, then cleared after a timeout,
//! on lock and on exit, but only if the clipboard still holds our copy (checked with
//! `GetClipboardSequenceNumber`), so we never wipe something the user copied since.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    thread,
    time::Duration,
};

use crate::error::{AppError, AppResult};

/// OS clipboard operations, abstracted for tests.
pub trait Backend: Send + Sync + 'static {
    /// Puts `text` on the clipboard, excluded from history, cloud and monitors.
    fn set_secret(&self, text: &str) -> AppResult<()>;
    /// Changes whenever the clipboard content changes.
    fn sequence(&self) -> u32;
    fn clear(&self) -> AppResult<()>;
}

#[derive(Clone)]
pub struct SecureClipboard(Arc<Inner>);

struct Inner {
    backend: Box<dyn Backend>,
    /// Our last copy, if it may still be on the clipboard.
    owned: Mutex<Option<Owned>>,
    generation: AtomicU64,
}

#[derive(Clone, Copy)]
struct Owned {
    sequence: u32,
    generation: u64,
}

impl SecureClipboard {
    pub fn new(backend: impl Backend) -> Self {
        Self(Arc::new(Inner {
            backend: Box::new(backend),
            owned: Mutex::new(None),
            generation: AtomicU64::new(0),
        }))
    }

    /// The real Windows clipboard.
    pub fn system() -> Self {
        Self::new(SystemClipboard)
    }

    /// Copies `text` and schedules clearing it after `clear_after`.
    pub fn copy(&self, text: &str, clear_after: Duration) -> AppResult<()> {
        let generation = {
            let mut owned = lock(&self.0.owned);
            self.0.backend.set_secret(text)?;
            let generation = self.0.generation.fetch_add(1, Ordering::SeqCst) + 1;
            *owned = Some(Owned {
                sequence: self.0.backend.sequence(),
                generation,
            });
            generation
        };
        let this = self.clone();
        thread::Builder::new()
            .name("xpass-clipboard-clear".into())
            .spawn(move || {
                thread::sleep(clear_after);
                this.clear_if_owned(Some(generation));
            })
            .map_err(|_| AppError::Internal)?;
        Ok(())
    }

    /// Clears the clipboard if it still holds our last copy (on lock and exit).
    pub fn clear_if_ours(&self) -> bool {
        self.clear_if_owned(None)
    }

    /// `only_generation`: a timer only clears the copy it was started for.
    fn clear_if_owned(&self, only_generation: Option<u64>) -> bool {
        let mut owned = lock(&self.0.owned);
        let Some(current) = *owned else {
            return false;
        };
        if only_generation.is_some_and(|g| g != current.generation) {
            return false;
        }
        *owned = None;
        // Someone else changed the clipboard since our copy: leave it alone.
        if self.0.backend.sequence() != current.sequence {
            return false;
        }
        self.0.backend.clear().is_ok()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct SystemClipboard;

#[cfg(windows)]
impl Backend for SystemClipboard {
    fn set_secret(&self, text: &str) -> AppResult<()> {
        use arboard::SetExtWindows;
        arboard::Clipboard::new()
            .and_then(|mut c| {
                c.set()
                    .exclude_from_history()
                    .exclude_from_cloud()
                    .exclude_from_monitoring()
                    .text(text)
            })
            .map_err(|_| AppError::Clipboard)
    }

    fn sequence(&self) -> u32 {
        // SAFETY: no arguments, no preconditions.
        unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
    }

    fn clear(&self) -> AppResult<()> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.clear())
            .map_err(|_| AppError::Clipboard)
    }
}

#[cfg(not(windows))]
impl Backend for SystemClipboard {
    fn set_secret(&self, _text: &str) -> AppResult<()> {
        Err(AppError::Clipboard)
    }

    fn sequence(&self) -> u32 {
        0
    }

    fn clear(&self) -> AppResult<()> {
        Err(AppError::Clipboard)
    }
}

#[cfg(test)]
pub mod fake {
    use std::sync::{Arc, Mutex};

    use super::Backend;
    use crate::error::AppResult;

    /// In-memory clipboard for tests; clones share state.
    #[derive(Clone, Default)]
    pub struct FakeClipboard(Arc<Mutex<(Option<String>, u32)>>);

    impl FakeClipboard {
        pub fn text(&self) -> Option<String> {
            self.0.lock().unwrap().0.clone()
        }

        /// Simulates the user copying something else.
        pub fn external_copy(&self, text: &str) {
            let mut s = self.0.lock().unwrap();
            *s = (Some(text.to_owned()), s.1 + 1);
        }
    }

    impl Backend for FakeClipboard {
        fn set_secret(&self, text: &str) -> AppResult<()> {
            self.external_copy(text);
            Ok(())
        }

        fn sequence(&self) -> u32 {
            self.0.lock().unwrap().1
        }

        fn clear(&self) -> AppResult<()> {
            let mut s = self.0.lock().unwrap();
            *s = (None, s.1 + 1);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::FakeClipboard, *};

    const SHORT: Duration = Duration::from_millis(60);
    const WAIT: Duration = Duration::from_millis(250);
    const LONG: Duration = Duration::from_secs(60);

    #[test]
    fn copied_secret_is_cleared_after_the_timeout() {
        let fake = FakeClipboard::default();
        let clip = SecureClipboard::new(fake.clone());
        clip.copy("s3cret", SHORT).unwrap();
        assert_eq!(fake.text().as_deref(), Some("s3cret"));
        thread::sleep(WAIT);
        assert_eq!(fake.text(), None);
    }

    #[test]
    fn something_copied_meanwhile_is_not_wiped() {
        let fake = FakeClipboard::default();
        let clip = SecureClipboard::new(fake.clone());
        clip.copy("s3cret", SHORT).unwrap();
        fake.external_copy("user text");
        thread::sleep(WAIT);
        assert_eq!(fake.text().as_deref(), Some("user text"));
        assert!(!clip.clear_if_ours());
    }

    #[test]
    fn an_older_timer_does_not_clear_a_newer_copy_early() {
        let fake = FakeClipboard::default();
        let clip = SecureClipboard::new(fake.clone());
        clip.copy("first", SHORT).unwrap();
        clip.copy("second", LONG).unwrap();
        thread::sleep(WAIT);
        assert_eq!(fake.text().as_deref(), Some("second"));
    }

    #[test]
    fn lock_clears_only_our_copy() {
        let fake = FakeClipboard::default();
        let clip = SecureClipboard::new(fake.clone());
        assert!(!clip.clear_if_ours(), "nothing copied yet");
        clip.copy("s3cret", LONG).unwrap();
        assert!(clip.clear_if_ours());
        assert_eq!(fake.text(), None);
        assert!(!clip.clear_if_ours(), "already cleared");
    }
}

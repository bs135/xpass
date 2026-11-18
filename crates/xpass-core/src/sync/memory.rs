//! In-memory [`RemoteStore`] for tests: the same compare-and-swap semantics as
//! GitHub, plus offline simulation and fault injection.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{blob_sha, Remote, RemoteStore, StoreError};

type Hook = Box<dyn FnOnce(&MemoryStore) + Send>;

#[derive(Default)]
pub struct MemoryStore {
    file: Mutex<Option<Remote>>,
    /// Error returned by every call while set (e.g. `Offline`, `Unauthorized`).
    failure: Mutex<Option<StoreError>>,
    before_put: Mutex<Option<Hook>>,
    /// Bodies returned by the next GETs instead of the stored file.
    bad_reads: Mutex<Vec<Vec<u8>>>,
    puts: Mutex<u32>,
    pauses: Mutex<u32>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_offline(&self, offline: bool) {
        self.fail_with(offline.then_some(StoreError::Offline));
    }

    /// Every call fails with `error` until called again with `None`.
    pub fn fail_with(&self, error: Option<StoreError>) {
        *lock(&self.failure) = error;
    }

    /// Runs once at the start of the next `put`, before the SHA check: simulates
    /// another device pushing between our GET and PUT.
    pub fn before_next_put(&self, hook: impl FnOnce(&MemoryStore) + Send + 'static) {
        *lock(&self.before_put) = Some(Box::new(hook));
    }

    /// The next GET returns `body` (with its own SHA) instead of the stored file.
    pub fn serve_once(&self, body: Vec<u8>) {
        lock(&self.bad_reads).push(body);
    }

    pub fn current(&self) -> Option<Remote> {
        lock(&self.file).clone()
    }

    /// Unconditional write (another device pushing).
    pub fn force(&self, bytes: &[u8]) {
        *lock(&self.file) = Some(Remote::new(bytes.to_vec()));
    }

    /// Number of successful PUTs.
    pub fn puts(&self) -> u32 {
        *lock(&self.puts)
    }

    /// Number of `pause` calls (retries of unreadable bodies).
    pub fn pauses(&self) -> u32 {
        *lock(&self.pauses)
    }

    fn check(&self) -> Result<(), StoreError> {
        match *lock(&self.failure) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn get_now(&self) -> Result<Option<Remote>, StoreError> {
        self.check()?;
        let mut bad = lock(&self.bad_reads);
        if !bad.is_empty() {
            return Ok(Some(Remote::new(bad.remove(0))));
        }
        Ok(self.current())
    }

    fn put_now(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError> {
        self.check()?;
        let hook = lock(&self.before_put).take();
        if let Some(hook) = hook {
            hook(self);
        }
        let mut file = lock(&self.file);
        if file.as_ref().map(|r| r.sha.as_str()) != expected {
            return Err(StoreError::Conflict);
        }
        let sha = blob_sha(bytes);
        *file = Some(Remote {
            bytes: bytes.to_vec(),
            sha: sha.clone(),
        });
        *lock(&self.puts) += 1;
        Ok(sha)
    }
}

impl RemoteStore for MemoryStore {
    async fn get(&self) -> Result<Option<Remote>, StoreError> {
        self.get_now()
    }

    async fn put(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError> {
        self.put_now(bytes, expected)
    }

    async fn pause(&self, _ms: u64) {
        *lock(&self.pauses) += 1;
    }
}

//! Remote storage abstraction with optimistic concurrency (compare-and-swap on
//! the git blob SHA), matching what the GitHub Contents API offers.

use std::sync::Mutex;

use sha1::{Digest, Sha1};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Expected SHA did not match the current remote file (someone else pushed).
    Conflict,
    /// Network unreachable / timeout. Caller keeps working offline.
    Offline(String),
    /// Token missing, expired or lacking permission.
    Unauthorized(String),
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Conflict => write!(f, "remote changed (sha mismatch)"),
            StoreError::Offline(m) => write!(f, "offline: {m}"),
            StoreError::Unauthorized(m) => write!(f, "unauthorized: {m}"),
            StoreError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub bytes: Vec<u8>,
    /// Git blob SHA-1 of `bytes` (hex).
    pub sha: String,
}

pub trait RemoteStore {
    /// `Ok(None)` when the file does not exist yet.
    fn get(&self) -> Result<Option<Remote>, StoreError>;
    /// Writes `bytes` only if the remote is still at `expected` (`None` = must not exist).
    /// Returns the new blob SHA.
    fn put(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError>;
}

impl<S: RemoteStore + ?Sized> RemoteStore for &S {
    fn get(&self) -> Result<Option<Remote>, StoreError> {
        (**self).get()
    }
    fn put(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError> {
        (**self).put(bytes, expected)
    }
}

/// Git blob SHA-1: `sha1("blob <len>\0" || content)`. Lets us know the SHA of a
/// file downloaded with the raw media type, which returns no metadata.
pub fn blob_sha(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

type Hook = Box<dyn FnOnce(&MemoryStore) + Send>;

/// In-memory store for tests. Same CAS semantics as GitHub, plus fault injection.
#[derive(Default)]
pub struct MemoryStore {
    file: Mutex<Option<Remote>>,
    offline: Mutex<bool>,
    before_put: Mutex<Option<Hook>>,
    pub puts: Mutex<u32>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_offline(&self, offline: bool) {
        *self.offline.lock().unwrap() = offline;
    }

    /// Runs once at the start of the next `put`, before the SHA check —
    /// used to simulate another device pushing in between our get and put.
    pub fn before_next_put(&self, hook: impl FnOnce(&MemoryStore) + Send + 'static) {
        *self.before_put.lock().unwrap() = Some(Box::new(hook));
    }

    pub fn current(&self) -> Option<Remote> {
        self.file.lock().unwrap().clone()
    }

    /// Unconditional write (simulates a push from another device).
    pub fn force(&self, bytes: &[u8]) {
        *self.file.lock().unwrap() = Some(Remote { bytes: bytes.to_vec(), sha: blob_sha(bytes) });
    }

    fn check_online(&self) -> Result<(), StoreError> {
        if *self.offline.lock().unwrap() { Err(StoreError::Offline("simulated".into())) } else { Ok(()) }
    }
}

impl RemoteStore for MemoryStore {
    fn get(&self) -> Result<Option<Remote>, StoreError> {
        self.check_online()?;
        Ok(self.current())
    }

    fn put(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError> {
        self.check_online()?;
        let hook = self.before_put.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        let mut file = self.file.lock().unwrap();
        let current = file.as_ref().map(|r| r.sha.as_str());
        if current != expected {
            return Err(StoreError::Conflict);
        }
        let sha = blob_sha(bytes);
        *file = Some(Remote { bytes: bytes.to_vec(), sha: sha.clone() });
        *self.puts.lock().unwrap() += 1;
        Ok(sha)
    }
}

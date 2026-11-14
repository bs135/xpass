//! A device: local encrypted copy + sync state + the sync loop.
//!
//! Offline-first: every edit is written to the local file first and marks the
//! device dirty. `sync()` reconciles with the remote whenever it is reachable.
//!
//! Local files (in the device directory):
//! - `vault.xpv`        latest local vault (encrypted, same format as remote)
//! - `sync-state.txt`   `base_sha` (remote SHA we last agreed with) and `dirty`

use std::{
    fs,
    path::{Path, PathBuf},
};

use rand::rngs::OsRng;
use vault_format::{crypto::KdfParams, model::Payload, Unlock, UnlockedVault};

use crate::{
    merge::merge,
    store::{RemoteStore, StoreError},
};

const VAULT_FILE: &str = "vault.xpv";
const STATE_FILE: &str = "sync-state.txt";
const MAX_ATTEMPTS: usize = 5;

#[derive(Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Nothing to do.
    UpToDate,
    /// Local changes uploaded; remote had not changed.
    Pushed,
    /// Remote changes downloaded; no local changes.
    Pulled,
    /// Both sides changed: merged and uploaded.
    Merged,
    /// Remote unreachable; local changes kept for later.
    Offline,
}

#[derive(Debug)]
pub enum SyncError {
    Store(StoreError),
    Vault(vault_format::Error),
    Io(std::io::Error),
    /// Gave up after repeated conflicts (another device keeps pushing).
    TooManyConflicts,
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Store(e) => write!(f, "store: {e}"),
            SyncError::Vault(e) => write!(f, "vault: {e}"),
            SyncError::Io(e) => write!(f, "io: {e}"),
            SyncError::TooManyConflicts => write!(f, "too many conflicting pushes, try again"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<StoreError> for SyncError {
    fn from(e: StoreError) -> Self {
        SyncError::Store(e)
    }
}
impl From<vault_format::Error> for SyncError {
    fn from(e: vault_format::Error) -> Self {
        SyncError::Vault(e)
    }
}
impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        SyncError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;

pub struct Device<S: RemoteStore> {
    dir: PathBuf,
    store: S,
    vault: UnlockedVault,
    base_sha: Option<String>,
    dirty: bool,
}

impl<S: RemoteStore> Device<S> {
    /// First device: creates a new vault locally (dirty, not yet pushed).
    pub fn create(dir: &Path, store: S, password: &[u8], kdf: KdfParams) -> Result<(Self, String)> {
        fs::create_dir_all(dir)?;
        let (vault, recovery) = UnlockedVault::create(&mut OsRng, password, kdf)?;
        let mut d = Self { dir: dir.to_owned(), store, vault, base_sha: None, dirty: true };
        d.write_local()?;
        Ok((d, recovery.to_string()))
    }

    /// New device: downloads the remote vault and unlocks it with the password.
    pub fn clone_remote(dir: &Path, store: S, password: &[u8]) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let remote = store.get()?.ok_or(SyncError::Store(StoreError::Other("remote vault not found".into())))?;
        let vault = UnlockedVault::unlock(&remote.bytes, Unlock::Password(password))?;
        let d = Self { dir: dir.to_owned(), store, vault, base_sha: Some(remote.sha), dirty: false };
        fs::write(d.dir.join(VAULT_FILE), &remote.bytes)?;
        d.write_state()?;
        Ok(d)
    }

    /// Existing device after restart: unlocks the local copy, no network needed.
    pub fn open(dir: &Path, store: S, password: &[u8]) -> Result<Self> {
        let bytes = fs::read(dir.join(VAULT_FILE))?;
        let vault = UnlockedVault::unlock(&bytes, Unlock::Password(password))?;
        let (base_sha, dirty) = read_state(dir)?;
        Ok(Self { dir: dir.to_owned(), store, vault, base_sha, dirty })
    }

    pub fn payload(&self) -> &Payload {
        &self.vault.payload
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// Applies an edit and persists it locally. Never touches the network.
    pub fn edit(&mut self, f: impl FnOnce(&mut Payload)) -> Result<()> {
        f(&mut self.vault.payload);
        self.dirty = true;
        self.write_local()
    }

    pub fn sync(&mut self) -> Result<SyncOutcome> {
        for _ in 0..MAX_ATTEMPTS {
            let remote = match self.store.get() {
                Ok(r) => r,
                Err(StoreError::Offline(_)) => return Ok(SyncOutcome::Offline),
                Err(e) => return Err(e.into()),
            };

            let (candidate, expected, outcome) = match remote {
                None => (None, None, SyncOutcome::Pushed),
                Some(r) if Some(&r.sha) == self.base_sha.as_ref() => {
                    if !self.dirty {
                        return Ok(SyncOutcome::UpToDate);
                    }
                    (None, Some(r.sha), SyncOutcome::Pushed)
                }
                Some(r) => {
                    let mut theirs = self.vault.open_sibling(&r.bytes)?;
                    if !self.dirty {
                        self.vault = theirs;
                        self.base_sha = Some(r.sha);
                        fs::write(self.dir.join(VAULT_FILE), &r.bytes)?;
                        self.write_state()?;
                        return Ok(SyncOutcome::Pulled);
                    }
                    theirs.payload = merge(&self.vault.payload, &theirs.payload);
                    (Some(theirs), Some(r.sha), SyncOutcome::Merged)
                }
            };

            // Push either our vault or the merged candidate; commit to memory only on success.
            let mut next = match candidate {
                Some(v) => v,
                None => self.vault.clone(),
            };
            let bytes = next.save(&mut OsRng)?;
            match self.store.put(&bytes, expected.as_deref()) {
                Ok(sha) => {
                    self.vault = next;
                    self.base_sha = Some(sha);
                    self.dirty = false;
                    write_atomic(&self.dir.join(VAULT_FILE), &bytes)?;
                    self.write_state()?;
                    return Ok(outcome);
                }
                Err(StoreError::Conflict) => continue,
                Err(StoreError::Offline(_)) => return Ok(SyncOutcome::Offline),
                Err(e) => return Err(e.into()),
            }
        }
        Err(SyncError::TooManyConflicts)
    }

    fn write_local(&mut self) -> Result<()> {
        let bytes = self.vault.save(&mut OsRng)?;
        write_atomic(&self.dir.join(VAULT_FILE), &bytes)?;
        self.write_state()
    }

    fn write_state(&self) -> Result<()> {
        let text = format!("base_sha={}\ndirty={}\n", self.base_sha.as_deref().unwrap_or(""), self.dirty);
        write_atomic(&self.dir.join(STATE_FILE), text.as_bytes())?;
        Ok(())
    }
}

fn read_state(dir: &Path) -> Result<(Option<String>, bool)> {
    let text = fs::read_to_string(dir.join(STATE_FILE))?;
    let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_owned();
    let sha = get("base_sha");
    Ok(((!sha.is_empty()).then_some(sha), get("dirty") == "true"))
}

/// Write to a temp file in the same directory, then rename over the target,
/// so a crash never leaves a half-written vault.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

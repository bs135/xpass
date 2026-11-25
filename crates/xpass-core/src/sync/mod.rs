//! Offline-first sync of the vault file with a remote copy (docs/plan-v0.4.0.md, S1–S8, S15).
//!
//! Every edit is written to the local file first; [`sync_once`] reconciles with the
//! remote whenever it is reachable. The remote is a single file with compare-and-swap
//! on its git blob SHA ([`RemoteStore`]), which is what the GitHub Contents API offers.
//!
//! ```text
//! sync_once: up to MAX_ATTEMPTS times
//!   snapshot local (vault clone, SyncState, dirty)
//!   GET remote                         unreadable body → retry once after 2 s
//!   no remote          → PUT(expected = none)                          → Pushed
//!   remote.sha == base → clean: UpToDate | dirty: PUT(expected = base)  → Pushed
//!   remote.sha != base → theirs = open_sibling(remote)                 (KeyChanged / DifferentVault stop)
//!                        clean, theirs covers ours: adopt theirs (local Hello slot kept) → Pulled
//!                        dirty, or the remote went back to an older copy (U10, A8):
//!                               merge, PUT(expected = remote.sha)       → Merged
//!   PUT Conflict → retry from the snapshot; the local side is only changed by `commit`
//! ```
//!
//! Key slots (S4, S5): Windows Hello slots are device-local, so the local Hello slot
//! is always kept and a remote one ignored. Password / recovery slots come from the
//! remote, unless this device changed them and has not pushed yet (`slots_dirty`).

pub mod memory;

use std::future::Future;

use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::{
    format::SlotKind,
    maintenance::{purge_tombstones, TOMBSTONE_TTL_MS},
    merge::{covers, merge},
    Error, Unlock, UnlockedVault,
};

/// Attempts per sync before giving up on repeated conflicts.
pub const MAX_ATTEMPTS: usize = 5;
/// Delay before re-reading a remote file whose body could not be parsed or decrypted.
pub const UNREADABLE_RETRY_MS: u64 = 2_000;

const ALL_SLOTS: [SlotKind; 3] = [SlotKind::Password, SlotKind::Recovery, SlotKind::Hello];

/// Errors of a remote store. Carries no free-form text, so it can never leak a
/// token, a URL or file content into logs or the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    /// The remote is not at the expected SHA any more (another device pushed).
    Conflict,
    /// Network unreachable or timed out; keep working locally.
    Offline,
    /// Token missing, invalid, expired or revoked.
    Unauthorized,
    /// Token valid but lacks a permission (e.g. Contents: write).
    Forbidden,
    /// API rate limit hit; retry after the given seconds when known.
    RateLimited { retry_after_s: Option<u64> },
    /// The repository does not exist or is not visible with this token.
    RepoNotFound,
    /// The server answered with something we cannot use.
    InvalidResponse,
    /// Any other HTTP status.
    Http(u16),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => write!(f, "remote changed (sha mismatch)"),
            Self::Offline => write!(f, "remote unreachable"),
            Self::Unauthorized => write!(f, "token invalid or expired"),
            Self::Forbidden => write!(f, "token lacks permission"),
            Self::RateLimited { .. } => write!(f, "rate limited"),
            Self::RepoNotFound => write!(f, "repository not found"),
            Self::InvalidResponse => write!(f, "invalid response"),
            Self::Http(status) => write!(f, "http status {status}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The remote file and its git blob SHA.
#[derive(Clone, PartialEq, Eq)]
pub struct Remote {
    pub bytes: Vec<u8>,
    /// Git blob SHA-1 of `bytes` (lowercase hex).
    pub sha: String,
}

impl Remote {
    pub fn new(bytes: Vec<u8>) -> Self {
        let sha = blob_sha(&bytes);
        Self { bytes, sha }
    }
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("len", &self.bytes.len())
            .field("sha", &self.sha)
            .finish()
    }
}

/// A single remote file with optimistic concurrency.
pub trait RemoteStore: Sync {
    /// `Ok(None)` when the file does not exist yet.
    fn get(&self) -> impl Future<Output = Result<Option<Remote>, StoreError>> + Send;

    /// Writes `bytes` only if the remote is still at `expected` (`None`: the file must
    /// not exist). Returns the new blob SHA; `Conflict` otherwise.
    fn put(&self, bytes: &[u8], expected: Option<&str>) -> impl Future<Output = Result<String, StoreError>> + Send;

    /// Waits before a retry. The default does not wait (tests); the app sleeps.
    fn pause(&self, ms: u64) -> impl Future<Output = ()> + Send {
        let _ = ms;
        async {}
    }
}

/// Git blob SHA-1: `sha1("blob <len>\0" || content)`, the SHA GitHub reports for a
/// file. Lets a raw download (no metadata) be used for compare-and-swap.
pub fn blob_sha(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Persistent sync bookkeeping (`sync-state.json`, S3).
///
/// Dirty is derived from the local file, not stored: the device has unpushed
/// changes when the file's SHA differs from `local_sha`. A crash between writing
/// the vault and writing this state can therefore cause one extra push, never a
/// lost edit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncState {
    /// Remote blob SHA the local file was last reconciled with.
    pub base_sha: Option<String>,
    /// Blob SHA of the local file right after that reconciliation.
    pub local_sha: Option<String>,
    /// The password or recovery slot was changed on this device and not pushed yet:
    /// the next merge keeps the local slots instead of adopting the remote ones (S5).
    pub slots_dirty: bool,
}

impl SyncState {
    /// State right after the local file `local_file` was reconciled with remote `base_sha`.
    pub fn synced(base_sha: String, local_file: &[u8]) -> Self {
        Self {
            base_sha: Some(base_sha),
            local_sha: Some(blob_sha(local_file)),
            slots_dirty: false,
        }
    }

    /// True when `local_file` has changes the remote has not seen.
    pub fn is_dirty(&self, local_file: &[u8]) -> bool {
        self.local_sha.as_deref() != Some(blob_sha(local_file).as_str())
    }
}

/// What a sync did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Nothing to do.
    UpToDate,
    /// Local changes uploaded; the remote had not changed.
    Pushed,
    /// Remote changes adopted; there were no local changes.
    Pulled,
    /// Both sides changed (or the remote went back to an older copy): merged and uploaded.
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    Store(StoreError),
    /// The remote file could not be used: `KeyChanged` (VaultKey rotated on another
    /// device, call [`resolve_key_change`]), `DifferentVault`, `Tampered`, ...
    Vault(Error),
    /// The vault is locked; sync needs the VaultKey in memory.
    Locked,
    /// Another device kept pushing; try again later.
    TooManyConflicts,
    /// Writing the local file or state failed.
    Local(std::io::ErrorKind),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "remote: {e}"),
            Self::Vault(e) => write!(f, "vault: {e}"),
            Self::Locked => write!(f, "the vault is locked"),
            Self::TooManyConflicts => write!(f, "too many conflicting pushes"),
            Self::Local(kind) => write!(f, "local file: {kind}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<StoreError> for SyncError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<Error> for SyncError {
    fn from(e: Error) -> Self {
        Self::Vault(e)
    }
}

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        Self::Local(e.kind())
    }
}

/// The local side of a sync at one point in time.
pub struct Snapshot {
    /// A copy of the unlocked vault.
    pub vault: UnlockedVault,
    pub state: SyncState,
    /// `state.is_dirty(local file)`.
    pub dirty: bool,
}

/// Result of a successful sync step, handed to [`LocalSide::commit`].
pub struct Synced {
    /// The vault as reconciled with the remote (key slots already resolved).
    pub vault: UnlockedVault,
    /// `vault` encrypted. For a push these are exactly the bytes now on the remote.
    pub bytes: Vec<u8>,
    /// Remote blob SHA these bytes were reconciled with.
    pub base_sha: String,
    pub outcome: SyncOutcome,
    /// The master password was changed on another device and adopted here (S5 notice).
    pub password_changed: bool,
}

/// The device: unlocked session, local file and `SyncState`.
///
/// `commit` must keep this invariant: `base_sha` may only move to `synced.base_sha`
/// if the local file afterwards contains everything in `synced` (otherwise a later
/// push with `expected = base_sha` would overwrite remote changes).
/// - No local change since the snapshot: write `synced.bytes` as the local file and
///   `SyncState::synced(synced.base_sha, &synced.bytes)`.
/// - Edits since the snapshot (session still unlocked): the session becomes
///   [`rebase`]`(current, &synced.vault, slots_changed_since_snapshot)`, the file is
///   that vault saved, and the state is `base_sha = synced.base_sha`,
///   `local_sha = blob_sha(&synced.bytes)` (so the device stays dirty), with
///   `slots_dirty` = slots changed since the snapshot.
/// - Locked meanwhile: write `synced.bytes` and the new state if the local file is
///   unchanged since the snapshot; otherwise leave file and state alone.
pub trait LocalSide {
    /// Identifies the snapshot (e.g. an edit generation counter).
    type Token: Send;

    /// `Err(SyncError::Locked)` when the vault is locked.
    fn snapshot(&mut self) -> Result<(Snapshot, Self::Token), SyncError>;

    fn commit(&mut self, token: Self::Token, synced: Synced) -> Result<(), SyncError>;
}

/// What to do after comparing the snapshot with the remote.
pub enum Step {
    UpToDate,
    /// Clean device, remote changed: take the remote content locally.
    Adopt {
        vault: UnlockedVault,
        base_sha: String,
        password_changed: bool,
    },
    /// Upload `vault` with `PUT(expected)`.
    Push {
        vault: UnlockedVault,
        expected: Option<String>,
        outcome: SyncOutcome,
        password_changed: bool,
    },
}

/// Pure decision for one sync attempt (no I/O).
pub fn reconcile(snap: &Snapshot, remote: Option<&Remote>, now_ms: i64) -> Result<Step, Error> {
    let ours = &snap.vault;
    let push = |mut vault: UnlockedVault, expected: Option<String>, outcome, password_changed| {
        purge_tombstones(&mut vault.payload, now_ms, TOMBSTONE_TTL_MS);
        Step::Push {
            vault,
            expected,
            outcome,
            password_changed,
        }
    };
    let Some(remote) = remote else {
        return Ok(push(ours.clone(), None, SyncOutcome::Pushed, false));
    };
    if snap.state.base_sha.as_deref() == Some(remote.sha.as_str()) {
        if !snap.dirty {
            return Ok(Step::UpToDate);
        }
        return Ok(push(ours.clone(), Some(remote.sha.clone()), SyncOutcome::Pushed, false));
    }

    let theirs = ours.open_sibling(&remote.bytes)?;
    let keep_local_credentials = snap.dirty && snap.state.slots_dirty;
    let (mut candidate, password_changed) = adopt_slots(ours, theirs, keep_local_credentials)?;
    let purge_before = now_ms.saturating_sub(TOMBSTONE_TTL_MS);
    // A clean device whose entries are all in the remote just takes it. When the remote
    // went back to an older copy, the clean device merges too, so no edit is lost (U10).
    if !snap.dirty && covers(&candidate.payload, &ours.payload, purge_before) {
        return Ok(Step::Adopt {
            vault: candidate,
            base_sha: remote.sha.clone(),
            password_changed,
        });
    }
    candidate.payload = merge(&ours.payload, &candidate.payload);
    Ok(push(
        candidate,
        Some(remote.sha.clone()),
        SyncOutcome::Merged,
        password_changed,
    ))
}

/// `theirs` with this device's Hello slot (S4) and, when `keep_local_credentials`,
/// this device's password and recovery slots (S5). Also reports whether the remote
/// brought a different password slot that is now adopted.
fn adopt_slots(
    ours: &UnlockedVault,
    mut theirs: UnlockedVault,
    keep_local_credentials: bool,
) -> Result<(UnlockedVault, bool), Error> {
    let kinds: &[SlotKind] = if keep_local_credentials {
        &ALL_SLOTS
    } else {
        &[SlotKind::Hello]
    };
    let password_changed = !keep_local_credentials && !ours.same_slots(&theirs, SlotKind::Password);
    theirs.copy_slots_from(ours, kinds)?;
    Ok((theirs, password_changed))
}

fn unreadable(e: &Error) -> bool {
    matches!(e, Error::Format(_) | Error::Payload(_) | Error::Tampered)
}

/// GET + reconcile, re-reading once after [`UNREADABLE_RETRY_MS`] when the body
/// cannot be parsed or decrypted (spike: a fresh file once came back wrong).
async fn fetch_step<S: RemoteStore>(store: &S, snap: &Snapshot, now_ms: i64) -> Result<Step, SyncError> {
    let mut retried = false;
    loop {
        let remote = store.get().await?;
        match reconcile(snap, remote.as_ref(), now_ms) {
            Err(e) if unreadable(&e) && !retried => {
                retried = true;
                store.pause(UNREADABLE_RETRY_MS).await;
            }
            other => return other.map_err(SyncError::Vault),
        }
    }
}

/// One full sync. The local side is only changed through [`LocalSide::commit`],
/// after a successful PUT (or for an adopt, after the GET).
pub async fn sync_once<S: RemoteStore, L: LocalSide>(
    store: &S,
    local: &mut L,
    now_ms: i64,
) -> Result<SyncOutcome, SyncError> {
    for _ in 0..MAX_ATTEMPTS {
        let (snap, token) = local.snapshot()?;
        match fetch_step(store, &snap, now_ms).await? {
            Step::UpToDate => return Ok(SyncOutcome::UpToDate),
            Step::Adopt {
                mut vault,
                base_sha,
                password_changed,
            } => {
                let bytes = vault.save(&mut OsRng)?;
                local.commit(
                    token,
                    Synced {
                        vault,
                        bytes,
                        base_sha,
                        outcome: SyncOutcome::Pulled,
                        password_changed,
                    },
                )?;
                return Ok(SyncOutcome::Pulled);
            }
            Step::Push {
                mut vault,
                expected,
                outcome,
                password_changed,
            } => {
                let bytes = vault.save(&mut OsRng)?;
                match store.put(&bytes, expected.as_deref()).await {
                    Ok(sha) => {
                        local.commit(
                            token,
                            Synced {
                                vault,
                                bytes,
                                base_sha: sha,
                                outcome,
                                password_changed,
                            },
                        )?;
                        return Ok(outcome);
                    }
                    Err(StoreError::Conflict) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Err(SyncError::TooManyConflicts)
}

/// S7: the VaultKey was rotated on another device (`sync_once` returned
/// `Vault(KeyChanged)`). Unlocks the remote with `password` (the master password as
/// set on that device), merges our payload into it, adopts its key and slots, drops
/// this device's Hello slot (it wraps the old key; re-enroll) and pushes.
///
/// `Vault(BadCredentials)` for a wrong password (the caller throttles). Falls back
/// to [`sync_once`] when the remote no longer needs it (gone, or same key again).
pub async fn resolve_key_change<S: RemoteStore, L: LocalSide>(
    store: &S,
    local: &mut L,
    password: &[u8],
    now_ms: i64,
) -> Result<SyncOutcome, SyncError> {
    // A copy holding the new key, so a retry after a conflict needs no second Argon2 run.
    let mut keyholder: Option<UnlockedVault> = None;
    for _ in 0..MAX_ATTEMPTS {
        let (snap, token) = local.snapshot()?;
        let Some(remote) = store.get().await? else {
            return sync_once(store, local, now_ms).await;
        };
        match snap.vault.open_sibling(&remote.bytes) {
            Err(Error::KeyChanged) => {}
            Err(e) => return Err(e.into()),
            Ok(_) => return sync_once(store, local, now_ms).await,
        }
        let theirs = match keyholder.as_ref().map(|k| k.open_sibling(&remote.bytes)) {
            Some(Ok(theirs)) => theirs,
            Some(Err(Error::KeyChanged)) | None => UnlockedVault::unlock(&remote.bytes, Unlock::Password(password))?,
            Some(Err(e)) => return Err(e.into()),
        };
        if theirs.vault_id() != snap.vault.vault_id() {
            return Err(Error::DifferentVault.into());
        }
        keyholder = Some(theirs.clone());
        let mut merged = merge_after_key_change(&snap.vault, theirs);
        purge_tombstones(&mut merged.payload, now_ms, TOMBSTONE_TTL_MS);
        let bytes = merged.save(&mut OsRng)?;
        match store.put(&bytes, Some(&remote.sha)).await {
            Ok(sha) => {
                local.commit(
                    token,
                    Synced {
                        vault: merged,
                        bytes,
                        base_sha: sha,
                        outcome: SyncOutcome::Merged,
                        password_changed: false,
                    },
                )?;
                return Ok(SyncOutcome::Merged);
            }
            Err(StoreError::Conflict) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(SyncError::TooManyConflicts)
}

/// `theirs` (unlocked with the new key) with our payload merged in and no Hello slot.
pub fn merge_after_key_change(ours: &UnlockedVault, mut theirs: UnlockedVault) -> UnlockedVault {
    theirs.remove_hello_slot();
    theirs.payload = merge(&ours.payload, &theirs.payload);
    theirs
}

/// Joining from a new device: unlocks the downloaded remote file with the master
/// password and drops the Hello slot it may carry (another device's, S4).
/// The caller saves the result as the local file and stores
/// `SyncState::synced(remote.sha, &saved_bytes)`.
pub fn join(remote_file: &[u8], password: &[u8]) -> Result<UnlockedVault, Error> {
    let mut vault = UnlockedVault::unlock(remote_file, Unlock::Password(password))?;
    vault.remove_hello_slot();
    Ok(vault)
}

/// Edits made while a sync was in flight, applied on top of its result (see
/// [`LocalSide`]). Payloads are merged; key slots come from `synced`, except the
/// Hello slot (always local) and, when `local_slots_changed`, the password and
/// recovery slots. If the key changed (key-change resolution), local slots no
/// longer fit and `synced`'s slots are kept.
pub fn rebase(current: &UnlockedVault, synced: &UnlockedVault, local_slots_changed: bool) -> UnlockedVault {
    let mut out = synced.clone();
    if out.same_key(current) {
        let kinds: &[SlotKind] = if local_slots_changed {
            &ALL_SLOTS
        } else {
            &[SlotKind::Hello]
        };
        out.copy_slots_from(current, kinds).expect("same key checked above");
    }
    out.payload = merge(&current.payload, &synced.payload);
    out
}

/// Key-less push on lock / exit (S8): uploads the local file as it is if the remote
/// is still at `base_sha`. `Ok(None)` when the remote moved on (merging needs the
/// key; the next unlock syncs). On `Ok(Some(sha))` the caller stores
/// `SyncState::synced(sha, local_file)`.
pub async fn push_raw<S: RemoteStore>(
    store: &S,
    local_file: &[u8],
    base_sha: Option<&str>,
) -> Result<Option<String>, StoreError> {
    match store.put(local_file, base_sha).await {
        Ok(sha) => Ok(Some(sha)),
        Err(StoreError::Conflict) => Ok(None),
        Err(e) => Err(e),
    }
}

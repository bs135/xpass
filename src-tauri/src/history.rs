//! "Remove old versions from GitHub" (v0.7 plan U10, T41).
//!
//! Every sync is a commit, so the branch keeps every earlier vault file, including
//! the ones encrypted with a VaultKey that was rotated away. This replaces the branch
//! with one commit that has the same tree and no parent:
//!
//! 1. read the branch head → its commit → its tree; the vault blob in that tree must
//!    be the one this device synced last (`base_sha`), else `Moved` (sync first);
//! 2. create a commit with that tree and no parent;
//! 3. read the head again: it must not have moved meanwhile (`Moved`, nothing changed;
//!    the unused commit is garbage collected by GitHub);
//! 4. force-update the branch to the new commit (`Refused` when branch protection
//!    or a ruleset blocks force pushes: nothing changed);
//! 5. read the vault back: its blob SHA must still be `base_sha` (`Unverified`).
//!
//! Another device can still push between 3 and 4. Its commit is dropped from the
//! branch, but its edits stay in its local file: its next sync finds a remote that
//! lacks them and merges them back (`xpass_core::merge::covers`).
//!
//! The steps go through [`GitData`], so the sequence is tested without a network.

use std::future::Future;

use serde::Serialize;
use xpass_core::sync::StoreError;

use crate::error::AppError;

pub const COMMIT_MESSAGE: &str = "xpass: history removed";
/// Other file names shown in the dialog at most.
pub const MAX_LISTED_FILES: usize = 20;

/// The branch in use and where it points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHead {
    pub branch: String,
    pub commit: String,
    pub tree: String,
    /// Parents of the head commit: 0 means the branch has a single commit already.
    pub parents: usize,
}

/// A file (blob) in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    pub sha: String,
}

/// The Git Data API calls the rewrite needs.
pub trait GitData: Sync {
    /// Path of the vault file in the repository.
    fn vault_path(&self) -> &str;
    fn head(&self) -> impl Future<Output = Result<BranchHead, RewriteError>> + Send;
    /// Every file of `tree`, recursively.
    fn tree_files(&self, tree: &str) -> impl Future<Output = Result<Vec<TreeFile>, RewriteError>> + Send;
    /// Creates a commit of `tree` with no parent; returns its SHA.
    fn create_root_commit(
        &self,
        tree: &str,
        message: &str,
    ) -> impl Future<Output = Result<String, RewriteError>> + Send;
    /// Points `branch` at `commit` even when that is not a fast-forward.
    fn force_branch(&self, branch: &str, commit: &str) -> impl Future<Output = Result<(), RewriteError>> + Send;
    /// Blob SHA of the vault file on the branch now (`None`: missing).
    fn vault_sha(&self) -> impl Future<Output = Result<Option<String>, RewriteError>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteError {
    Store(StoreError),
    /// Branch protection or a ruleset refused the forced update.
    Refused,
    /// The vault on GitHub is not the one this device synced last.
    Moved,
    /// The branch was updated but the vault read back is not the expected one.
    Unverified,
}

impl From<StoreError> for RewriteError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<RewriteError> for AppError {
    fn from(e: RewriteError) -> Self {
        match e {
            RewriteError::Store(e) => e.into(),
            RewriteError::Refused => Self::HistoryRewriteRefused,
            RewriteError::Moved => Self::SyncRequired,
            RewriteError::Unverified => Self::HistoryRewriteFailed,
        }
    }
}

/// What the dialog shows before the user confirms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPreview {
    pub branch: String,
    /// The branch has one commit already: nothing to remove.
    pub single_commit: bool,
    /// Number of files in the branch besides the vault (their history goes too).
    pub other_files: usize,
    /// Up to [`MAX_LISTED_FILES`] of their paths.
    pub other_paths: Vec<String>,
}

/// Result of a rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteReport {
    /// `false`: the branch had one commit already, nothing was changed.
    pub rewritten: bool,
    pub other_files: usize,
}

fn others(files: &[TreeFile], vault_path: &str) -> Vec<String> {
    files
        .iter()
        .filter(|f| f.path != vault_path)
        .map(|f| f.path.clone())
        .collect()
}

pub async fn preview<G: GitData>(api: &G) -> Result<HistoryPreview, RewriteError> {
    let head = api.head().await?;
    let files = api.tree_files(&head.tree).await?;
    let mut other_paths = others(&files, api.vault_path());
    let other_files = other_paths.len();
    other_paths.truncate(MAX_LISTED_FILES);
    Ok(HistoryPreview {
        branch: head.branch,
        single_commit: head.parents == 0,
        other_files,
        other_paths,
    })
}

/// Steps 1–5 above. `base_sha`: the vault blob this device synced last.
pub async fn rewrite<G: GitData>(api: &G, base_sha: &str) -> Result<RewriteReport, RewriteError> {
    let head = api.head().await?;
    let files = api.tree_files(&head.tree).await?;
    let vault = files.iter().find(|f| f.path == api.vault_path());
    if vault.map(|f| f.sha.as_str()) != Some(base_sha) {
        return Err(RewriteError::Moved);
    }
    let other_files = others(&files, api.vault_path()).len();
    if head.parents == 0 {
        return Ok(RewriteReport {
            rewritten: false,
            other_files,
        });
    }

    let commit = api.create_root_commit(&head.tree, COMMIT_MESSAGE).await?;
    if api.head().await?.commit != head.commit {
        return Err(RewriteError::Moved);
    }
    api.force_branch(&head.branch, &commit).await?;

    match api.vault_sha().await {
        Ok(Some(sha)) if sha == base_sha => Ok(RewriteReport {
            rewritten: true,
            other_files,
        }),
        _ => Err(RewriteError::Unverified),
    }
}

#[cfg(test)]
pub mod fake {
    //! A recording [`GitData`] for tests.

    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::*;

    pub struct FakeGit {
        pub vault_path: String,
        pub state: Mutex<FakeState>,
    }

    pub struct FakeState {
        pub head: BranchHead,
        pub files: Vec<TreeFile>,
        /// Requests in order, e.g. `"POST commits tree=t1 parents=[]"`.
        pub calls: Vec<String>,
        /// Answer of the forced update.
        pub force_result: Result<(), RewriteError>,
        /// Head returned by the second `head()` (another device pushed meanwhile).
        pub moved_head: Option<String>,
        /// Vault SHA read back after the update (`None`: the files' vault SHA).
        pub read_back: Option<String>,
        pub fail_all: Option<StoreError>,
        heads_read: usize,
    }

    impl FakeGit {
        pub fn new(vault_sha: &str, parents: usize, other: &[&str]) -> Self {
            let vault_path = "xpass/vault.xpv".to_owned();
            let mut files = vec![TreeFile {
                path: vault_path.clone(),
                sha: vault_sha.into(),
            }];
            files.extend(other.iter().map(|p| TreeFile {
                path: (*p).into(),
                sha: "f".repeat(40),
            }));
            Self {
                vault_path,
                state: Mutex::new(FakeState {
                    head: BranchHead {
                        branch: "main".into(),
                        commit: "c1".into(),
                        tree: "t1".into(),
                        parents,
                    },
                    files,
                    calls: Vec::new(),
                    force_result: Ok(()),
                    moved_head: None,
                    read_back: None,
                    fail_all: None,
                    heads_read: 0,
                }),
            }
        }

        pub fn state(&self) -> MutexGuard<'_, FakeState> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub fn calls(&self) -> Vec<String> {
            self.state().calls.clone()
        }

        fn call(&self, what: String) -> Result<MutexGuard<'_, FakeState>, RewriteError> {
            let mut s = self.state();
            if let Some(e) = s.fail_all {
                return Err(e.into());
            }
            s.calls.push(what);
            Ok(s)
        }
    }

    impl GitData for FakeGit {
        fn vault_path(&self) -> &str {
            &self.vault_path
        }

        async fn head(&self) -> Result<BranchHead, RewriteError> {
            let mut s = self.call("GET ref heads/main".into())?;
            s.heads_read += 1;
            let mut head = s.head.clone();
            if s.heads_read > 1 {
                if let Some(moved) = s.moved_head.clone() {
                    head.commit = moved;
                }
            }
            Ok(head)
        }

        async fn tree_files(&self, tree: &str) -> Result<Vec<TreeFile>, RewriteError> {
            Ok(self.call(format!("GET tree {tree} recursive"))?.files.clone())
        }

        async fn create_root_commit(&self, tree: &str, message: &str) -> Result<String, RewriteError> {
            drop(self.call(format!("POST commit tree={tree} parents=[] message={message}"))?);
            Ok("c2".into())
        }

        async fn force_branch(&self, branch: &str, commit: &str) -> Result<(), RewriteError> {
            let mut s = self.call(format!("PATCH ref heads/{branch} sha={commit} force"))?;
            s.force_result?;
            s.head.commit = commit.into();
            s.head.parents = 0;
            Ok(())
        }

        async fn vault_sha(&self) -> Result<Option<String>, RewriteError> {
            let s = self.call("GET vault".into())?;
            Ok(s.read_back.clone().or_else(|| {
                s.files
                    .iter()
                    .find(|f| f.path == self.vault_path)
                    .map(|f| f.sha.clone())
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    use super::{fake::FakeGit, *};

    const SHA: &str = "ce013625030ba8dba906f756967f9e9ca394464a";

    fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn rewrite_makes_a_parentless_commit_of_the_same_tree_and_forces_the_branch() {
        let git = FakeGit::new(SHA, 1, &["README.md"]);
        let report = block_on(rewrite(&git, SHA)).unwrap();
        assert_eq!(
            report,
            RewriteReport {
                rewritten: true,
                other_files: 1
            }
        );
        assert_eq!(
            git.calls(),
            [
                "GET ref heads/main",
                "GET tree t1 recursive",
                "POST commit tree=t1 parents=[] message=xpass: history removed",
                "GET ref heads/main",
                "PATCH ref heads/main sha=c2 force",
                "GET vault",
            ]
        );
    }

    #[test]
    fn a_vault_this_device_has_not_synced_is_not_rewritten() {
        let git = FakeGit::new(&"a".repeat(40), 3, &[]);
        assert_eq!(block_on(rewrite(&git, SHA)).unwrap_err(), RewriteError::Moved);
        assert_eq!(git.calls().len(), 2, "no commit, no ref update");
    }

    #[test]
    fn a_push_while_rewriting_stops_before_the_forced_update() {
        let git = FakeGit::new(SHA, 1, &[]);
        git.state().moved_head = Some("c9".into());
        assert_eq!(block_on(rewrite(&git, SHA)).unwrap_err(), RewriteError::Moved);
        assert!(!git.calls().iter().any(|c| c.starts_with("PATCH")));
    }

    #[test]
    fn a_refused_forced_update_changes_nothing() {
        let git = FakeGit::new(SHA, 1, &[]);
        git.state().force_result = Err(RewriteError::Refused);
        assert_eq!(block_on(rewrite(&git, SHA)).unwrap_err(), RewriteError::Refused);
        assert_eq!(git.state().head.commit, "c1");
        assert!(!git.calls().contains(&"GET vault".to_owned()));
        assert_eq!(AppError::from(RewriteError::Refused), AppError::HistoryRewriteRefused);
    }

    #[test]
    fn a_wrong_read_back_is_reported() {
        let git = FakeGit::new(SHA, 1, &[]);
        git.state().read_back = Some("b".repeat(40));
        assert_eq!(block_on(rewrite(&git, SHA)).unwrap_err(), RewriteError::Unverified);
        assert_eq!(AppError::from(RewriteError::Unverified), AppError::HistoryRewriteFailed);
    }

    #[test]
    fn a_single_commit_branch_is_left_alone() {
        let git = FakeGit::new(SHA, 0, &[]);
        let report = block_on(rewrite(&git, SHA)).unwrap();
        assert!(!report.rewritten);
        assert_eq!(git.calls().len(), 2);
    }

    #[test]
    fn store_errors_pass_through() {
        let git = FakeGit::new(SHA, 1, &[]);
        git.state().fail_all = Some(StoreError::Offline);
        assert_eq!(
            block_on(rewrite(&git, SHA)).unwrap_err(),
            RewriteError::Store(StoreError::Offline)
        );
        assert_eq!(
            AppError::from(RewriteError::Store(StoreError::Offline)),
            AppError::SyncOffline
        );
    }

    #[test]
    fn preview_lists_the_other_files() {
        let names: Vec<String> = (0..25).map(|i| format!("notes/{i}.md")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let git = FakeGit::new(SHA, 4, &refs);
        let p = block_on(preview(&git)).unwrap();
        assert_eq!(p.branch, "main");
        assert!(!p.single_commit);
        assert_eq!(p.other_files, 25);
        assert_eq!(p.other_paths.len(), MAX_LISTED_FILES);
        assert_eq!(p.other_paths[0], "notes/0.md");
    }
}

//! GitHub Contents API backend of the sync engine (S13), async on Tauri's tokio runtime.
//!
//! - `GET /repos/{owner}/{repo}/contents/{path}?ref={branch}` with
//!   `Accept: application/vnd.github.raw+json`: raw bytes, no 1 MB limit; the blob
//!   SHA is computed locally (`xpass_core::sync::blob_sha`).
//! - `PUT` the same path with `{message, content (base64), sha?, branch?}`;
//!   409, and 422 mentioning the sha, mean another device pushed first: `Conflict`.
//!
//! The token is only ever put into the `Authorization` header. Errors are mapped to
//! [`StoreError`] codes; reqwest errors and response bodies are never formatted
//! into messages or logs.

use std::{sync::OnceLock, time::Duration};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use reqwest::{header, Method, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use xpass_core::{
    format,
    sync::{Remote, RemoteStore, StoreError},
};
use zeroize::Zeroizing;

use crate::settings::SyncSettings;

const API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
const COMMIT_MESSAGE: &str = "xpass: update vault";
/// Larger remote bodies are refused (a vault of 2000 entries is ~750 KB).
const MAX_BODY: usize = 64 * 1024 * 1024;

fn client() -> Result<&'static reqwest::Client, StoreError> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(concat!("xpass/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(60))
                .https_only(true)
                .build()
                .ok()
        })
        .as_ref()
        .ok_or(StoreError::InvalidResponse)
}

/// One vault file in one repository, with the token to reach it.
pub struct GitHubStore {
    token: Zeroizing<String>,
    owner: String,
    repo: String,
    /// `None`: the repository's default branch.
    branch: Option<String>,
    path: String,
}

impl std::fmt::Debug for GitHubStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubStore")
            .field("owner", &self.owner)
            .field("repo", &self.repo)
            .field("branch", &self.branch)
            .field("path", &self.path)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Result of "Test connection" (S12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionReport {
    /// `false` for a public repository: warn before using it.
    pub private: bool,
    /// The branch that will be used (configured or the repository default).
    pub branch: String,
    /// The configured branch exists (always true for the default branch).
    pub branch_exists: bool,
    pub remote: RemoteFile,
}

/// What is at the vault path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteFile {
    /// Nothing yet: the first sync creates it.
    Missing,
    /// An xpass vault with the same id as this PC's vault.
    SameVault,
    /// An xpass vault with another id (`DifferentVault`).
    OtherVault,
    /// An xpass vault, and this PC has none yet (joining).
    Found,
    /// Some other file.
    NotAVault,
}

#[derive(Deserialize)]
struct RepoInfo {
    private: bool,
    default_branch: String,
}

#[derive(Deserialize)]
struct PutResponse {
    content: PutContent,
}

#[derive(Deserialize)]
struct PutContent {
    sha: String,
}

impl GitHubStore {
    /// `settings` must be validated (`SyncSettings::validate`).
    pub fn new(token: Zeroizing<String>, settings: &SyncSettings) -> Result<Self, StoreError> {
        let (owner, repo) = settings.repo.split_once('/').ok_or(StoreError::RepoNotFound)?;
        Ok(Self {
            token,
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            branch: (!settings.branch.is_empty()).then(|| settings.branch.clone()),
            path: settings.path.clone(),
        })
    }

    fn url(&self, tail: &[&str]) -> Url {
        let mut url = Url::parse(API).expect("valid API base");
        url.path_segments_mut()
            .expect("https URL has a path")
            .extend(["repos", self.owner.as_str(), self.repo.as_str()])
            .extend(tail);
        url
    }

    fn contents_url(&self) -> Url {
        let mut tail = vec!["contents"];
        tail.extend(self.path.split('/'));
        self.url(&tail)
    }

    fn request(&self, method: Method, url: Url) -> Result<RequestBuilder, StoreError> {
        Ok(client()?
            .request(method, url)
            .bearer_auth(self.token.as_str())
            .header("X-GitHub-Api-Version", API_VERSION)
            .header(header::CACHE_CONTROL, "no-cache"))
    }

    async fn send(&self, rb: RequestBuilder) -> Result<Response, StoreError> {
        rb.send().await.map_err(network_error)
    }

    async fn repo_info(&self) -> Result<RepoInfo, StoreError> {
        let rb = self
            .request(Method::GET, self.url(&[]))?
            .header(header::ACCEPT, "application/vnd.github+json");
        let resp = self.send(rb).await?;
        match resp.status() {
            StatusCode::OK => json(resp).await,
            StatusCode::NOT_FOUND => Err(StoreError::RepoNotFound),
            _ => Err(error_from(resp).await),
        }
    }

    async fn branch_exists(&self, branch: &str) -> Result<bool, StoreError> {
        let rb = self
            .request(Method::GET, self.url(&["branches", branch]))?
            .header(header::ACCEPT, "application/vnd.github+json");
        let resp = self.send(rb).await?;
        match resp.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            _ => Err(error_from(resp).await),
        }
    }

    /// S12: repository access and visibility, branch, and what is at the path.
    /// `local_vault_id` is this PC's vault, if it has one.
    pub async fn test_connection(&self, local_vault_id: Option<Uuid>) -> Result<ConnectionReport, StoreError> {
        let info = self.repo_info().await?;
        let (branch, branch_exists) = match &self.branch {
            Some(b) => (b.clone(), self.branch_exists(b).await?),
            None => (info.default_branch.clone(), true),
        };
        let remote = if branch_exists {
            match self.get().await? {
                None => RemoteFile::Missing,
                Some(remote) => classify_file(&remote.bytes, local_vault_id),
            }
        } else {
            RemoteFile::Missing
        };
        Ok(ConnectionReport {
            private: info.private,
            branch,
            branch_exists,
            remote,
        })
    }
}

impl RemoteStore for GitHubStore {
    async fn get(&self) -> Result<Option<Remote>, StoreError> {
        let mut url = self.contents_url();
        if let Some(branch) = &self.branch {
            url.query_pairs_mut().append_pair("ref", branch);
        }
        let rb = self
            .request(Method::GET, url)?
            .header(header::ACCEPT, "application/vnd.github.raw+json");
        let resp = self.send(rb).await?;
        match resp.status() {
            StatusCode::OK => {
                let bytes = resp.bytes().await.map_err(network_error)?;
                if bytes.len() > MAX_BODY {
                    return Err(StoreError::InvalidResponse);
                }
                Ok(Some(Remote::new(bytes.to_vec())))
            }
            // A missing file and a missing repository both answer 404.
            StatusCode::NOT_FOUND => self.repo_info().await.map(|_| None),
            _ => Err(error_from(resp).await),
        }
    }

    async fn put(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError> {
        let mut body = serde_json::json!({
            "message": COMMIT_MESSAGE,
            "content": B64.encode(bytes),
        });
        if let Some(sha) = expected {
            body["sha"] = sha.into();
        }
        if let Some(branch) = &self.branch {
            body["branch"] = branch.as_str().into();
        }
        let body = serde_json::to_vec(&body).map_err(|_| StoreError::InvalidResponse)?;
        let rb = self
            .request(Method::PUT, self.contents_url())?
            .header(header::ACCEPT, "application/vnd.github+json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body);
        let resp = self.send(rb).await?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => {
                let sha = json::<PutResponse>(resp).await?.content.sha;
                if is_blob_sha(&sha) {
                    Ok(sha)
                } else {
                    Err(StoreError::InvalidResponse)
                }
            }
            _ => Err(error_from(resp).await),
        }
    }

    async fn pause(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

fn classify_file(bytes: &[u8], local_vault_id: Option<Uuid>) -> RemoteFile {
    match format::parse(bytes) {
        Err(_) => RemoteFile::NotAVault,
        Ok(file) => match local_vault_id {
            None => RemoteFile::Found,
            Some(id) if id == Uuid::from_bytes(file.header.vault_id) => RemoteFile::SameVault,
            Some(_) => RemoteFile::OtherVault,
        },
    }
}

fn is_blob_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Transport failures: unreachable, DNS, TLS, timeout, connection reset. The
/// error itself is dropped (it may contain the URL).
fn network_error(e: reqwest::Error) -> StoreError {
    if e.is_builder() {
        StoreError::InvalidResponse
    } else {
        StoreError::Offline
    }
}

async fn json<T: serde::de::DeserializeOwned>(resp: Response) -> Result<T, StoreError> {
    let bytes = resp.bytes().await.map_err(network_error)?;
    serde_json::from_slice(&bytes).map_err(|_| StoreError::InvalidResponse)
}

/// Rate-limit hints from a response.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RateLimit {
    /// `x-ratelimit-remaining: 0`.
    exhausted: bool,
    /// `retry-after` (seconds), or seconds until `x-ratelimit-reset`.
    retry_after_s: Option<u64>,
}

fn rate_limit(headers: &header::HeaderMap, now_s: u64) -> RateLimit {
    let num = |name: &str| -> Option<u64> { headers.get(name)?.to_str().ok()?.trim().parse().ok() };
    let exhausted = num("x-ratelimit-remaining") == Some(0);
    let retry_after_s = num("retry-after").or_else(|| {
        exhausted
            .then(|| num("x-ratelimit-reset"))
            .flatten()
            .map(|reset| reset.saturating_sub(now_s))
    });
    RateLimit {
        exhausted,
        retry_after_s,
    }
}

async fn error_from(resp: Response) -> StoreError {
    let status = resp.status().as_u16();
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let limit = rate_limit(resp.headers(), now_s);
    // Bodies are small JSON error documents; only used to tell 422 cases apart.
    let body = resp.bytes().await.unwrap_or_default();
    classify(status, limit, &body)
}

/// Maps an unsuccessful status to a [`StoreError`].
fn classify(status: u16, limit: RateLimit, body: &[u8]) -> StoreError {
    match status {
        401 => StoreError::Unauthorized,
        403 | 429 if limit.exhausted || limit.retry_after_s.is_some() || status == 429 => StoreError::RateLimited {
            retry_after_s: limit.retry_after_s,
        },
        403 => StoreError::Forbidden,
        404 => StoreError::RepoNotFound,
        409 => StoreError::Conflict,
        // 422 without a sha for an existing file, or with a sha that does not match.
        422 if mentions_sha(body) => StoreError::Conflict,
        _ => StoreError::Http(status),
    }
}

fn mentions_sha(body: &[u8]) -> bool {
    body.windows(3).any(|w| w.eq_ignore_ascii_case(b"sha"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(repo: &str, branch: &str, path: &str) -> SyncSettings {
        SyncSettings {
            repo: repo.into(),
            branch: branch.into(),
            path: path.into(),
            ..SyncSettings::default()
        }
    }

    fn store(branch: &str, path: &str) -> GitHubStore {
        GitHubStore::new(
            Zeroizing::new("token".into()),
            &settings("octo/vault-repo", branch, path),
        )
        .unwrap()
    }

    #[test]
    fn urls_are_built_and_escaped() {
        let s = store("", "xpass/vault.xpv");
        assert_eq!(
            s.contents_url().as_str(),
            "https://api.github.com/repos/octo/vault-repo/contents/xpass/vault.xpv"
        );
        let s = store("feature/x", "my vault/a#b?.xpv");
        assert_eq!(
            s.contents_url().as_str(),
            "https://api.github.com/repos/octo/vault-repo/contents/my%20vault/a%23b%3F.xpv"
        );
        assert_eq!(
            s.url(&["branches", "feature/x"]).as_str(),
            "https://api.github.com/repos/octo/vault-repo/branches/feature%2Fx"
        );
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let s = GitHubStore::new(
            Zeroizing::new("github_pat_SECRET_VALUE_123".into()),
            &settings("o/r", "", "v.xpv"),
        )
        .unwrap();
        let text = format!("{s:?}");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn statuses_map_to_store_errors() {
        let none = RateLimit::default();
        assert_eq!(classify(401, none, b""), StoreError::Unauthorized);
        assert_eq!(classify(403, none, b"{}"), StoreError::Forbidden);
        assert_eq!(classify(404, none, b""), StoreError::RepoNotFound);
        assert_eq!(classify(409, none, b""), StoreError::Conflict);
        assert_eq!(
            classify(
                422,
                none,
                br#"{"message":"Invalid request.\n\n\"sha\" wasn't supplied."}"#
            ),
            StoreError::Conflict
        );
        assert_eq!(
            classify(422, none, br#"{"message":"Branch nope not found"}"#),
            StoreError::Http(422)
        );
        assert_eq!(classify(500, none, b""), StoreError::Http(500));
        let limited = RateLimit {
            exhausted: true,
            retry_after_s: Some(42),
        };
        assert_eq!(
            classify(403, limited, b""),
            StoreError::RateLimited {
                retry_after_s: Some(42)
            }
        );
        assert_eq!(
            classify(429, none, b""),
            StoreError::RateLimited { retry_after_s: None }
        );
    }

    #[test]
    fn rate_limit_headers() {
        let mut h = header::HeaderMap::new();
        assert_eq!(rate_limit(&h, 100), RateLimit::default());
        h.insert("x-ratelimit-remaining", "0".parse().unwrap());
        h.insert("x-ratelimit-reset", "160".parse().unwrap());
        assert_eq!(
            rate_limit(&h, 100),
            RateLimit {
                exhausted: true,
                retry_after_s: Some(60)
            }
        );
        h.insert("retry-after", "5".parse().unwrap());
        assert_eq!(rate_limit(&h, 100).retry_after_s, Some(5));
    }

    #[test]
    fn remote_files_are_classified() {
        use rand::rngs::OsRng;
        use xpass_core::{crypto::KdfParams, UnlockedVault};
        let kdf = KdfParams {
            m_kib: 8 * 1024,
            t: 1,
            p: 1,
        };
        let (mut v, _) = UnlockedVault::create(&mut OsRng, b"pw", kdf).unwrap();
        let file = v.save(&mut OsRng).unwrap();
        assert_eq!(classify_file(&file, None), RemoteFile::Found);
        assert_eq!(classify_file(&file, Some(v.vault_id())), RemoteFile::SameVault);
        assert_eq!(classify_file(&file, Some(Uuid::nil())), RemoteFile::OtherVault);
        assert_eq!(classify_file(b"# readme", None), RemoteFile::NotAVault);
    }

    #[test]
    fn blob_sha_shape() {
        assert!(is_blob_sha("ce013625030ba8dba906f756967f9e9ca394464a"));
        assert!(!is_blob_sha("CE013625030BA8DBA906F756967F9E9CA394464A"));
        assert!(!is_blob_sha("ce01"));
    }
}

//! GitHub Contents API backend.
//!
//! - GET  /repos/{owner}/{repo}/contents/{path}?ref={branch}
//!   with `Accept: application/vnd.github.raw+json` -> raw bytes (up to 100 MB),
//!   no JSON/base64 wrapper. The blob SHA is computed locally (`blob_sha`).
//! - PUT  same path, JSON `{message, content(base64), sha?, branch}`.
//!   409 = sha mismatch, 422 = sha missing for an existing file -> `Conflict`.
//! - DELETE same path, JSON `{message, sha, branch}` (spike cleanup only).

use std::{sync::Mutex, time::Duration};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use reqwest::{blocking::Client, header, StatusCode};
use serde_json::json;
use zeroize::Zeroizing;

use crate::store::{blob_sha, Remote, RemoteStore, StoreError};

const API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";

pub struct GitHubStore {
    client: Client,
    token: Zeroizing<String>,
    owner: String,
    repo: String,
    path: String,
    branch: String,
    /// `x-ratelimit-remaining` / `x-ratelimit-limit` from the last response.
    pub last_rate_limit: Mutex<Option<String>>,
    /// Diagnostics of the last GET: status, headers of interest, body preview.
    pub last_get_meta: Mutex<Option<String>>,
}

impl GitHubStore {
    pub fn new(token: Zeroizing<String>, owner_repo: &str, path: &str, branch: &str) -> Result<Self, StoreError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or_else(|| StoreError::Other("repo must be owner/name".into()))?;
        let client = Client::builder()
            .user_agent("xpass-sync-spike")
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| StoreError::Other(e.to_string()))?;
        Ok(Self {
            client,
            token,
            owner: owner.into(),
            repo: repo.into(),
            path: path.into(),
            branch: branch.into(),
            last_rate_limit: Mutex::new(None),
            last_get_meta: Mutex::new(None),
        })
    }

    /// Same repo/branch/token, different file path.
    pub fn with_path(&self, path: &str) -> Self {
        Self {
            client: self.client.clone(),
            token: self.token.clone(),
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            path: path.into(),
            branch: self.branch.clone(),
            last_rate_limit: Mutex::new(None),
            last_get_meta: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    fn url(&self) -> String {
        format!("{API}/repos/{}/{}/contents/{}", self.owner, self.repo, self.path)
    }

    fn request(&self, method: reqwest::Method) -> reqwest::blocking::RequestBuilder {
        self.request_to(method, self.url())
    }

    fn request_to(&self, method: reqwest::Method, url: String) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, url)
            .bearer_auth(self.token.as_str())
            .header("X-GitHub-Api-Version", API_VERSION)
    }

    fn send(&self, rb: reqwest::blocking::RequestBuilder) -> Result<reqwest::blocking::Response, StoreError> {
        let resp = rb.send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                StoreError::Offline(e.to_string())
            } else {
                StoreError::Other(e.to_string())
            }
        })?;
        let h = resp.headers();
        if let (Some(rem), Some(lim)) = (h.get("x-ratelimit-remaining"), h.get("x-ratelimit-limit")) {
            *self.last_rate_limit.lock().unwrap() =
                Some(format!("{}/{}", rem.to_str().unwrap_or("?"), lim.to_str().unwrap_or("?")));
        }
        Ok(resp)
    }

    /// Deletes the file if it exists (spike cleanup).
    pub fn delete(&self, message: &str) -> Result<bool, StoreError> {
        let Some(current) = self.get()? else { return Ok(false) };
        let body = json!({ "message": message, "sha": current.sha, "branch": self.branch });
        let resp = self.send(self.request(reqwest::Method::DELETE).json(&body))?;
        match resp.status() {
            s if s.is_success() => Ok(true),
            s => Err(status_error(s, resp.text().unwrap_or_default())),
        }
    }
}

impl RemoteStore for GitHubStore {
    fn get(&self) -> Result<Option<Remote>, StoreError> {
        let rb = self
            .request_to(reqwest::Method::GET, format!("{}?ref={}", self.url(), self.branch))
            .header(header::ACCEPT, "application/vnd.github.raw+json")
            .header(header::CACHE_CONTROL, "no-cache");
        let resp = self.send(rb)?;
        let h = resp.headers();
        let hv = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("-").to_owned();
        let meta = format!(
            "status={} content-type={} content-encoding={} content-length={} etag={} x-github-request-id={}",
            resp.status(),
            hv("content-type"),
            hv("content-encoding"),
            hv("content-length"),
            hv("etag"),
            hv("x-github-request-id"),
        );
        match resp.status() {
            StatusCode::OK => {
                let bytes = resp.bytes().map_err(|e| StoreError::Offline(e.to_string()))?.to_vec();
                let preview = match std::str::from_utf8(&bytes) {
                    Ok(t) => format!("utf8 {:?}", t.chars().take(300).collect::<String>()),
                    Err(_) => format!("binary, first 16 bytes {:02x?}", &bytes[..bytes.len().min(16)]),
                };
                *self.last_get_meta.lock().unwrap() = Some(format!("{meta}
       body: {preview}"));
                let sha = blob_sha(&bytes);
                Ok(Some(Remote { bytes, sha }))
            }
            StatusCode::NOT_FOUND => Ok(None),
            s => Err(status_error(s, resp.text().unwrap_or_default())),
        }
    }

    fn put(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, StoreError> {
        let mut body = json!({
            "message": "xpass: update vault",
            "content": B64.encode(bytes),
            "branch": self.branch,
        });
        if let Some(sha) = expected {
            body["sha"] = json!(sha);
        }
        let resp = self.send(self.request(reqwest::Method::PUT).json(&body))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => {
                let v: serde_json::Value = resp.json().map_err(|e| StoreError::Other(e.to_string()))?;
                v["content"]["sha"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| StoreError::Other("PUT response without content.sha".into()))
            }
            StatusCode::CONFLICT => Err(StoreError::Conflict),
            StatusCode::UNPROCESSABLE_ENTITY => {
                let text = resp.text().unwrap_or_default();
                if text.contains("sha") { Err(StoreError::Conflict) } else { Err(StoreError::Other(format!("422: {text}"))) }
            }
            s => Err(status_error(s, resp.text().unwrap_or_default())),
        }
    }
}

fn status_error(status: StatusCode, body: String) -> StoreError {
    let msg = format!("{status}: {}", body.chars().take(200).collect::<String>());
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => StoreError::Unauthorized(msg),
        _ => StoreError::Other(msg),
    }
}

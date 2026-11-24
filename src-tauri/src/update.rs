//! Update check (D10, v0.6 plan H9): Rust asks the GitHub Releases API for the
//! latest release and compares it with this build. Nothing is downloaded or
//! installed; the WebView never talks to the network (CSP unchanged). A newer
//! release is shown in Settings → About and opened in the browser on request.
//!
//! - Manual: `check_update` (Settings → About → Check for updates).
//! - Optional at startup (`checkUpdatesOnStartup`, default off): at most one
//!   request per [`STARTUP_INTERVAL_MS`], result sent as `update://status`.
//!
//! Only `https://github.com/bs135/xpass/releases/…` is ever opened, whatever the
//! API answers. Errors carry no response content.

use std::{sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use url::Url;

use crate::{
    error::{AppError, AppResult},
    shell,
    state::{now_ms, AppState},
};

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/bs135/xpass/releases/latest";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/bs135/xpass/releases/";
/// The API answer is ~5–20 KB; anything larger is refused.
const MAX_BODY: usize = 256 * 1024;
const STARTUP_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
/// Event with the [`UpdateInfo`] of a startup check.
pub const EVENT_UPDATE_STATUS: &str = "update://status";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub newer: bool,
    /// Release page; always under [`RELEASE_PAGE_PREFIX`].
    pub url: String,
    pub checked_at: i64,
}

/// Last result, kept for the Settings badge and for `open_release_page`.
#[derive(Default)]
pub struct UpdateState(Mutex<Option<UpdateInfo>>);

impl UpdateState {
    fn get(&self) -> Option<UpdateInfo> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, info: UpdateInfo) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(info);
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `x.y.z` with an optional leading `v`; pre-release or build suffixes are refused.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.strip_prefix('v').unwrap_or(s).split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    let v = (next()?, next()?, next()?);
    parts.next().is_none().then_some(v)
}

/// Release page URL from the API, accepted only under our releases path.
fn release_url(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    (url.as_str().starts_with(RELEASE_PAGE_PREFIX) && url.query().is_none() && url.fragment().is_none()).then_some(url)
}

/// Turns the API answer into an [`UpdateInfo`] for this build.
fn evaluate(current: &str, body: &[u8], now: i64) -> AppResult<UpdateInfo> {
    let release: Release = serde_json::from_slice(body).map_err(|_| AppError::UpdateFailed)?;
    if release.draft || release.prerelease {
        return Err(AppError::UpdateFailed);
    }
    let latest = parse_version(&release.tag_name).ok_or(AppError::UpdateFailed)?;
    let ours = parse_version(current).ok_or(AppError::UpdateFailed)?;
    let url = release_url(&release.html_url).ok_or(AppError::UpdateFailed)?;
    Ok(UpdateInfo {
        current: current.to_owned(),
        latest: format!("{}.{}.{}", latest.0, latest.1, latest.2),
        newer: latest > ours,
        url: url.into(),
        checked_at: now,
    })
}

async fn fetch_latest() -> AppResult<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("xpass/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .https_only(true)
        .build()
        .map_err(|_| AppError::UpdateFailed)?;
    let mut response = client
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| AppError::UpdateFailed)?;
    if !response.status().is_success() {
        return Err(AppError::UpdateFailed);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| AppError::UpdateFailed)? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(AppError::UpdateFailed);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// One check against GitHub; stores the result and the check time.
async fn check(app: &AppHandle) -> AppResult<UpdateInfo> {
    let body = fetch_latest().await?;
    let info = evaluate(env!("CARGO_PKG_VERSION"), &body, now_ms())?;
    app.state::<UpdateState>().set(info.clone());
    app.state::<AppState>().record_update_check(info.checked_at);
    Ok(info)
}

/// Whether a startup check is due: the setting is on and the last check is at
/// least [`STARTUP_INTERVAL_MS`] old (or there was none).
fn startup_check_due(enabled: bool, last: Option<i64>, now: i64) -> bool {
    enabled && last.is_none_or(|t| now.saturating_sub(t) >= STARTUP_INTERVAL_MS || t > now)
}

/// Starts the optional startup check in the background.
pub fn start(app: AppHandle) {
    let settings = app.state::<AppState>().settings();
    if !startup_check_due(
        settings.check_updates_on_startup,
        settings.last_update_check_at,
        now_ms(),
    ) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        match check(&app).await {
            Ok(info) => {
                if let Err(e) = app.emit(EVENT_UPDATE_STATUS, &info) {
                    eprintln!("xpass: update status event failed: {e}");
                }
            }
            Err(_) => eprintln!("xpass: startup update check failed"),
        }
    });
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> AppResult<UpdateInfo> {
    check(&app).await
}

/// Result of the last check in this run (startup or manual), if any.
#[tauri::command]
pub fn update_status(state: tauri::State<'_, UpdateState>) -> Option<UpdateInfo> {
    state.get()
}

/// Opens the release page of the last check in the default browser.
#[tauri::command]
pub async fn open_release_page(state: tauri::State<'_, UpdateState>) -> AppResult<()> {
    let info = state.get().ok_or(AppError::UpdateFailed)?;
    let url = release_url(&info.url).ok_or(AppError::UpdateFailed)?;
    tauri::async_runtime::spawn_blocking(move || shell::open_url(&url))
        .await
        .map_err(|_| AppError::Internal)?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(tag: &str, url: &str) -> Vec<u8> {
        serde_json::json!({ "tag_name": tag, "html_url": url, "draft": false, "prerelease": false, "body": "notes" })
            .to_string()
            .into_bytes()
    }

    const PAGE: &str = "https://github.com/bs135/xpass/releases/tag/v0.6.0";

    #[test]
    fn newer_equal_and_older_releases() {
        let newer = evaluate("0.5.0", &body("v0.6.0", PAGE), 1).unwrap();
        assert!(newer.newer);
        assert_eq!(newer.latest, "0.6.0");
        assert_eq!(newer.current, "0.5.0");
        assert_eq!(newer.url, PAGE);
        assert!(!evaluate("0.6.0", &body("v0.6.0", PAGE), 1).unwrap().newer);
        assert!(!evaluate("0.10.0", &body("v0.9.9", PAGE), 1).unwrap().newer);
        assert!(
            evaluate("0.9.0", &body("0.10.0", PAGE), 1).unwrap().newer,
            "numeric, not text order"
        );
    }

    #[test]
    fn malformed_or_prerelease_tags_are_refused() {
        for tag in ["v0.6", "v0.6.0-rc1", "v0.6.0.1", "latest", "v.6.0", "v1.2.x", ""] {
            assert_eq!(
                evaluate("0.5.0", &body(tag, PAGE), 1),
                Err(AppError::UpdateFailed),
                "{tag}"
            );
        }
        let pre = serde_json::json!({ "tag_name": "v0.7.0", "html_url": PAGE, "prerelease": true }).to_string();
        assert_eq!(evaluate("0.5.0", pre.as_bytes(), 1), Err(AppError::UpdateFailed));
        assert_eq!(evaluate("0.5.0", b"not json", 1), Err(AppError::UpdateFailed));
    }

    #[test]
    fn only_our_release_pages_are_accepted() {
        assert!(release_url(PAGE).is_some());
        for bad in [
            "https://github.com/bs135/xpass/issues/1",
            "https://github.com/evil/xpass/releases/tag/v1.0.0",
            "https://github.com.evil.example/bs135/xpass/releases/",
            "http://github.com/bs135/xpass/releases/tag/v1.0.0",
            "https://github.com/bs135/xpass/releases/tag/v1?x=1",
            "javascript:alert(1)",
        ] {
            assert!(release_url(bad).is_none(), "{bad}");
            assert_eq!(
                evaluate("0.5.0", &body("v9.0.0", bad), 1),
                Err(AppError::UpdateFailed),
                "{bad}"
            );
        }
    }

    /// Real request to GitHub: `cargo test -p xpass --lib update -- --ignored`.
    #[test]
    #[ignore = "network"]
    fn live_latest_release_parses() {
        let body = tauri::async_runtime::block_on(fetch_latest()).unwrap();
        let info = evaluate("0.0.1", &body, 1).unwrap();
        assert!(info.newer, "{info:?}");
        assert!(info.url.starts_with(RELEASE_PAGE_PREFIX), "{info:?}");
    }

    #[test]
    fn startup_check_is_opt_in_and_daily() {
        let day = STARTUP_INTERVAL_MS;
        assert!(!startup_check_due(false, None, 10 * day));
        assert!(startup_check_due(true, None, 10 * day));
        assert!(!startup_check_due(true, Some(10 * day - 1000), 10 * day));
        assert!(startup_check_due(true, Some(9 * day), 10 * day));
        assert!(startup_check_due(true, Some(11 * day), 10 * day), "clock moved back");
    }
}

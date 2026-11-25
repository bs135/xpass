//! Signed in-app update (v0.7 plan U2–U7), on top of `tauri-plugin-updater`,
//! used from Rust only: the WebView has no `updater:*` permission and never
//! talks to the network (CSP unchanged).
//!
//! - Check: the plugin reads `latest.json` from the endpoint in `tauri.conf.json`
//!   (`…/releases/latest/download/latest.json`). A release counts only if its
//!   version is plain `x.y.z` and newer than this build, and its download URL is
//!   under `https://github.com/bs135/xpass/releases/download/v<version>/`.
//! - Install (`install_update`), only after the user confirmed in the UI:
//!   download with progress (`update://progress`); the plugin verifies the
//!   minisign signature against the public key built into the app, including the
//!   version the file was signed for (`requireSignedVersion`); then the vault is
//!   locked and unsynced changes are pushed (bounded), and the installer runs.
//!   On Windows the plugin ends the process when the installer has started, so
//!   `RunEvent::Exit` does not run: everything it would do happens before.
//! - Optional startup check (`checkUpdatesOnStartup`, default off): at most once
//!   per [`STARTUP_INTERVAL_MS`]; it never downloads.
//!
//! Debug builds only: `XPASS_UPDATE_ENDPOINT` and `XPASS_UPDATE_PUBKEY` point the
//! updater at a local test server and a throwaway key (manual test, plan U7).
//! Errors carry no response content.

use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

use crate::{
    error::{AppError, AppResult},
    lock_watch::{self, LockReason},
    shell,
    state::{now_ms, AppState},
    sync_worker,
};

const RELEASE_PAGE_PREFIX: &str = "https://github.com/bs135/xpass/releases/";
const LATEST_RELEASE_PAGE: &str = "https://github.com/bs135/xpass/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/bs135/xpass/releases/download/";
/// Release notes shown in About are cut to this many characters.
const MAX_NOTES: usize = 4000;
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A download fails after this long without data (no limit on the total time).
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
/// Event with the [`UpdateInfo`] of a startup check.
pub const EVENT_UPDATE_STATUS: &str = "update://status";
/// Download progress of `install_update` ([`Progress`]).
pub const EVENT_UPDATE_PROGRESS: &str = "update://progress";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    /// The newer version when there is one, otherwise the running version.
    pub latest: String,
    pub newer: bool,
    /// Release page; always under [`RELEASE_PAGE_PREFIX`].
    pub url: String,
    /// Release notes (plain text, at most [`MAX_NOTES`] characters).
    pub notes: Option<String>,
    /// Publication date of the newer release (Unix ms).
    pub date: Option<i64>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// Last result, for the Settings badge, `open_release_page` and `install_update`.
#[derive(Default)]
pub struct UpdateState {
    last: Mutex<Option<(UpdateInfo, Option<Update>)>>,
    installing: AtomicBool,
}

impl UpdateState {
    fn info(&self) -> Option<UpdateInfo> {
        self.lock().as_ref().map(|(info, _)| info.clone())
    }

    fn pending(&self) -> Option<Update> {
        self.lock().as_ref().and_then(|(_, update)| update.clone())
    }

    fn set(&self, info: UpdateInfo, update: Option<Update>) {
        *self.lock() = Some((info, update));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(UpdateInfo, Option<Update>)>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }
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

/// Release page URL, accepted only under our releases path.
fn release_url(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    (url.as_str().starts_with(RELEASE_PAGE_PREFIX) && url.query().is_none() && url.fragment().is_none()).then_some(url)
}

/// Where the installer of `version` may come from: our release assets, or (debug
/// builds with a test endpoint) the test server's origin.
fn download_allowed(url: &Url, version: &str, test_origin: Option<&Url>) -> bool {
    if url.query().is_some() || url.fragment().is_some() || url.username() != "" || url.password().is_some() {
        return false;
    }
    if let Some(origin) = test_origin {
        if url.origin() == origin.origin() {
            return true;
        }
    }
    let prefix = format!("{DOWNLOAD_PREFIX}v{version}/");
    url.as_str()
        .strip_prefix(&prefix)
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
}

/// Cuts `notes` to [`MAX_NOTES`] characters (on a character boundary).
fn cap_notes(notes: Option<&str>) -> Option<String> {
    let notes = notes?.trim();
    (!notes.is_empty()).then(|| notes.chars().take(MAX_NOTES).collect())
}

/// The plugin found a newer release: accept it only if its version and URL pass.
fn evaluate(
    current: &str,
    version: &str,
    download_url: &Url,
    notes: Option<&str>,
    date: Option<i64>,
    test_origin: Option<&Url>,
    now: i64,
) -> AppResult<UpdateInfo> {
    let latest = parse_version(version).ok_or(AppError::UpdateSignature)?;
    let ours = parse_version(current).ok_or(AppError::UpdateFailed)?;
    if latest <= ours || !download_allowed(download_url, version, test_origin) {
        return Err(AppError::UpdateSignature);
    }
    let version = format!("{}.{}.{}", latest.0, latest.1, latest.2);
    let url = release_url(&format!("{RELEASE_PAGE_PREFIX}tag/v{version}")).ok_or(AppError::UpdateFailed)?;
    Ok(UpdateInfo {
        current: current.to_owned(),
        latest: version,
        newer: true,
        url: url.into(),
        notes: cap_notes(notes),
        date,
        checked_at: now,
    })
}

fn up_to_date(current: &str, now: i64) -> UpdateInfo {
    UpdateInfo {
        current: current.to_owned(),
        latest: current.to_owned(),
        newer: false,
        url: LATEST_RELEASE_PAGE.to_owned(),
        notes: None,
        date: None,
        checked_at: now,
    }
}

/// Debug builds only: a test endpoint / public key from the environment (plan U7).
fn debug_override(name: &str) -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    } else {
        None
    }
}

fn test_endpoint() -> Option<Url> {
    debug_override("XPASS_UPDATE_ENDPOINT").and_then(|s| Url::parse(&s).ok())
}

fn plugin_error(e: tauri_plugin_updater::Error) -> AppError {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Minisign(_)
        | E::SignatureUtf8(_)
        | E::Base64(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion
        | E::InvalidUpdaterFormat => AppError::UpdateSignature,
        _ => AppError::UpdateFailed,
    }
}

fn updater(app: &AppHandle) -> AppResult<tauri_plugin_updater::Updater> {
    let test = test_endpoint();
    let https_only = test.is_none();
    let mut builder = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        // Plain releases only, and only newer ones (no downgrade, no pre-release).
        .version_comparator(|current, release| {
            release.version.pre.is_empty() && release.version.build.is_empty() && release.version > current
        })
        .configure_client(move |client| {
            client
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
                .https_only(https_only)
        });
    if let Some(endpoint) = test {
        builder = builder.endpoints(vec![endpoint]).map_err(|_| AppError::UpdateFailed)?;
    }
    if let Some(pubkey) = debug_override("XPASS_UPDATE_PUBKEY") {
        builder = builder.pubkey(pubkey);
    }
    builder.build().map_err(plugin_error)
}

/// One check; stores the result (and the update to install) and the check time.
async fn check(app: &AppHandle) -> AppResult<UpdateInfo> {
    // The version the plugin compares with (from tauri.conf.json, i.e. package.json;
    // `release:prepare` keeps it equal to the crate version).
    let current = app.package_info().version.to_string();
    let current = current.as_str();
    let found = updater(app)?.check().await.map_err(plugin_error)?;
    let now = now_ms();
    let info = match &found {
        Some(update) => evaluate(
            current,
            &update.version,
            &update.download_url,
            update.body.as_deref(),
            update.date.map(|d| (d.unix_timestamp_nanos() / 1_000_000) as i64),
            test_endpoint().as_ref(),
            now,
        )?,
        None => up_to_date(current, now),
    };
    app.state::<UpdateState>()
        .set(info.clone(), found.filter(|_| info.newer));
    app.state::<AppState>().record_update_check(now);
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

/// Download (verified by the plugin), then lock and push, then install. Each
/// step runs only if the one before it succeeded; `install` does not return on
/// success (the process ends).
async fn run_install<D, DF, P, PF, I>(download: D, prepare: P, install: I) -> AppResult<()>
where
    D: FnOnce() -> DF,
    DF: Future<Output = AppResult<Vec<u8>>>,
    P: FnOnce() -> PF,
    PF: Future<Output = ()>,
    I: FnOnce(Vec<u8>) -> AppResult<()>,
{
    let bytes = download().await?;
    prepare().await;
    install(bytes)
}

/// What `RunEvent::Exit` would do: lock (wipes the session and our clipboard
/// copy) and push unsynced changes, bounded like on exit.
async fn prepare_for_install(app: &AppHandle) {
    lock_watch::lock_app(app, LockReason::Update);
    if let Some(state) = app.try_state::<AppState>() {
        sync_worker::push_locked(state.inner()).await;
    }
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> AppResult<UpdateInfo> {
    check(&app).await
}

/// Result of the last check in this run (startup or manual), if any.
#[tauri::command]
pub fn update_status(state: tauri::State<'_, UpdateState>) -> Option<UpdateInfo> {
    state.info()
}

/// Installs the update found by the last check. The UI asks the user first.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let update = state.pending().ok_or(AppError::UpdateFailed)?;
    if state.installing.swap(true, Ordering::SeqCst) {
        return Err(AppError::UpdateFailed);
    }
    let result = run_install(
        || {
            let app = app.clone();
            let update = update.clone();
            async move {
                let mut downloaded = 0u64;
                let mut last = Instant::now() - PROGRESS_INTERVAL;
                update
                    .download(
                        |chunk, total| {
                            downloaded += chunk as u64;
                            if last.elapsed() >= PROGRESS_INTERVAL || total == Some(downloaded) {
                                last = Instant::now();
                                let _ = app.emit(EVENT_UPDATE_PROGRESS, Progress { downloaded, total });
                            }
                        },
                        || {},
                    )
                    .await
                    .map_err(plugin_error)
            }
        },
        || prepare_for_install(&app),
        |bytes| update.install(bytes).map_err(plugin_error),
    )
    .await;
    state.installing.store(false, Ordering::SeqCst);
    if result.is_err() {
        eprintln!("xpass: update install failed");
    }
    result
}

/// Opens the release page of the last check in the default browser.
#[tauri::command]
pub async fn open_release_page(state: tauri::State<'_, UpdateState>) -> AppResult<()> {
    let info = state.info().ok_or(AppError::UpdateFailed)?;
    let url = release_url(&info.url).ok_or(AppError::UpdateFailed)?;
    tauri::async_runtime::spawn_blocking(move || shell::open_url(&url))
        .await
        .map_err(|_| AppError::Internal)?
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    const DL: &str = "https://github.com/bs135/xpass/releases/download/v0.7.0/xpass_0.7.0_x64-setup.exe";

    #[test]
    fn newer_release_is_accepted_with_page_notes_and_date() {
        let info = evaluate("0.6.1", "0.7.0", &url(DL), Some("  notes  "), Some(5), None, 1).unwrap();
        assert!(info.newer);
        assert_eq!(info.latest, "0.7.0");
        assert_eq!(info.current, "0.6.1");
        assert_eq!(info.url, "https://github.com/bs135/xpass/releases/tag/v0.7.0");
        assert_eq!(info.notes.as_deref(), Some("notes"));
        assert_eq!(info.date, Some(5));
        assert!(evaluate(
            "0.9.0",
            "0.10.0",
            &url(&DL.replace("0.7.0", "0.10.0")),
            None,
            None,
            None,
            1
        )
        .is_ok());
    }

    #[test]
    fn equal_older_and_prerelease_versions_are_refused() {
        for (current, version) in [
            ("0.7.0", "0.7.0"),
            ("0.8.0", "0.7.0"),
            ("0.6.1", "0.7.0-rc1"),
            ("0.6.1", "0.7"),
        ] {
            let dl = url(&DL.replace("0.7.0", version));
            assert_eq!(
                evaluate(current, version, &dl, None, None, None, 1),
                Err(AppError::UpdateSignature),
                "{current} -> {version}"
            );
        }
    }

    #[test]
    fn only_our_release_assets_for_that_version_are_downloaded() {
        assert!(download_allowed(&url(DL), "0.7.0", None));
        for bad in [
            "https://github.com/bs135/xpass/releases/download/v0.6.0/xpass_0.6.0_x64-setup.exe",
            "https://github.com/evil/xpass/releases/download/v0.7.0/xpass_0.7.0_x64-setup.exe",
            "http://github.com/bs135/xpass/releases/download/v0.7.0/xpass_0.7.0_x64-setup.exe",
            "https://github.com.evil.example/bs135/xpass/releases/download/v0.7.0/x.exe",
            "https://github.com/bs135/xpass/releases/download/v0.7.0/",
            "https://github.com/bs135/xpass/releases/download/v0.7.0/sub/x.exe",
            "https://github.com/bs135/xpass/releases/download/v0.7.0/x.exe?x=1",
            "https://user@github.com/bs135/xpass/releases/download/v0.7.0/x.exe",
            "http://localhost:8080/xpass_0.7.0_x64-setup.exe",
        ] {
            assert!(!download_allowed(&url(bad), "0.7.0", None), "{bad}");
            assert_eq!(
                evaluate("0.6.1", "0.7.0", &url(bad), None, None, None, 1),
                Err(AppError::UpdateSignature),
                "{bad}"
            );
        }
    }

    #[test]
    fn test_origin_allows_only_the_test_server() {
        let origin = url("http://localhost:8080/latest.json");
        assert!(download_allowed(
            &url("http://localhost:8080/x.exe"),
            "0.7.0",
            Some(&origin)
        ));
        assert!(!download_allowed(
            &url("http://localhost:9090/x.exe"),
            "0.7.0",
            Some(&origin)
        ));
        assert!(!download_allowed(
            &url("http://127.0.0.1:8080/x.exe"),
            "0.7.0",
            Some(&origin)
        ));
    }

    #[test]
    fn notes_are_trimmed_and_capped() {
        assert_eq!(cap_notes(None), None);
        assert_eq!(cap_notes(Some("   ")), None);
        let long = "é".repeat(MAX_NOTES + 10);
        assert_eq!(cap_notes(Some(&long)).unwrap().chars().count(), MAX_NOTES);
    }

    #[test]
    fn up_to_date_points_to_the_latest_release_page() {
        let info = up_to_date("0.7.0", 3);
        assert!(!info.newer);
        assert_eq!(info.latest, "0.7.0");
        assert!(release_url(&info.url).is_some());
    }

    #[test]
    fn only_our_release_pages_are_opened() {
        assert!(release_url("https://github.com/bs135/xpass/releases/tag/v0.7.0").is_some());
        for bad in [
            "https://github.com/bs135/xpass/issues/1",
            "https://github.com/evil/xpass/releases/tag/v1.0.0",
            "http://github.com/bs135/xpass/releases/tag/v1.0.0",
            "https://github.com/bs135/xpass/releases/tag/v1?x=1",
            "javascript:alert(1)",
        ] {
            assert!(release_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn signature_errors_map_to_update_signature() {
        use tauri_plugin_updater::Error as E;
        assert_eq!(plugin_error(E::MissingSignedVersion), AppError::UpdateSignature);
        assert_eq!(plugin_error(E::InvalidUpdaterFormat), AppError::UpdateSignature);
        assert_eq!(plugin_error(E::ReleaseNotFound), AppError::UpdateFailed);
        assert_eq!(plugin_error(E::Network("x".into())), AppError::UpdateFailed);
    }

    type Log = Arc<StdMutex<Vec<&'static str>>>;

    fn install_with(download_ok: bool) -> (AppResult<()>, Vec<&'static str>) {
        let log: Log = Arc::default();
        let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
        let result = tauri::async_runtime::block_on(run_install(
            move || async move {
                l1.lock().unwrap().push("download");
                if download_ok {
                    Ok(vec![1, 2, 3])
                } else {
                    Err(AppError::UpdateSignature)
                }
            },
            move || async move { l2.lock().unwrap().push("lock+push") },
            move |bytes| {
                assert_eq!(bytes, [1, 2, 3]);
                l3.lock().unwrap().push("install");
                Ok(())
            },
        ));
        let log = log.lock().unwrap().clone();
        (result, log)
    }

    #[test]
    fn install_locks_and_pushes_after_a_verified_download_and_before_the_installer() {
        let (result, log) = install_with(true);
        assert_eq!(result, Ok(()));
        assert_eq!(log, ["download", "lock+push", "install"]);
    }

    #[test]
    fn a_failed_or_unverified_download_changes_nothing() {
        let (result, log) = install_with(false);
        assert_eq!(result, Err(AppError::UpdateSignature));
        assert_eq!(log, ["download"], "no lock, no installer");
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

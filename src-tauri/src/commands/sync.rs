//! Sync setup commands (T19). The token arrives here once, is moved into a
//! `Zeroizing` buffer and stored in Windows Credential Manager; no command returns
//! it. All GitHub requests are made from Rust (the WebView has no network access).

use tauri::State;
use url::Url;
use zeroize::Zeroizing;

use crate::{
    error::{AppError, AppResult},
    github::ConnectionReport,
    settings::SyncSettings,
    shell,
    state::{AppState, SyncConfig},
};

/// GitHub page for creating a fine-grained personal access token.
pub const TOKEN_PAGE_URL: &str = "https://github.com/settings/personal-access-tokens/new";

#[tauri::command]
pub fn sync_get_config(state: State<'_, AppState>) -> AppResult<SyncConfig> {
    state.sync_config()
}

/// Saves repository, branch (empty = default) and path (empty = default).
#[tauri::command]
pub fn sync_configure(state: State<'_, AppState>, repo: String, branch: String, path: String) -> AppResult<SyncConfig> {
    state.configure_sync(SyncSettings::from_input(&repo, &branch, &path)?)
}

#[tauri::command]
pub fn sync_set_token(state: State<'_, AppState>, token: String) -> AppResult<SyncConfig> {
    let token = Zeroizing::new(token);
    state.set_sync_token(&token)
}

/// Deletes the token and the sync setup; the local vault stays.
#[tauri::command]
pub fn sync_disconnect(state: State<'_, AppState>) -> AppResult<SyncConfig> {
    state.disconnect_sync()
}

/// Tests the values in the form. `token` is the one typed in the form, or `None`
/// to use the stored token.
#[tauri::command]
pub async fn sync_test_connection(
    state: State<'_, AppState>,
    repo: String,
    branch: String,
    path: String,
    token: Option<String>,
) -> AppResult<ConnectionReport> {
    let token = token.map(Zeroizing::new);
    let target = SyncSettings::from_input(&repo, &branch, &path)?;
    let state = state.inner().clone();
    state.test_sync_connection(target, token).await
}

/// Opens GitHub's "new fine-grained token" page in the default browser.
#[tauri::command]
pub async fn sync_open_token_page() -> AppResult<()> {
    let url = Url::parse(TOKEN_PAGE_URL).map_err(|_| AppError::Internal)?;
    tauri::async_runtime::spawn_blocking(move || shell::open_url(&url))
        .await
        .map_err(|_| AppError::Internal)?
}

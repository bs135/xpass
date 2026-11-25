//! Sync setup commands (T19). The token arrives here once, is moved into a
//! `Zeroizing` buffer and stored in Windows Credential Manager; no command returns
//! it. All GitHub requests are made from Rust (the WebView has no network access).

use std::future::Future;

use tauri::State;

use super::session::blocking;
use url::Url;
use zeroize::Zeroizing;

use crate::{
    error::{AppError, AppResult},
    github::ConnectionReport,
    history::{HistoryPreview, RewriteReport},
    settings::SyncSettings,
    shell,
    state::{AppState, Auth, SyncConfig, SyncStatus, VaultStatus},
    sync_worker::Trigger,
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

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    state.sync_status()
}

/// Starts a sync now ("Sync now"). The result arrives as `sync://status`.
#[tauri::command]
pub fn sync_now(state: State<'_, AppState>) -> AppResult<SyncStatus> {
    let config = state.sync_config()?;
    if !config.enabled || !config.has_token {
        return Err(AppError::SyncNotConfigured);
    }
    if state.status() != VaultStatus::Unlocked {
        return Err(AppError::Locked);
    }
    state.request_sync(Trigger::Manual);
    Ok(state.sync_status())
}

/// Hides the "password changed on another device" notice.
#[tauri::command]
pub fn sync_dismiss_password_changed(state: State<'_, AppState>) {
    state.dismiss_password_changed();
}

/// Opens GitHub's "new fine-grained token" page in the default browser.
#[tauri::command]
pub async fn sync_open_token_page() -> AppResult<()> {
    let url = Url::parse(TOKEN_PAGE_URL).map_err(|_| AppError::Internal)?;
    tauri::async_runtime::spawn_blocking(move || shell::open_url(&url))
        .await
        .map_err(|_| AppError::Internal)?
}

/// Runs a sync flow that also runs Argon2 on a blocking thread (inside the async
/// runtime, so GitHub requests still work), keeping the IPC workers free.
async fn off_ipc<T: Send + 'static>(f: impl Future<Output = AppResult<T>> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || tauri::async_runtime::block_on(f))
        .await
        .map_err(|_| AppError::Internal)?
}

/// "Use my vault from GitHub" (no vault on this PC yet): downloads the vault,
/// opens it with the master password and turns sync on with `token`.
#[tauri::command]
pub async fn sync_join(
    state: State<'_, AppState>,
    repo: String,
    branch: String,
    path: String,
    token: String,
    password: String,
) -> AppResult<()> {
    let token = Zeroizing::new(token);
    let password = Zeroizing::new(password);
    let target = SyncSettings::from_input(&repo, &branch, &path)?;
    let state = state.inner().clone();
    let store = state.github_store(&target, Some(&token))?;
    off_ipc(async move { state.join_vault(&store, target, &token, &password).await }).await
}

/// *Needs password*: the vault key was changed on another device (S7).
#[tauri::command]
pub async fn sync_resolve_key_change(state: State<'_, AppState>, password: String) -> AppResult<SyncStatus> {
    let password = Zeroizing::new(password);
    let state = state.inner().clone();
    let store = state.sync_store()?.ok_or(AppError::SyncNotConfigured)?;
    off_ipc(async move { state.resolve_key_change(&store, &password).await }).await
}

/// *Different vault*: replaces this PC's vault with the one on GitHub (the current
/// file is kept as a backup). `password` is the master password of that vault.
#[tauri::command]
pub async fn sync_replace_local(state: State<'_, AppState>, password: String) -> AppResult<()> {
    let password = Zeroizing::new(password);
    let state = state.inner().clone();
    let store = state.sync_store()?.ok_or(AppError::SyncNotConfigured)?;
    off_ipc(async move { state.replace_with_remote(&store, &password).await }).await
}

/// "Remove old versions from GitHub": what the dialog shows before the user confirms.
#[tauri::command]
pub async fn sync_history_preview(state: State<'_, AppState>) -> AppResult<HistoryPreview> {
    let state = state.inner().clone();
    state.check_history_rewrite()?;
    let store = state.sync_store()?.ok_or(AppError::SyncNotConfigured)?;
    state.preview_history(&store).await
}

/// Replaces the sync branch with one commit holding the current files (U10), after
/// re-authentication (password or Windows Hello).
#[tauri::command]
pub async fn sync_rewrite_history(state: State<'_, AppState>, auth: Auth) -> AppResult<RewriteReport> {
    let state = state.inner().clone();
    state.check_history_rewrite()?;
    let store = state.sync_store()?.ok_or(AppError::SyncNotConfigured)?;
    {
        let state = state.clone();
        blocking(move || state.reauthorize(&auth)).await?;
    }
    state.rewrite_history(&store).await
}

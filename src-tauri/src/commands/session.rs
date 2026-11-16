//! Session and security commands: thin wrappers over [`AppState`]. Argon2 runs on a blocking
//! thread so the IPC worker and the UI stay responsive.
//!
//! Secrets arriving over IPC are moved into `Zeroizing` buffers right away; copies
//! made by the IPC layer itself are outside our control.

use serde::Serialize;
use tauri::{AppHandle, State};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{
    error::{AppError, AppResult},
    lock_watch::{self, LockReason},
    state::{AppState, SessionInfo},
    strength::{self, Strength},
};

#[derive(Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryKit {
    /// Printable recovery key for the Emergency Kit. Shown once, never stored.
    pub recovery_key: String,
}

#[tauri::command]
pub fn vault_status(state: State<'_, AppState>) -> SessionInfo {
    state.session_info()
}

#[tauri::command]
pub async fn create_vault(state: State<'_, AppState>, password: String) -> AppResult<RecoveryKit> {
    let password = Zeroizing::new(password);
    let state = state.inner().clone();
    let recovery_key = blocking(move || state.create_vault(&password)).await?;
    Ok(RecoveryKit {
        recovery_key: recovery_key.to_string(),
    })
}

#[tauri::command]
pub async fn unlock_password(state: State<'_, AppState>, password: String) -> AppResult<()> {
    let password = Zeroizing::new(password);
    let state = state.inner().clone();
    blocking(move || state.unlock_password(&password)).await
}

#[tauri::command]
pub async fn unlock_recovery(state: State<'_, AppState>, recovery_key: String) -> AppResult<()> {
    let recovery_key = Zeroizing::new(recovery_key);
    let state = state.inner().clone();
    blocking(move || state.unlock_recovery(&recovery_key)).await
}

/// `current_password` may be omitted right after a recovery-key unlock.
#[tauri::command]
pub async fn change_password(
    state: State<'_, AppState>,
    current_password: Option<String>,
    new_password: String,
) -> AppResult<()> {
    let current = current_password.map(Zeroizing::new);
    let new = Zeroizing::new(new_password);
    let state = state.inner().clone();
    blocking(move || state.change_password(current.as_deref().map(String::as_str), &new)).await
}

/// New Emergency Kit: replaces the recovery key. `current_password` may be omitted
/// right after a recovery-key unlock.
#[tauri::command]
pub async fn new_recovery_key(state: State<'_, AppState>, current_password: Option<String>) -> AppResult<RecoveryKit> {
    let current = current_password.map(Zeroizing::new);
    let state = state.inner().clone();
    let recovery_key = blocking(move || state.new_recovery_key(current.as_deref().map(String::as_str))).await?;
    Ok(RecoveryKit {
        recovery_key: recovery_key.to_string(),
    })
}

#[tauri::command]
pub fn password_strength(password: String) -> Strength {
    let password = Zeroizing::new(password);
    strength::estimate(&password)
}

/// Locks now; emits `vault://locked` with reason `manual`.
#[tauri::command]
pub fn lock(app: AppHandle) {
    lock_watch::lock_app(&app, LockReason::Manual);
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::Internal)?
}

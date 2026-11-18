//! Restore a previous version (T22): list the local backups and restore one as a
//! new edit. The password (only for a backup from before a key rotation) is moved
//! into a `Zeroizing` buffer and Argon2 runs on a blocking thread.

use tauri::State;
use zeroize::Zeroizing;

use super::session::blocking;
use crate::{
    error::AppResult,
    state::{AppState, BackupInfo},
};

#[tauri::command]
pub fn list_backups(state: State<'_, AppState>) -> AppResult<Vec<BackupInfo>> {
    state.list_backups()
}

#[tauri::command]
pub async fn restore_backup(state: State<'_, AppState>, name: String, password: Option<String>) -> AppResult<()> {
    let password = password.map(Zeroizing::new);
    let state = state.inner().clone();
    blocking(move || state.restore_backup(&name, password.as_deref().map(String::as_str))).await
}

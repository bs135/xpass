//! Windows Hello commands. Hello calls block until the user answers the dialog, so
//! they run on a blocking thread. Only status codes cross IPC: the challenge, the
//! signature and the derived key stay in Rust.

use tauri::State;
use zeroize::Zeroizing;

use super::session::blocking;
use crate::{
    error::AppResult,
    state::{AppState, HelloStatus},
};

/// Whether Hello is usable on this PC, plus the vault's Hello state.
#[tauri::command]
pub async fn hello_status(state: State<'_, AppState>) -> AppResult<HelloStatus> {
    let state = state.inner().clone();
    blocking(move || Ok(state.hello_status())).await
}

#[tauri::command]
pub async fn unlock_hello(state: State<'_, AppState>) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.unlock_hello()).await
}

/// `current_password` may be omitted right after a recovery-key unlock.
#[tauri::command]
pub async fn hello_enable(state: State<'_, AppState>, current_password: Option<String>) -> AppResult<()> {
    let current = current_password.map(Zeroizing::new);
    let state = state.inner().clone();
    blocking(move || state.enable_hello(current.as_deref().map(String::as_str))).await
}

#[tauri::command]
pub async fn hello_disable(state: State<'_, AppState>) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.disable_hello()).await
}

#[tauri::command]
pub fn hello_dismiss_reenroll(state: State<'_, AppState>) {
    state.dismiss_hello_reenroll();
}

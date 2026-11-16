//! Tools that need no unlocked vault.

use tauri::State;
use zeroize::Zeroizing;

use crate::{
    error::AppResult,
    generator::{self, Generated, Options},
    state::AppState,
};

#[tauri::command]
pub fn generate_password(options: Options) -> AppResult<Generated> {
    generator::generate(&options)
}

/// Copies text such as a generated password with the same protections and timeout
/// as `copy_field`.
#[tauri::command]
pub fn copy_text(state: State<'_, AppState>, text: String) -> AppResult<()> {
    let text = Zeroizing::new(text);
    state.copy_text(&text)
}

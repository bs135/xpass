//! Tools that need no unlocked vault.

use tauri::State;
use zeroize::Zeroizing;

use crate::{
    entries::{self, TotpPreview},
    error::AppResult,
    generator::{self, Generated, Options},
    state::{now_ms, AppState},
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

/// Checks a pasted one-time password secret (`otpauth://` URI or base32) and shows
/// its current code before it is added to an entry.
#[tauri::command]
pub fn totp_preview(input: String, issuer: String, account: String) -> AppResult<TotpPreview> {
    let input = Zeroizing::new(input);
    entries::totp_preview(&input, &issuer, &account, now_ms())
}

//! Vault export command (v0.5 plan I7–I10). The UI has already shown its warning;
//! here: export password check → re-authentication (password or Windows Hello) →
//! save dialog → write. The path stays in Rust; the UI gets the file name and the
//! number of entries.

use tauri::{State, WebviewWindow};
use zeroize::Zeroizing;

use super::session::blocking;
use crate::{
    error::{AppError, AppResult},
    file_dialog::{self, Request},
    state::{now_ms, suggested_file_name, AppState, Auth, ExportFormat, ExportResult, VaultStatus},
};

/// `None`: the save dialog was canceled (nothing written).
#[tauri::command]
pub async fn export_vault(
    window: WebviewWindow,
    state: State<'_, AppState>,
    format: ExportFormat,
    auth: Auth,
    kdbx_password: Option<String>,
) -> AppResult<Option<ExportResult>> {
    let kdbx_password = kdbx_password.map(Zeroizing::new);
    if state.status() != VaultStatus::Unlocked {
        return Err(AppError::Locked);
    }
    AppState::check_export_password(format, kdbx_password.as_deref().map(String::as_str))?;
    let owner = file_dialog::owner_of(&window);
    let state = state.inner().clone();
    blocking(move || {
        state.reauthorize(&auth)?;
        let filter = match format {
            ExportFormat::Csv => file_dialog::CSV,
            ExportFormat::Kdbx => file_dialog::KDBX,
        };
        let request = Request::Save {
            file_name: suggested_file_name(format, now_ms()),
            extension: format.extension(),
            filters: vec![filter],
        };
        let Some(path) = file_dialog::show(owner, request)? else {
            return Ok(None);
        };
        state
            .export_to(&path, format, kdbx_password.as_deref().map(String::as_str))
            .map(Some)
    })
    .await
}

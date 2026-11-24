//! CSV import commands (v0.5 plan, IPC table). The file path comes from the native
//! open dialog and stays in Rust; the frontend gets an [`ImportPreview`] without
//! secrets and, after the commit, only counts.

use tauri::{State, WebviewWindow};
use xpass_interop::{
    apply::{ImportOptions, ImportSummary},
    preset::{Mapping, Preset},
};

use super::session::blocking;
use crate::{
    error::{AppError, AppResult},
    file_dialog::{self, Request},
    state::{AppState, ImportPreview, VaultStatus},
};

/// Shows the open dialog, then reads and parses the chosen file. `None`: the user
/// canceled the dialog. `preset` overrides the detected one.
#[tauri::command]
pub async fn import_open(
    window: WebviewWindow,
    state: State<'_, AppState>,
    preset: Option<Preset>,
) -> AppResult<Option<ImportPreview>> {
    if state.status() != VaultStatus::Unlocked {
        return Err(AppError::Locked);
    }
    let owner = file_dialog::owner_of(&window);
    let state = state.inner().clone();
    blocking(move || {
        let request = Request::Open {
            filters: vec![file_dialog::CSV, file_dialog::ALL],
        };
        match file_dialog::show(owner, request)? {
            Some(path) => state.import_file(path, preset).map(Some),
            None => Ok(None),
        }
    })
    .await
}

#[tauri::command]
pub fn import_set_mapping(
    state: State<'_, AppState>,
    preset: Preset,
    mapping: Option<Mapping>,
) -> AppResult<ImportPreview> {
    state.import_set_mapping(preset, mapping)
}

#[tauri::command]
pub fn import_commit(state: State<'_, AppState>, options: ImportOptions) -> AppResult<ImportSummary> {
    state.import_commit(&options)
}

#[tauri::command]
pub fn import_cancel(state: State<'_, AppState>) {
    state.import_cancel();
}

#[tauri::command]
pub fn import_delete_source(state: State<'_, AppState>) -> AppResult<()> {
    state.import_delete_source()
}

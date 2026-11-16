//! Entry commands (PLAN.md §3.5). Metadata and details never include secrets.

use serde::Serialize;
use tauri::State;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    entries::{self, EntryDetail, EntryInput, EntrySummary, FieldRef},
    error::{AppError, AppResult},
    shell,
    state::AppState,
};

#[tauri::command]
pub fn list_entries(state: State<'_, AppState>) -> AppResult<Vec<EntrySummary>> {
    state.read(|p| Ok(entries::list(p)))
}

#[tauri::command]
pub fn get_entry(state: State<'_, AppState>, id: Uuid) -> AppResult<EntryDetail> {
    state.read(|p| entries::detail(p, id))
}

/// Creates (`input.id` null) or updates an entry; returns its id.
#[tauri::command]
pub fn save_entry(state: State<'_, AppState>, input: EntryInput) -> AppResult<Uuid> {
    state.edit(|p, ts| entries::save(p, &input, ts))
}

#[tauri::command]
pub fn delete_entry(state: State<'_, AppState>, id: Uuid) -> AppResult<()> {
    state.edit(|p, ts| entries::delete(p, id, ts))
}

#[tauri::command]
pub fn set_favorite(state: State<'_, AppState>, id: Uuid, favorite: bool) -> AppResult<()> {
    state.edit(|p, ts| entries::set_favorite(p, id, favorite, ts))
}

/// Opens the entry's URL at `index` in the default browser (http/https only).
#[tauri::command]
pub async fn open_url(state: State<'_, AppState>, id: Uuid, index: usize) -> AppResult<()> {
    let url = state.read(|p| entries::url_to_open(p, id, index))?;
    tauri::async_runtime::spawn_blocking(move || shell::open_url(&url))
        .await
        .map_err(|_| AppError::Internal)?
}

/// Copies a field to the clipboard (excluded from history and cloud); cleared after
/// the timeout, on lock and on exit unless the user copied something else.
#[tauri::command]
pub fn copy_field(state: State<'_, AppState>, id: Uuid, field: FieldRef) -> AppResult<()> {
    state.copy_field(id, field)
}

#[derive(Serialize, Zeroize, ZeroizeOnDrop)]
pub struct Secret {
    pub value: String,
}

/// Returns the password or a secret custom field; the UI hides it again after 15 s.
#[tauri::command]
pub fn reveal_secret(state: State<'_, AppState>, id: Uuid, field: FieldRef) -> AppResult<Secret> {
    let value = state.reveal_secret(id, field)?;
    Ok(Secret {
        value: value.to_string(),
    })
}

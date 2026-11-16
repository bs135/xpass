//! Settings commands (`settings.json`, no secrets).

use tauri::State;

use crate::{error::AppResult, settings::Settings, state::AppState};

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Validates, saves and applies the settings; returns what was stored.
#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, settings: Settings) -> AppResult<Settings> {
    state.update_settings(settings)
}

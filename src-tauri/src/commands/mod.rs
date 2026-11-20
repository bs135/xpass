pub mod backups;
pub mod entries;
pub mod hello;
pub mod import;
pub mod session;
pub mod settings;
pub mod sync;
pub mod tools;

use serde::Serialize;
use url::Url;

use crate::{
    error::{AppError, AppResult},
    shell,
};

/// Project page shown in About.
pub const PROJECT_URL: &str = "https://github.com/bs135/xpass";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub format_version: u16,
}

/// Basic app metadata; also serves as the IPC smoke test for the frontend.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        format_version: xpass_core::FORMAT_VERSION,
    }
}

/// Opens the project page (About) in the default browser.
#[tauri::command]
pub async fn open_project_page() -> AppResult<()> {
    let url = Url::parse(PROJECT_URL).map_err(|_| AppError::Internal)?;
    tauri::async_runtime::spawn_blocking(move || shell::open_url(&url))
        .await
        .map_err(|_| AppError::Internal)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_versions() {
        let info = app_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.format_version, 1);
    }
}

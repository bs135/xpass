//! On-disk locations. Everything lives in the app data directory
//! (`%APPDATA%\com.bs135.xpass` on Windows).

use std::path::{Path, PathBuf};

use tauri::Manager;

#[derive(Debug, Clone)]
pub struct Paths {
    dir: PathBuf,
}

impl Paths {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn from_app(app: &tauri::AppHandle) -> tauri::Result<Self> {
        Ok(Self::new(app.path().app_data_dir()?))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The encrypted vault file.
    pub fn vault(&self) -> PathBuf {
        self.dir.join("vault.xpv")
    }

    /// Copies of previous vault files (see `storage::BACKUPS_KEPT`).
    pub fn backups(&self) -> PathBuf {
        self.dir.join("backups")
    }
}

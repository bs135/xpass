//! App settings in `settings.json` next to the vault. No secrets live here.
//!
//! A missing or unreadable file falls back to defaults (and is rewritten on the next
//! change), so a damaged settings file never blocks access to the vault.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Inactivity timeouts offered in the UI (minutes); `None` means never.
pub const AUTO_LOCK_CHOICES: [u32; 5] = [1, 5, 15, 30, 60];
/// Allowed clipboard timeout range (seconds).
pub const CLIPBOARD_SECONDS_RANGE: std::ops::RangeInclusive<u32> = 5..=300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    En,
    Vi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    /// `None` follows the OS language.
    pub language: Option<Language>,
    /// Lock after this many minutes without input; `None` = never.
    pub auto_lock_minutes: Option<u32>,
    pub clipboard_clear_seconds: u32,
    /// The close button hides xpass to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Lock when the window is minimized or hidden to the tray.
    pub lock_on_minimize: bool,
    /// When the current Emergency Kit was generated (Unix ms); set by the app only.
    pub last_kit_at: Option<i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            language: None,
            auto_lock_minutes: Some(5),
            clipboard_clear_seconds: 20,
            close_to_tray: true,
            lock_on_minimize: false,
            last_kit_at: None,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> AppResult<()> {
        if self.auto_lock_minutes.is_some_and(|m| !AUTO_LOCK_CHOICES.contains(&m)) {
            return Err(AppError::InvalidInput {
                field: "autoLockMinutes",
            });
        }
        if !CLIPBOARD_SECONDS_RANGE.contains(&self.clipboard_clear_seconds) {
            return Err(AppError::InvalidInput {
                field: "clipboardClearSeconds",
            });
        }
        Ok(())
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        self.auto_lock_minutes.map(|m| Duration::from_secs(u64::from(m) * 60))
    }

    pub fn clipboard_clear(&self) -> Duration {
        Duration::from_secs(u64::from(self.clipboard_clear_seconds))
    }
}

/// Loads settings; defaults if the file is missing, unreadable or invalid.
pub fn load(path: &Path) -> Settings {
    let Ok(bytes) = fs::read(path) else {
        return Settings::default();
    };
    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(s) if s.validate().is_ok() => s,
        _ => {
            eprintln!("xpass: settings.json is invalid; using defaults");
            Settings::default()
        }
    }
}

/// Writes settings atomically (temp file + rename).
pub fn save(path: &Path, settings: &Settings) -> AppResult<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    {
        let mut f = File::create(&tmp)?;
        let json = serde_json::to_vec_pretty(settings).map_err(|_| AppError::Internal)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        assert_eq!(load(&path), Settings::default(), "missing file");

        let s = Settings {
            theme: Theme::Dark,
            language: Some(Language::Vi),
            auto_lock_minutes: None,
            clipboard_clear_seconds: 45,
            close_to_tray: false,
            lock_on_minimize: true,
            last_kit_at: Some(123),
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["autoLockMinutes"], serde_json::Value::Null);
        assert_eq!(json["language"], "vi");
    }

    #[test]
    fn partial_or_broken_files_fall_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(&path, r#"{"theme":"light"}"#).unwrap();
        let s = load(&path);
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.auto_lock_minutes, Some(5), "missing fields use defaults");

        fs::write(&path, b"{not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        fs::write(&path, r#"{"clipboardClearSeconds": 1}"#).unwrap();
        assert_eq!(load(&path), Settings::default(), "out-of-range values are rejected");
    }

    #[test]
    fn validation() {
        let ok = Settings::default();
        assert!(ok.validate().is_ok());
        assert!(Settings {
            auto_lock_minutes: Some(7),
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert!(Settings {
            auto_lock_minutes: None,
            ..ok.clone()
        }
        .validate()
        .is_ok());
        assert!(Settings {
            clipboard_clear_seconds: 301,
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert_eq!(ok.idle_timeout(), Some(Duration::from_secs(300)));
        assert_eq!(ok.clipboard_clear(), Duration::from_secs(20));
    }
}

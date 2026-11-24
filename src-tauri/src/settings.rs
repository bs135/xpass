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
    /// Ask GitHub for a newer release at startup, at most once a day (v0.6 H9).
    /// Off by default: xpass makes no network request unless asked to.
    pub check_updates_on_startup: bool,
    /// When the last update check ran (Unix ms); set by the app only.
    pub last_update_check_at: Option<i64>,
    /// GitHub sync target; changed only through the sync commands. The token is
    /// in Windows Credential Manager, never here.
    pub sync: SyncSettings,
}

/// Default path of the vault file in the repository.
pub const DEFAULT_SYNC_PATH: &str = "xpass/vault.xpv";

/// Where the vault is synced (S10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncSettings {
    /// `owner/name`; empty = sync not set up.
    pub repo: String,
    /// Empty = the repository's default branch.
    pub branch: String,
    /// Path of the vault file in the repository.
    pub path: String,
    /// Sync runs (repo configured and a token stored).
    pub enabled: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            repo: String::new(),
            branch: String::new(),
            path: DEFAULT_SYNC_PATH.to_owned(),
            enabled: false,
        }
    }
}

impl SyncSettings {
    /// Normalizes user input: `owner/name` or a `https://github.com/owner/name` URL,
    /// branch and path trimmed (empty path: default). Errors name the bad field.
    pub fn from_input(repo: &str, branch: &str, path: &str) -> AppResult<Self> {
        let out = Self {
            repo: parse_repo(repo)?,
            branch: branch.trim().to_owned(),
            path: match path.trim().trim_matches('/') {
                "" => DEFAULT_SYNC_PATH.to_owned(),
                p => p.to_owned(),
            },
            enabled: false,
        };
        out.validate()?;
        Ok(out)
    }

    pub fn is_configured(&self) -> bool {
        !self.repo.is_empty()
    }

    /// Identity of the remote file; a change resets the sync state (S3).
    pub fn remote_id(&self) -> String {
        format!("{}@{}:{}", self.repo.to_ascii_lowercase(), self.branch, self.path)
    }

    pub fn validate(&self) -> AppResult<()> {
        if !self.repo.is_empty() && parse_repo(&self.repo).as_deref() != Ok(self.repo.as_str()) {
            return Err(AppError::InvalidInput { field: "syncRepo" });
        }
        if !self.branch.is_empty() && !valid_branch(&self.branch) {
            return Err(AppError::InvalidInput { field: "syncBranch" });
        }
        if !valid_path(&self.path) {
            return Err(AppError::InvalidInput { field: "syncPath" });
        }
        if self.enabled && self.repo.is_empty() {
            return Err(AppError::InvalidInput { field: "syncRepo" });
        }
        Ok(())
    }
}

/// `owner/name` from `owner/name`, `github.com/owner/name` or
/// `https://github.com/owner/name[.git][/]`.
pub fn parse_repo(input: &str) -> AppResult<String> {
    let bad = || AppError::InvalidInput { field: "syncRepo" };
    let s = input.trim();
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    let (owner, name) = s.split_once('/').ok_or_else(bad)?;
    let owner_ok = (1..=39).contains(&owner.len())
        && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-');
    let name_ok = (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && name != "."
        && name != "..";
    if owner_ok && name_ok {
        Ok(format!("{owner}/{name}"))
    } else {
        Err(bad())
    }
}

/// A reasonable subset of git's ref-name rules.
fn valid_branch(b: &str) -> bool {
    b.len() <= 255
        && !b.starts_with(['-', '/'])
        && !b.ends_with(['/', '.'])
        && !b.ends_with(".lock")
        && !b.contains("..")
        && !b.contains("//")
        && !b.contains("@{")
        && b.chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
}

/// Relative file path with `/` separators, no empty, `.` or `..` segments.
fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 255
        && p.split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != ".." && !seg.ends_with('.'))
        && p.chars().all(|c| !c.is_control() && c != '\\' && c != ':')
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
            check_updates_on_startup: false,
            last_update_check_at: None,
            sync: SyncSettings::default(),
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
        self.sync.validate()
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
    write_json_atomic(path, settings)
}

/// Writes `value` as pretty JSON to a temp file, flushes it and renames it over `path`.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    {
        let mut f = File::create(&tmp)?;
        let json = serde_json::to_vec_pretty(value).map_err(|_| AppError::Internal)?;
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
            check_updates_on_startup: true,
            last_update_check_at: Some(456),
            sync: SyncSettings {
                repo: "octo/vault".into(),
                branch: "main".into(),
                path: "a/b.xpv".into(),
                enabled: true,
            },
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

#[cfg(test)]
mod sync_tests {
    use super::*;

    #[test]
    fn repo_input_forms() {
        for input in [
            "octo/vault",
            " octo/vault ",
            "github.com/octo/vault",
            "https://github.com/octo/vault",
            "https://www.github.com/octo/vault/",
            "https://github.com/octo/vault.git",
        ] {
            assert_eq!(parse_repo(input).unwrap(), "octo/vault", "{input:?}");
        }
        assert_eq!(parse_repo("my-org/my.repo_1").unwrap(), "my-org/my.repo_1");
        for bad in [
            "",
            "octo",
            "octo/",
            "/vault",
            "octo/vault/x",
            "-octo/vault",
            "octo/..",
            "oc to/vault",
            "octo/v@ult",
        ] {
            assert!(parse_repo(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn sync_settings_from_input() {
        let s = SyncSettings::from_input("https://github.com/octo/vault", " main ", "").unwrap();
        assert_eq!(
            (s.repo.as_str(), s.branch.as_str(), s.path.as_str()),
            ("octo/vault", "main", DEFAULT_SYNC_PATH)
        );
        assert_eq!(
            SyncSettings::from_input("octo/vault", "", "/dir/v.xpv/").unwrap().path,
            "dir/v.xpv"
        );
        let field = |r: AppResult<SyncSettings>| match r {
            Err(AppError::InvalidInput { field }) => field,
            other => panic!("{other:?}"),
        };
        assert_eq!(field(SyncSettings::from_input("nope", "", "")), "syncRepo");
        for branch in ["-x", "a..b", "a b", "a~1", "x.lock", "a//b"] {
            assert_eq!(
                field(SyncSettings::from_input("o/r", branch, "")),
                "syncBranch",
                "{branch:?}"
            );
        }
        assert!(SyncSettings::from_input("o/r", "feature/sync-1", "").is_ok());
        for path in ["a/../b", "a//b", "a\\b", "./a", "c:/x"] {
            assert_eq!(field(SyncSettings::from_input("o/r", "", path)), "syncPath", "{path:?}");
        }
    }

    #[test]
    fn remote_id_ignores_owner_case_only() {
        let a = SyncSettings::from_input("Octo/Vault", "", "").unwrap();
        let b = SyncSettings::from_input("octo/vault", "", "").unwrap();
        assert_eq!(a.remote_id(), b.remote_id());
        let c = SyncSettings::from_input("octo/vault", "dev", "").unwrap();
        assert_ne!(a.remote_id(), c.remote_id());
    }

    #[test]
    fn old_settings_files_get_sync_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(s.sync, SyncSettings::default());
        assert!(!s.sync.is_configured());
    }
}

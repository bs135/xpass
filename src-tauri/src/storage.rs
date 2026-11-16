//! Vault file I/O: atomic replace and rolling backups.
//!
//! Every write copies the current file to `backups/vault-<UTC yyyyMMdd-HHmmss-mmm>.xpv`
//! first, then writes the new bytes to a temp file, flushes it to disk and renames it
//! over `vault.xpv`. A crash at any point leaves either the old or the new file.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::paths::Paths;

/// Number of backups kept; older ones are deleted after each write.
pub const BACKUPS_KEPT: usize = 10;

const BACKUP_PREFIX: &str = "vault-";
const BACKUP_SUFFIX: &str = ".xpv";

#[derive(Debug, Clone)]
pub struct Storage {
    paths: Paths,
}

impl Storage {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn exists(&self) -> bool {
        self.paths.vault().is_file()
    }

    /// Reads the vault file; `None` when there is no vault yet.
    pub fn read(&self) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.paths.vault()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Backs up the current file (if any) and atomically replaces it with `bytes`.
    pub fn write(&self, bytes: &[u8], now_ms: i64) -> io::Result<()> {
        fs::create_dir_all(self.paths.dir())?;
        let vault = self.paths.vault();
        if vault.is_file() {
            self.backup(&vault, now_ms)?;
        }
        let tmp = vault.with_extension("xpv.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        // std::fs::rename replaces an existing destination on Windows (MOVEFILE_REPLACE_EXISTING).
        fs::rename(&tmp, &vault)
    }

    /// Backup files, oldest first.
    pub fn backups(&self) -> io::Result<Vec<PathBuf>> {
        let entries = match fs::read_dir(self.paths.backups()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(BACKUP_PREFIX) && n.ends_with(BACKUP_SUFFIX))
            })
            .collect();
        // The timestamp format sorts lexicographically in time order.
        files.sort();
        Ok(files)
    }

    fn backup(&self, vault: &Path, now_ms: i64) -> io::Result<()> {
        let dir = self.paths.backups();
        fs::create_dir_all(&dir)?;
        let name = format!("{BACKUP_PREFIX}{}{BACKUP_SUFFIX}", stamp(now_ms));
        fs::copy(vault, dir.join(name))?;
        let files = self.backups()?;
        let excess = files.len().saturating_sub(BACKUPS_KEPT);
        for old in &files[..excess] {
            fs::remove_file(old)?;
        }
        Ok(())
    }
}

/// UTC timestamp `yyyyMMdd-HHmmss-mmm` for backup names.
fn stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let tod = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (tod / 3600, tod % 3600 / 60, tod % 60);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}-{millis:03}")
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian calendar
/// (the well-known `civil_from_days` algorithm by Howard Hinnant).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> (tempfile::TempDir, Storage) {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(Paths::new(tmp.path().join("data")));
        (tmp, storage)
    }

    #[test]
    fn stamp_formats_utc() {
        assert_eq!(stamp(0), "19700101-000000-000");
        assert_eq!(stamp(1_700_000_000_123), "20231114-221320-123");
        assert_eq!(stamp(951_782_400_000), "20000229-000000-000", "leap day");
    }

    #[test]
    fn read_missing_vault_is_none() {
        let (_tmp, s) = storage();
        assert!(!s.exists());
        assert_eq!(s.read().unwrap(), None);
        assert!(s.backups().unwrap().is_empty());
    }

    #[test]
    fn write_replaces_atomically_and_backs_up_previous() {
        let (_tmp, s) = storage();
        s.write(b"one", 1_000).unwrap();
        assert!(s.backups().unwrap().is_empty(), "nothing to back up on first write");
        s.write(b"two", 2_000).unwrap();
        assert_eq!(s.read().unwrap().as_deref(), Some(&b"two"[..]));
        let backups = s.backups().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), b"one");
        assert!(
            !s.paths.vault().with_extension("xpv.tmp").exists(),
            "temp file renamed away"
        );
    }

    #[test]
    fn keeps_only_the_newest_backups() {
        let (_tmp, s) = storage();
        for i in 0..15 {
            s.write(format!("v{i}").as_bytes(), 1_000 * i).unwrap();
        }
        let backups = s.backups().unwrap();
        assert_eq!(backups.len(), BACKUPS_KEPT);
        assert_eq!(fs::read(backups.first().unwrap()).unwrap(), b"v4");
        assert_eq!(fs::read(backups.last().unwrap()).unwrap(), b"v13");
    }
}

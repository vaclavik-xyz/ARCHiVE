//! Open, decrypt, and read files from an on-disk iOS backup.

use std::io::Write;
use std::path::{Path, PathBuf};

use crabapple::error::BackupError as CrabError;
use crabapple::{Authentication, Backup as RawBackup};

/// Errors from opening or reading a backup.
#[derive(Debug)]
pub enum BackupError {
    /// The backup could not be opened or decrypted (wrong/missing password,
    /// corrupt manifest, …). Carries a human-readable reason.
    Open(String),
    /// The backup is encrypted and the password was missing or incorrect.
    Locked(String),
    /// An I/O error while materializing a decrypted file.
    Io(std::io::Error),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Open(why) => write!(f, "could not open backup: {why}"),
            BackupError::Locked(why) => write!(f, "backup is locked: {why}"),
            BackupError::Io(why) => write!(f, "backup I/O error: {why}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<std::io::Error> for BackupError {
    fn from(value: std::io::Error) -> Self {
        BackupError::Io(value)
    }
}

/// Device metadata read from the backup's lockdown record.
pub struct DeviceInfo {
    /// User-facing device name (e.g. "Jana's iPhone").
    pub device_name: String,
    /// iOS version string (e.g. "17.5").
    pub product_version: String,
    /// The backup's unique device identifier.
    pub udid: String,
}

/// Pick the crabapple authentication for a given optional password.
/// No (or empty) password → `None` (unencrypted backups); otherwise `Password`.
fn choose_auth(password: Option<&str>) -> Authentication {
    match password {
        Some(pw) if !pw.is_empty() => Authentication::Password(pw.to_string()),
        _ => Authentication::None,
    }
}

/// Classify a crabapple open failure: missing/incorrect password → `Locked`
/// (the caller can surface an auth error); anything else (bad path, corrupt
/// manifest, I/O) → `Open`.
fn map_open_err(why: CrabError) -> BackupError {
    match why {
        CrabError::PasswordOrKeyRequired | CrabError::PasswordOrKeyIncorrect => {
            BackupError::Locked(why.to_string())
        }
        other => BackupError::Open(other.to_string()),
    }
}

/// An opened (and, if needed, unlocked) iOS backup.
pub struct Backup {
    raw: RawBackup,
    info: DeviceInfo,
}

impl Backup {
    /// Open a backup directory. `password` is required for encrypted backups and
    /// ignored for unencrypted ones.
    ///
    /// crabapple rejects a password on an unencrypted backup (`NotEncrypted`), so
    /// we pick `Authentication::None` when no password is given and transparently
    /// retry without one if a supplied password turns out to be for an
    /// unencrypted backup. `Backup::open` accepts any `AsRef<Path>`, so the
    /// directory path is passed through directly.
    pub fn open(dir: &Path, password: Option<&str>) -> Result<Self, BackupError> {
        let auth = choose_auth(password);
        let raw = match RawBackup::open(dir, &auth) {
            Ok(raw) => raw,
            // A password was supplied but the backup is not encrypted: retry unauthenticated.
            Err(CrabError::NotEncrypted) => {
                RawBackup::open(dir, &Authentication::None).map_err(map_open_err)?
            }
            Err(why) => return Err(map_open_err(why)),
        };

        let lockdown = raw.lockdown();
        let info = DeviceInfo {
            device_name: lockdown.device_name.clone(),
            product_version: lockdown.product_version.clone(),
            udid: raw
                .udid()
                .map_err(|why| BackupError::Open(why.to_string()))?
                .to_string(),
        };
        Ok(Backup { raw, info })
    }

    /// Device metadata read from the backup's lockdown record.
    pub fn device_info(&self) -> &DeviceInfo {
        &self.info
    }

    /// Decrypt the file at `domain` + `relative_path` to `dest` and return its
    /// path, or `Ok(None)` when the backup contains no such file.
    pub fn fetch(
        &self,
        domain: &str,
        relative_path: &str,
        dest: &Path,
    ) -> Result<Option<PathBuf>, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let Some(entry) = entries
            .into_iter()
            .find(|e| e.domain == domain && e.relative_path == relative_path)
        else {
            return Ok(None);
        };
        // crabapple's `decrypt_entry` only works on encrypted backups (it returns
        // `NotEncrypted` otherwise). On an unencrypted backup the file already
        // sits in plaintext under `backup_path/<id[..2]>/<id>`, so read it directly.
        let bytes = if self.raw.is_encrypted() {
            self.raw
                .decrypt_entry(&entry)
                .map_err(|why| BackupError::Open(why.to_string()))?
        } else {
            std::fs::read(self.raw.backup_path.join(entry.source()))?
        };
        write_file(dest, &bytes)?;
        Ok(Some(dest.to_path_buf()))
    }
}

/// Write `bytes` to `dest`, creating parent directories as needed; returns `dest`.
fn write_file(dest: &Path, bytes: &[u8]) -> Result<PathBuf, BackupError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::File::create(dest)?.write_all(bytes)?;
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration test against a real backup. Set BACKUP_EXTRACTOR_TEST_BACKUP
    // to a backup directory (and BACKUP_EXTRACTOR_TEST_PASSWORD if encrypted).
    // Skipped when the env var is unset so CI stays green without fixtures.
    #[test]
    fn opens_real_backup_and_reads_device_info() {
        let Ok(dir) = std::env::var("BACKUP_EXTRACTOR_TEST_BACKUP") else {
            eprintln!("skipping: set BACKUP_EXTRACTOR_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("BACKUP_EXTRACTOR_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref())
            .expect("open backup");
        let info = backup.device_info();
        assert!(!info.product_version.is_empty(), "iOS version should be set");
    }

    #[test]
    fn error_display_is_readable() {
        let e = BackupError::Open("bad password".into());
        assert_eq!(e.to_string(), "could not open backup: bad password");
    }

    #[test]
    fn classifies_auth_and_other_open_errors() {
        assert!(matches!(map_open_err(CrabError::PasswordOrKeyRequired), BackupError::Locked(_)));
        assert!(matches!(map_open_err(CrabError::PasswordOrKeyIncorrect), BackupError::Locked(_)));
        assert!(matches!(map_open_err(CrabError::ManifestDbNotFound), BackupError::Open(_)));
    }

    #[test]
    fn choose_auth_none_without_password() {
        assert!(matches!(choose_auth(None), Authentication::None));
    }

    #[test]
    fn choose_auth_none_for_empty_password() {
        assert!(matches!(choose_auth(Some("")), Authentication::None));
    }

    #[test]
    fn choose_auth_password_when_present() {
        match choose_auth(Some("secret")) {
            Authentication::Password(p) => assert_eq!(p, "secret"),
            other => panic!("expected Password, got {other:?}"),
        }
    }

    #[test]
    fn write_file_creates_parent_dirs_and_writes() {
        let base = std::env::temp_dir().join(format!("be-writefile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dest = base.join("nested/deeper/out.bin");
        let returned = write_file(&dest, b"hello bytes").unwrap();
        assert_eq!(returned, dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello bytes");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn fetch_returns_none_for_absent_file() {
        let Ok(dir) = std::env::var("BACKUP_EXTRACTOR_TEST_BACKUP") else {
            eprintln!("skipping: set BACKUP_EXTRACTOR_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("BACKUP_EXTRACTOR_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let out = std::env::temp_dir().join("be-fetch-none.bin");
        let got = backup
            .fetch("NoSuchDomain", "no/such/file", &out)
            .expect("fetch should not error");
        assert!(got.is_none(), "absent file must return None");
    }

    #[test]
    fn fetch_writes_address_book_when_present() {
        let Ok(dir) = std::env::var("BACKUP_EXTRACTOR_TEST_BACKUP") else {
            eprintln!("skipping: set BACKUP_EXTRACTOR_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("BACKUP_EXTRACTOR_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let out = std::env::temp_dir().join("be-fetch-ab.sqlitedb");
        let _ = std::fs::remove_file(&out);
        if let Some(path) = backup
            .fetch("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb", &out)
            .unwrap()
        {
            // SQLite files start with the "SQLite format 3\0" magic.
            let head = std::fs::read(&path).unwrap();
            assert!(head.starts_with(b"SQLite format 3\0"), "decrypted a real DB");
        } else {
            eprintln!("backup has no AddressBook; skipping content assertion");
        }
    }
}

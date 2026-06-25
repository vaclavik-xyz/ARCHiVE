//! Open, decrypt, and read files from an on-disk iOS backup.

use std::path::Path;

use crabapple::error::BackupError as CrabError;
use crabapple::{Authentication, Backup as RawBackup};

/// Errors from opening or reading a backup.
#[derive(Debug)]
pub enum BackupError {
    /// The backup could not be opened or decrypted (wrong/missing password,
    /// corrupt manifest, …). Carries a human-readable reason.
    Open(String),
    /// An I/O error while materializing a decrypted file.
    Io(std::io::Error),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Open(why) => write!(f, "could not open backup: {why}"),
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

/// An opened (and, if needed, unlocked) iOS backup.
pub struct Backup {
    // First read by the file-fetch methods added in the next change; unused here.
    #[allow(dead_code)]
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
            Err(CrabError::NotEncrypted) => RawBackup::open(dir, &Authentication::None)
                .map_err(|why| BackupError::Open(why.to_string()))?,
            Err(why) => return Err(BackupError::Open(why.to_string())),
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
}

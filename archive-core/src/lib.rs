#![warn(missing_docs)]
//! Open, decrypt, and read files from an on-disk iOS backup.

use std::path::{Path, PathBuf};

use crabapple::error::BackupError as CrabError;
use crabapple::{Authentication, Backup as RawBackup};

pub mod carve;
pub mod keychain;

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
    /// Hardware model identifier (e.g. "iPhone14,2").
    pub model: String,
    /// Device serial number.
    pub serial: String,
    /// The backup's unique device identifier.
    pub udid: String,
}

/// Result of a backup completeness check (see [`Backup::verify_integrity`]).
pub struct IntegrityReport {
    /// Regular-file entries considered (directories/symlinks excluded).
    pub total_files: usize,
    /// Entries whose stored file exists on disk.
    pub present: usize,
    /// Entries whose stored file is missing (an incomplete/truncated backup).
    pub missing: usize,
    /// Whether on-disk sizes were compared (false for encrypted backups, whose
    /// stored blobs are AES-padded and legitimately differ in size).
    pub size_checked: bool,
    /// Present entries whose on-disk size differs from the recorded size
    /// (always 0 when `size_checked` is false).
    pub size_mismatch: usize,
    /// Up to `sample_cap` missing entries as `"<domain>:<relative_path>"`.
    pub missing_sample: Vec<String>,
    /// Up to `sample_cap` size-mismatched entries as `"<domain>:<relative_path>"`.
    pub mismatch_sample: Vec<String>,
}

/// Whether a Unix `mode` word denotes a regular file (vs a directory or symlink).
fn is_regular_file(mode: u64) -> bool {
    const S_IFMT: u64 = 0o170000;
    const S_IFREG: u64 = 0o100000;
    (mode & S_IFMT) == S_IFREG
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

/// Decrypted keychain plist bytes paired with the protection-class key map needed
/// to unwrap its items.
type KeychainMaterial = (Vec<u8>, std::collections::HashMap<u32, Vec<u8>>);

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
            model: lockdown.product_type.clone(),
            serial: lockdown.serial_number.clone(),
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

    /// Whether the backup is encrypted. Callers use this to decide whether a
    /// password must be forwarded to downstream tooling.
    pub fn is_encrypted(&self) -> bool {
        self.raw.is_encrypted()
    }

    /// Recover saved **Wi-Fi passwords** from the backup keychain.
    ///
    /// Returns an empty list when the backup has no keychain — only **encrypted**
    /// backups include `KeychainDomain/keychain-backup.plist`. The keychain file
    /// is decrypted at the file level by crabapple; the per-item secrets inside
    /// are then unwrapped against crabapple's already-unlocked protection-class
    /// keys and AES-GCM-decrypted by [`keychain::extract_wifi`].
    ///
    /// Sensitive: the returned `password` fields are plaintext PSKs. Callers must
    /// not log them.
    pub fn wifi_credentials(&self) -> Result<Vec<keychain::WifiCredential>, BackupError> {
        match self.keychain_material()? {
            Some((plist_bytes, class_keys)) => Ok(keychain::extract_wifi(&plist_bytes, &class_keys)),
            None => Ok(Vec::new()),
        }
    }

    /// Recover saved **website/app passwords** (the keychain `inet` array) from
    /// the backup keychain. Empty when the backup has no keychain (only encrypted
    /// backups include it).
    ///
    /// Sensitive: the returned `password` fields are plaintext. Callers must not
    /// log them. See [`Backup::wifi_credentials`] for the decryption model.
    pub fn saved_passwords(&self) -> Result<Vec<keychain::PasswordCredential>, BackupError> {
        match self.keychain_material()? {
            Some((plist_bytes, class_keys)) => Ok(keychain::extract_passwords(&plist_bytes, &class_keys)),
            None => Ok(Vec::new()),
        }
    }

    /// Decrypt the keychain plist and gather the protection-class keys needed to
    /// unwrap its items. `Ok(None)` when the backup has no keychain (unencrypted
    /// backups). Shared by every keychain extractor.
    fn keychain_material(&self) -> Result<Option<KeychainMaterial>, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let Some(entry) = entries
            .iter()
            .find(|e| e.domain == "KeychainDomain" && e.relative_path == "keychain-backup.plist")
        else {
            return Ok(None);
        };
        let plist_bytes = self
            .raw
            .decrypt_entry(entry)
            .map_err(|why| BackupError::Open(why.to_string()))?;
        // Class keys arrive already unwrapped from crabapple's manifest keybag.
        let class_keys: std::collections::HashMap<u32, Vec<u8>> = self
            .raw
            .manifest
            .keys()
            .map_err(|why| BackupError::Open(why.to_string()))?
            .iter()
            .map(|(id, pck)| (*id, pck.key.as_ref().to_vec()))
            .collect();
        Ok(Some((plist_bytes, class_keys)))
    }

    /// Whether the backup contains a file at `domain` + `relative_path`, without
    /// decrypting it.
    pub fn has(&self, domain: &str, relative_path: &str) -> Result<bool, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        Ok(entries
            .iter()
            .any(|e| e.domain == domain && e.relative_path == relative_path))
    }

    /// Relative paths of every backup entry in `domain` whose `relative_path`
    /// starts with `prefix` (empty `prefix` lists the whole domain). Sorted for
    /// deterministic output. Read-only; decrypts nothing (manifest scan only).
    pub fn list(&self, domain: &str, prefix: &str) -> Result<Vec<String>, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let mut paths: Vec<String> = entries
            .iter()
            .filter(|e| e.domain == domain && e.relative_path.starts_with(prefix))
            .map(|e| e.relative_path.clone())
            .collect();
        paths.sort();
        Ok(paths)
    }

    /// Distinct third-party app bundle identifiers installed on the device,
    /// derived from the backup's per-app manifest domains (`AppDomain-<bundle id>`
    /// — each installed user app gets one). Sorted, deduped. Read-only (manifest
    /// scan only; decrypts nothing). App-group / system domains are excluded (they
    /// are containers shared between apps), and first-party `com.apple.*` bundles
    /// are filtered out so the result is genuinely third-party apps.
    pub fn app_bundle_ids(&self) -> Result<Vec<String>, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let mut ids = std::collections::BTreeSet::new();
        for e in entries.iter() {
            if let Some(id) = e
                .domain
                .strip_prefix("AppDomain-")
                .filter(|id| !id.is_empty() && !id.starts_with("com.apple."))
            {
                ids.insert(id.to_string());
            }
        }
        Ok(ids.into_iter().collect())
    }

    /// Verify the backup is complete: every regular-file manifest entry has its
    /// stored file on disk, and (for unencrypted backups only) the on-disk size
    /// matches the recorded size. `sample_cap` bounds each sample list. Read-only;
    /// decrypts nothing.
    pub fn verify_integrity(&self, sample_cap: usize) -> Result<IntegrityReport, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let size_checked = !self.raw.is_encrypted();
        let mut report = IntegrityReport {
            total_files: 0,
            present: 0,
            missing: 0,
            size_checked,
            size_mismatch: 0,
            missing_sample: Vec::new(),
            mismatch_sample: Vec::new(),
        };
        for e in entries.iter() {
            // Only regular files have stored content; skip directories/symlinks.
            if !is_regular_file(e.metadata.mode) {
                continue;
            }
            report.total_files += 1;
            let path = self.raw.backup_path.join(e.source());
            // `symlink_metadata` does not follow links; require an actual regular
            // file at the storage path (a dir/symlink there means a malformed
            // backup and is treated as missing, not silently counted present).
            match std::fs::symlink_metadata(&path) {
                Ok(md) if md.file_type().is_file() => {
                    report.present += 1;
                    if size_checked && md.len() != e.metadata.size {
                        report.size_mismatch += 1;
                        if report.mismatch_sample.len() < sample_cap {
                            report.mismatch_sample.push(format!("{}:{}", e.domain, e.relative_path));
                        }
                    }
                }
                _ => {
                    report.missing += 1;
                    if report.missing_sample.len() < sample_cap {
                        report.missing_sample.push(format!("{}:{}", e.domain, e.relative_path));
                    }
                }
            }
        }
        Ok(report)
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
        // Materialize atomically (stream to a sibling temp file, rename onto
        // `dest` only on success) so large media (e.g. videos) never buffer fully
        // in memory and a failed fetch never leaves a truncated file at the export
        // path. Encrypted entries stream through crabapple's decrypt reader;
        // unencrypted entries already sit in plaintext on disk.
        if self.raw.is_encrypted() {
            let mut reader = self
                .raw
                .decrypt_entry_stream(&entry)
                .map_err(|why| BackupError::Open(why.to_string()))?;
            write_atomic(dest, |out| std::io::copy(&mut reader, out).map(|_| ()))?;
        } else {
            let src = self.raw.backup_path.join(entry.source());
            write_atomic(dest, |out| {
                let mut input = std::fs::File::open(&src)?;
                std::io::copy(&mut input, out).map(|_| ())
            })?;
        }
        Ok(Some(dest.to_path_buf()))
    }
}

/// Atomically materialize a file: `write` fills a sibling temp file, which is
/// renamed onto `dest` only after it succeeds. On any error the temp file is
/// discarded (and a pre-existing `dest` is left untouched), so callers never see
/// a truncated output at the export path.
fn write_atomic(
    dest: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<(), BackupError> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    write(tmp.as_file_mut())?;
    tmp.persist(dest).map_err(|e| BackupError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP
    // to a backup directory (and ARCHIVE_TEST_PASSWORD if encrypted).
    // Skipped when the env var is unset so CI stays green without fixtures.
    #[test]
    fn opens_real_backup_and_reads_device_info() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref())
            .expect("open backup");
        let info = backup.device_info();
        assert!(!info.product_version.is_empty(), "iOS version should be set");
        // model/serial are exposed from the lockdown record (non-fatal if empty);
        // access them to document they are part of DeviceInfo without logging the
        // values, which are sensitive device identifiers.
        let _ = (&info.model, &info.serial);
    }

    #[test]
    fn write_atomic_writes_dest_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested/out.bin");
        write_atomic(&dest, |f| f.write_all(b"hello")).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[test]
    fn write_atomic_leaves_dest_untouched_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(&dest, b"original").unwrap();
        // The writer partially writes, then fails: dest must keep its old content.
        let result = write_atomic(&dest, |f| {
            f.write_all(b"partial").unwrap();
            Err(std::io::Error::other("boom"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original", "no partial output exposed");
    }

    #[test]
    fn is_regular_file_detects_file_type() {
        assert!(is_regular_file(0o100644)); // regular file
        assert!(!is_regular_file(0o040755)); // directory
        assert!(!is_regular_file(0o120777)); // symlink
        assert!(!is_regular_file(0));
    }

    #[test]
    fn verify_integrity_on_real_backup() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let r = backup.verify_integrity(20).unwrap();
        assert_eq!(r.present + r.missing, r.total_files, "present + missing == total");
        assert!(r.missing_sample.len() <= 20, "missing sample is capped");
        assert!(r.mismatch_sample.len() <= 20, "mismatch sample is capped");
        if !r.size_checked {
            assert_eq!(r.size_mismatch, 0, "size mismatch is 0 when size not checked");
        }
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
    fn list_filters_by_domain_and_prefix() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let got = backup.list("HomeDomain", "Library/").unwrap();
        assert!(got.iter().all(|p| p.starts_with("Library/")), "all under prefix");
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted, "list returns sorted paths");
    }

    #[test]
    fn fetch_returns_none_for_absent_file() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let out = std::env::temp_dir().join("be-fetch-none.bin");
        let got = backup
            .fetch("NoSuchDomain", "no/such/file", &out)
            .expect("fetch should not error");
        assert!(got.is_none(), "absent file must return None");
    }

    #[test]
    fn fetch_writes_address_book_when_present() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
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

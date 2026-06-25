//! Open, decrypt, and read files from an on-disk iOS backup.

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

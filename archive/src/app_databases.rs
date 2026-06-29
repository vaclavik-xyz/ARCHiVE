//! A per-app database recoverability report. For every installed third-party app
//! it lists the database-like files in the app's backup domain and classifies each
//! as a **readable** plain SQLite database (table count included) or **not
//! readable** (encrypted/SQLCipher, a Core Data/binary store, or otherwise not a
//! plain SQLite file).
//!
//! This answers the practical recovery question — *what can actually be pulled out
//! of this app?* — which matters because many modern messaging apps (Viber,
//! Messenger, …) deliberately exclude or encrypt their message stores, so the
//! data simply is not present in a normal backup.
//!
//! Unlike the single-file extractors this is a *view* that walks the backup
//! manifest, so the heavy lifting (listing domains, fetching candidate files)
//! lives in the command layer; this module owns the data shape and the cheap,
//! testable classification helpers.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// One database-like file found in an app's backup domain.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppDatabase {
    /// Owning app/group label (e.g. `com.viber`, or
    /// `group.net.whatsapp.WhatsApp.shared`).
    pub app: String,
    /// Full backup domain the file lives in (`AppDomain-…` / `AppDomainGroup-…`).
    pub domain: String,
    /// Path of the file within the app domain.
    pub path: String,
    /// File size in bytes (of the decrypted file).
    pub bytes: u64,
    /// Whether the file is a plain, openable SQLite database.
    pub readable: bool,
    /// Number of tables when `readable`; `None` otherwise.
    pub tables: Option<i64>,
}

/// If `domain` is a third-party app or app-group container, return a short label
/// (the bundle id or group id). `None` for system / first-party (`com.apple.*`)
/// and non-app domains, so the report stays focused on third-party data.
pub fn third_party_label(domain: &str) -> Option<String> {
    let id = domain
        .strip_prefix("AppDomain-")
        .or_else(|| domain.strip_prefix("AppDomainGroup-"))?;
    if id.is_empty() || id.starts_with("com.apple.") || id.starts_with("group.com.apple.") {
        return None;
    }
    Some(id.to_string())
}

/// File-name suffixes worth probing as databases (lower-cased comparison).
pub const DB_SUFFIXES: &[&str] = &[".sqlite", ".sqlite3", ".sqlitedb", ".db", ".data"];

/// Whether a relative path looks like a database file by extension.
pub fn is_db_like(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    DB_SUFFIXES.iter().any(|s| p.ends_with(s))
}

/// Whether the bytes begin with the SQLite file magic (a plain, unencrypted
/// database). Encrypted stores (SQLCipher) and Core Data binary stores do not.
pub fn is_sqlite(head: &[u8]) -> bool {
    head.starts_with(b"SQLite format 3\0")
}

/// Count the tables in a SQLite database; `None` if it cannot be opened/queried.
pub fn count_tables(db_path: &Path) -> Option<i64> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_like_matches_known_suffixes() {
        assert!(is_db_like("Documents/ChatStorage.sqlite"));
        assert!(is_db_like("a/b/Contacts.DATA"));
        assert!(is_db_like("x.sqlitedb"));
        assert!(!is_db_like("Documents/photo.jpg"));
        assert!(!is_db_like("Library/prefs.plist"));
    }

    #[test]
    fn third_party_label_covers_app_and_group_domains() {
        assert_eq!(third_party_label("AppDomain-com.viber"), Some("com.viber".to_string()));
        assert_eq!(
            third_party_label("AppDomainGroup-group.net.whatsapp.WhatsApp.shared"),
            Some("group.net.whatsapp.WhatsApp.shared".to_string())
        );
        // System / first-party domains are excluded.
        assert_eq!(third_party_label("AppDomain-com.apple.mobilesafari"), None);
        assert_eq!(third_party_label("AppDomainGroup-group.com.apple.notes"), None);
        assert_eq!(third_party_label("HomeDomain"), None);
        assert_eq!(third_party_label("CameraRollDomain"), None);
    }

    #[test]
    fn sqlite_magic_detects_plain_databases() {
        assert!(is_sqlite(b"SQLite format 3\0and more"));
        assert!(!is_sqlite(b"SQLCipher encrypted...."));
        assert!(!is_sqlite(b""));
        assert!(!is_sqlite(b"SQLite"));
    }

    #[test]
    fn count_tables_reads_real_sqlite() {
        let dir = std::env::temp_dir().join(format!("be-appdb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE a (x INTEGER); CREATE TABLE b (y TEXT);").unwrap();
        drop(conn);
        assert_eq!(count_tables(&db), Some(2));
        // A non-SQLite file yields None rather than panicking.
        let junk = dir.join("junk.db");
        std::fs::write(&junk, b"not a database").unwrap();
        assert_eq!(count_tables(&junk), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}

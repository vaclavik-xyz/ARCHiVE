//! Read voicemail metadata from an iOS `voicemail.db` (audio files excluded).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::{cocoa_to_iso, unix_to_iso};
use crate::sqlite_util::table_columns;

/// One voicemail record (metadata only).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Voicemail {
    /// Caller phone number; empty when withheld/unknown.
    pub sender: String,
    /// Receipt time as ISO 8601 (RFC 3339) UTC (Unix epoch); empty if unconvertible.
    pub date: String,
    /// Voicemail length in seconds.
    pub duration_seconds: i64,
    /// Whether the voicemail was moved to the Deleted folder.
    pub trashed: bool,
    /// When it was trashed (ISO 8601 UTC, Cocoa epoch); `None` when not trashed.
    pub trashed_at: Option<String>,
    /// Carrier expiry (ISO 8601 UTC, Unix epoch); `None` when unset/absent.
    pub expiration: Option<String>,
    /// Raw `flags` bitmask, preserved (bit meanings are undocumented).
    pub flags: i64,
}

/// Parse every voicemail record from `db_path` (opened read-only), tolerating a
/// missing optional `expiration` column. `date` is Unix epoch; `trashed_date`
/// is Cocoa 2001 epoch (the two columns intentionally use different epochs).
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Voicemail>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "voicemail")?;
    let expiration_sel = if cols.contains("expiration") { "expiration" } else { "NULL" };

    let sql = format!(
        "SELECT sender, date, duration, trashed_date, flags, {expiration_sel} \
         FROM voicemail ORDER BY date"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let sender: Option<String> = row.get(0)?;
        let date: Option<i64> = row.get(1)?;
        let duration: Option<i64> = row.get(2)?;
        let trashed_date: Option<i64> = row.get(3)?;
        let flags: Option<i64> = row.get(4)?;
        let expiration: Option<i64> = row.get(5)?;

        let trashed = trashed_date.unwrap_or(0) != 0;
        Ok(Voicemail {
            sender: sender.unwrap_or_default(),
            date: date.and_then(unix_to_iso).unwrap_or_default(),
            duration_seconds: duration.unwrap_or(0),
            trashed,
            trashed_at: if trashed {
                trashed_date.and_then(|t| cocoa_to_iso(t as f64))
            } else {
                None
            },
            expiration: expiration.filter(|&e| e != 0).and_then(unix_to_iso),
            flags: flags.unwrap_or(0),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_voicemail;

    #[test]
    fn parses_voicemail_with_mixed_epochs() {
        let dir = std::env::temp_dir().join(format!("be-vm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("voicemail.db");
        let _ = std::fs::remove_file(&db);
        make_voicemail(&db);

        let vms = parse(&db).unwrap();
        assert_eq!(vms.len(), 2);

        let active = &vms[0];
        assert_eq!(active.sender, "+420776452878");
        assert_eq!(active.date, "2020-09-13T12:26:40+00:00"); // Unix 1_600_000_000
        assert_eq!(active.duration_seconds, 30);
        assert!(!active.trashed);
        assert_eq!(active.trashed_at, None);
        assert_eq!(active.expiration, None);

        let trashed = &vms[1];
        assert_eq!(trashed.sender, ""); // NULL → empty
        assert!(trashed.trashed);
        assert!(trashed.trashed_at.is_some()); // Cocoa 600_000_000 → some ISO
        assert_eq!(trashed.flags, 75);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_when_expiration_column_absent() {
        let dir = std::env::temp_dir().join(format!("be-vm-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("voicemail.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE voicemail (ROWID INTEGER PRIMARY KEY, date INTEGER, sender TEXT, duration INTEGER, trashed_date INTEGER, flags INTEGER);
             INSERT INTO voicemail (ROWID, date, sender, duration, trashed_date, flags) VALUES (1, 1600000000, '+1', 5, 0, 0);",
        )
        .unwrap();
        drop(conn);

        let vms = parse(&db).unwrap();
        assert_eq!(vms.len(), 1);
        assert_eq!(vms[0].expiration, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}

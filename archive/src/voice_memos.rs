//! Read Voice Memos metadata from an iOS `CloudRecordings.db`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One Voice Memo record (audio path filled in by the extraction step).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceMemo {
    /// User-visible title (`ZCUSTOMLABEL`); empty when unnamed.
    pub title: String,
    /// Recording start as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub date: String,
    /// Length in seconds (rounded).
    pub duration_seconds: i64,
    /// `ZPATH` basename, e.g. `A1B2C3.m4a`; joins audio ↔ metadata. Empty if unknown.
    pub source_file: String,
    /// Output-relative path to the extracted audio (`voice_memos/<name>`);
    /// `None` until extraction runs or when the file is absent from the backup.
    pub audio_file: Option<String>,
}

/// Parse the Voice Memos store. Schema-tolerant: the title and path columns vary
/// by iOS version, so they are selected only when present.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<VoiceMemo>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZCLOUDRECORDING")?;
    let label_sel = if cols.contains("ZCUSTOMLABEL") {
        "ZCUSTOMLABEL"
    } else if cols.contains("ZENCRYPTEDTITLE") {
        "ZENCRYPTEDTITLE"
    } else {
        "NULL"
    };
    let path_sel = if cols.contains("ZPATH") { "ZPATH" } else { "NULL" };

    let sql = format!(
        "SELECT {label_sel}, ZDATE, ZDURATION, {path_sel} \
         FROM ZCLOUDRECORDING ORDER BY ZDATE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let title: Option<String> = row.get(0)?;
        let date: Option<f64> = row.get(1)?;
        let duration: Option<f64> = row.get(2)?;
        let path: Option<String> = row.get(3)?;
        Ok(VoiceMemo {
            title: title.unwrap_or_default(),
            date: date.and_then(cocoa_to_iso).unwrap_or_default(),
            duration_seconds: duration.unwrap_or(0.0).round() as i64,
            source_file: path.map(|p| basename(&p)).unwrap_or_default(),
            audio_file: None,
        })
    })?;
    rows.collect()
}

/// Last path component of a (possibly `/`-containing) `ZPATH`.
pub(crate) fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_voicememos;

    #[test]
    fn parses_two_memos_with_cocoa_dates() {
        let dir = std::env::temp_dir().join(format!("be-vm-memos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CloudRecordings.db");
        let _ = std::fs::remove_file(&db);
        make_voicememos(&db);

        let memos = parse(&db).unwrap();
        assert_eq!(memos.len(), 2);
        assert_eq!(memos[0].title, "Schůzka");
        assert_eq!(memos[0].date, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000 + 978_307_200
        assert_eq!(memos[0].duration_seconds, 13); // 12.5 rounded
        assert_eq!(memos[0].source_file, "20200101 120000.m4a");
        assert_eq!(memos[0].audio_file, None);
        assert_eq!(memos[1].title, ""); // NULL label → empty
        assert_eq!(memos[1].source_file, "A1B2C3.m4a");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn basename_takes_last_component() {
        assert_eq!(basename("a/b/c.m4a"), "c.m4a");
        assert_eq!(basename("x.m4a"), "x.m4a");
        assert_eq!(basename(""), "");
    }
}

//! Per-app foreground usage from CoreDuet's `knowledgeC.db` — one of the richest
//! behavioural artifacts in an iOS backup. The `ZOBJECT` table holds typed event
//! streams; the `/app/usage` stream records each foreground app session with a
//! bundle id (`ZVALUESTRING`) and a start/end (`ZSTARTDATE`/`ZENDDATE`, Cocoa
//! epoch). We aggregate sessions per bundle into total foreground seconds, a
//! session count, and the first/last use. Schema-tolerant: missing columns or a
//! database without the usage stream yield an empty result rather than an error.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// Backup domain of the knowledge store (the canonical one; `inspect` reports
/// presence by probing every [`CANDIDATES`] path).
pub const DOMAIN: &str = "AppDomainGroup-group.com.apple.coreduet";

/// Candidate (domain, path) pairs probed in order — the store has lived in a few
/// places across iOS versions.
pub const CANDIDATES: &[(&str, &str)] = &[
    ("AppDomainGroup-group.com.apple.coreduet", "Library/Knowledge/knowledgeC.db"),
    ("HomeDomain", "Library/CoreDuet/Knowledge/knowledgeC.db"),
    ("HomeDomain", "Library/CoreDuet/knowledgeC.db"),
];

/// Aggregated foreground usage for one app bundle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppUsage {
    /// App bundle identifier (`ZVALUESTRING` of the `/app/usage` stream).
    pub bundle: String,
    /// Total foreground time in whole seconds (sum of session durations).
    pub total_seconds: i64,
    /// Number of foreground sessions.
    pub sessions: i64,
    /// First / last session time, ISO 8601 UTC; empty when unknown.
    pub first_used: String,
    pub last_used: String,
}

impl AppUsage {
    /// Total foreground time as `Hh Mm` (or `Mm`), for the HTML view.
    pub fn total_human(&self) -> String {
        let (h, m) = (self.total_seconds / 3600, (self.total_seconds % 3600) / 60);
        if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
    }
}

/// Parse and aggregate `/app/usage` sessions per bundle, ordered by total time
/// descending. Returns an empty vector when the usage stream is absent.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<AppUsage>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZOBJECT")?;
    // Required columns for the aggregation; bail out cleanly if the schema differs.
    let needed = ["ZSTREAMNAME", "ZVALUESTRING", "ZSTARTDATE", "ZENDDATE"];
    if needed.iter().any(|c| !cols.contains(*c)) {
        return Ok(Vec::new());
    }

    let sql = "SELECT ZVALUESTRING, \
               CAST(COALESCE(SUM(ZENDDATE - ZSTARTDATE), 0) AS INTEGER), \
               COUNT(*), MIN(ZSTARTDATE), MAX(ZENDDATE) \
               FROM ZOBJECT \
               WHERE ZSTREAMNAME = '/app/usage' AND ZVALUESTRING IS NOT NULL \
               GROUP BY ZVALUESTRING \
               ORDER BY SUM(ZENDDATE - ZSTARTDATE) DESC";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        let bundle: String = row.get(0)?;
        let total_seconds: i64 = row.get(1)?;
        let sessions: i64 = row.get(2)?;
        let first: Option<f64> = row.get(3)?;
        let last: Option<f64> = row.get(4)?;
        Ok(AppUsage {
            bundle,
            total_seconds: total_seconds.max(0),
            sessions,
            first_used: first.and_then(cocoa_to_iso).unwrap_or_default(),
            last_used: last.and_then(cocoa_to_iso).unwrap_or_default(),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        // Minimal ZOBJECT with two app-usage sessions for one app, one for another,
        // plus an unrelated stream row that must be ignored.
        conn.execute_batch(
            "CREATE TABLE ZOBJECT (Z_PK INTEGER PRIMARY KEY, ZSTREAMNAME TEXT,
                ZVALUESTRING TEXT, ZSTARTDATE REAL, ZENDDATE REAL);
             INSERT INTO ZOBJECT VALUES
                (1, '/app/usage', 'com.apple.mobilesafari', 600000000.0, 600000060.0),
                (2, '/app/usage', 'com.apple.mobilesafari', 600000100.0, 600000130.0),
                (3, '/app/usage', 'com.burbn.instagram', 600000200.0, 600000260.0),
                (4, '/display/isBacklit', NULL, 600000000.0, 600000300.0);",
        )
        .unwrap();
    }

    #[test]
    fn aggregates_app_usage_sorted_by_time() {
        let dir = std::env::temp_dir().join(format!("be-ku-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("knowledgeC.db");
        let _ = std::fs::remove_file(&db);
        make_db(&db);

        let rows = parse(&db).unwrap();
        assert_eq!(rows.len(), 2); // the non-usage stream and NULL value are excluded

        // Safari: 60 + 30 = 90s over 2 sessions → first by total time.
        assert_eq!(rows[0].bundle, "com.apple.mobilesafari");
        assert_eq!(rows[0].total_seconds, 90);
        assert_eq!(rows[0].sessions, 2);
        assert_eq!(rows[0].first_used, "2020-01-06T10:40:00+00:00");
        assert_eq!(rows[0].last_used, "2020-01-06T10:42:10+00:00"); // 600000130

        assert_eq!(rows[1].bundle, "com.burbn.instagram");
        assert_eq!(rows[1].total_seconds, 60);
        assert_eq!(rows[1].sessions, 1);
    }

    #[test]
    fn total_human_formats_hours_and_minutes() {
        let mk = |s| AppUsage { bundle: String::new(), total_seconds: s, sessions: 0, first_used: String::new(), last_used: String::new() };
        assert_eq!(mk(90).total_human(), "1m");
        assert_eq!(mk(3661).total_human(), "1h 1m");
        assert_eq!(mk(7200).total_human(), "2h 0m");
    }

    #[test]
    fn unsupported_schema_yields_empty() {
        let dir = std::env::temp_dir().join(format!("be-ku-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("knowledgeC.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE ZOBJECT (Z_PK INTEGER, ZUUID TEXT);").unwrap();
        drop(conn);
        assert!(parse(&db).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}

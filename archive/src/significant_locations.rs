//! Recorded location history from iOS's **routined** database — the subsystem
//! behind *Settings → Privacy → Location Services → System Services → Significant
//! Locations*. The `ZRTCLLOCATIONMO` table stores individual CoreLocation fixes
//! (latitude/longitude, timestamp, altitude, accuracy, speed) that the on-device
//! learning uses to cluster frequently-visited places.
//!
//! Availability: the routined store lives under `Library/Caches/`, which iOS
//! **excludes from ordinary iTunes/Finder backups** — so on a standard backup
//! this extractor finds nothing and returns an honest empty result. It still
//! recovers the history from backups that *do* include the caches (full
//! filesystem / forensic extractions). Schema-tolerant: a missing table or column
//! yields an empty result rather than an error.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// Backup domain and candidate paths of the routined location databases, probed
/// in order (the store has used a few file names across iOS versions).
pub const DOMAIN: &str = "RootDomain";
pub const PATHS: &[&str] = &[
    "Library/Caches/com.apple.routined/Cache.sqlite",
    "Library/Caches/com.apple.routined/cloud.sqlite",
    "Library/Caches/com.apple.routined/local.sqlite",
];

/// One recorded location fix.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocationFix {
    /// Fix time, ISO 8601 UTC; empty when the timestamp is unparseable.
    pub timestamp: String,
    /// WGS-84 latitude / longitude in degrees.
    pub latitude: f64,
    pub longitude: f64,
    /// Altitude in metres (0.0 when not recorded).
    pub altitude: f64,
    /// Horizontal accuracy radius in metres; negative means invalid.
    pub horizontal_accuracy: f64,
    /// Speed in m/s; negative means invalid/unknown.
    pub speed: f64,
}

/// Read the location-fix history from a routined database, newest first. Returns
/// an empty vector when `ZRTCLLOCATIONMO` (or its required columns) is absent.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<LocationFix>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = match table_columns(&conn, "ZRTCLLOCATIONMO") {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    // Coordinates and a timestamp are the load-bearing columns; bail cleanly when
    // the schema differs.
    if !["ZLATITUDE", "ZLONGITUDE", "ZTIMESTAMP"].iter().all(|c| cols.contains(*c)) {
        return Ok(Vec::new());
    }
    let opt = |name: &'static str| if cols.contains(name) { name } else { "NULL" };
    let sql = format!(
        "SELECT ZTIMESTAMP, ZLATITUDE, ZLONGITUDE, {}, {}, {} \
         FROM ZRTCLLOCATIONMO \
         WHERE ZLATITUDE IS NOT NULL AND ZLONGITUDE IS NOT NULL \
         ORDER BY ZTIMESTAMP DESC",
        opt("ZALTITUDE"), opt("ZHORIZONTALACCURACY"), opt("ZSPEED"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let ts: Option<f64> = row.get(0)?;
        Ok(LocationFix {
            timestamp: ts.and_then(cocoa_to_iso).unwrap_or_default(),
            latitude: row.get(1)?,
            longitude: row.get(2)?,
            altitude: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
            horizontal_accuracy: row.get::<_, Option<f64>>(4)?.unwrap_or(-1.0),
            speed: row.get::<_, Option<f64>>(5)?.unwrap_or(-1.0),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &Path, full: bool) {
        let conn = Connection::open(path).unwrap();
        if full {
            conn.execute_batch(
                "CREATE TABLE ZRTCLLOCATIONMO (Z_PK INTEGER PRIMARY KEY, ZTIMESTAMP REAL,
                    ZLATITUDE REAL, ZLONGITUDE REAL, ZALTITUDE REAL,
                    ZHORIZONTALACCURACY REAL, ZSPEED REAL);
                 INSERT INTO ZRTCLLOCATIONMO VALUES
                    (1, 600000000.0, 50.0875, 14.4213, 200.0, 10.0, 1.5),
                    (2, 600000100.0, 48.8566, 2.3522, 35.0, 5.0, 0.0),
                    (3, 600000050.0, NULL, NULL, NULL, NULL, NULL);",
            )
            .unwrap();
        } else {
            // Minimal schema: only the required columns present.
            conn.execute_batch(
                "CREATE TABLE ZRTCLLOCATIONMO (Z_PK INTEGER PRIMARY KEY, ZTIMESTAMP REAL,
                    ZLATITUDE REAL, ZLONGITUDE REAL);
                 INSERT INTO ZRTCLLOCATIONMO VALUES (1, 600000000.0, 50.0, 14.0);",
            )
            .unwrap();
        }
    }

    #[test]
    fn reads_fixes_newest_first_skipping_null_coords() {
        let dir = std::env::temp_dir().join(format!("be-loc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Cache.sqlite");
        let _ = std::fs::remove_file(&db);
        make_db(&db, true);

        let fixes = parse(&db).unwrap();
        assert_eq!(fixes.len(), 2); // the NULL-coordinate row is dropped
        // Newest (ZTIMESTAMP 600000100) first.
        assert_eq!(fixes[0].latitude, 48.8566);
        assert_eq!(fixes[0].timestamp, "2020-01-06T10:41:40+00:00");
        assert_eq!(fixes[1].latitude, 50.0875);
        assert_eq!(fixes[1].altitude, 200.0);
        assert_eq!(fixes[1].speed, 1.5);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn minimal_schema_defaults_optional_fields() {
        let dir = std::env::temp_dir().join(format!("be-loc-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("local.sqlite");
        let _ = std::fs::remove_file(&db);
        make_db(&db, false);
        let fixes = parse(&db).unwrap();
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].altitude, 0.0);
        assert_eq!(fixes[0].horizontal_accuracy, -1.0);
        assert_eq!(fixes[0].speed, -1.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn absent_table_yields_empty() {
        let dir = std::env::temp_dir().join(format!("be-loc-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Cache.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE Unrelated (x INTEGER);").unwrap();
        drop(conn);
        assert!(parse(&db).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}

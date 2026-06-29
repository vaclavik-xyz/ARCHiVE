//! Read call history from an iOS `CallHistory.storedata` (Core Data SQLite).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One call-history record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Call {
    /// Remote party: a phone number, or an Apple ID/email for FaceTime calls.
    pub number: String,
    /// Call start time as ISO 8601 (RFC 3339) UTC; empty if unconvertible.
    pub date: String,
    /// Duration in whole seconds (0 for unanswered).
    pub duration_seconds: i64,
    /// `"incoming"` or `"outgoing"`.
    pub direction: String,
    /// Whether the call was answered (false = missed/declined/no-answer).
    pub answered: bool,
    /// `"phone"`, `"facetime"`, a raw third-party bundle id, or `"unknown"`.
    pub service: String,
    /// Best-effort FaceTime video flag (true=video, false=audio); `None` when not
    /// derivable. Version-dependent and undocumented — see `call_type`.
    pub video: Option<bool>,
    /// Raw `ZCALLTYPE` integer, preserved for fidelity.
    pub call_type: Option<i64>,
    /// Optional carrier/region location hint.
    pub location: Option<String>,
    /// Optional ISO 3166-1 alpha-2 country code (uppercased).
    pub country: Option<String>,
}

fn decode_address(bytes: Option<Vec<u8>>) -> String {
    match bytes {
        Some(b) => String::from_utf8_lossy(&b).trim_end_matches('\0').to_string(),
        None => String::new(),
    }
}

fn classify_service(provider: Option<&str>, call_type: Option<i64>) -> String {
    if let Some(p) = provider {
        if p == "com.apple.Telephony" {
            return "phone".to_string();
        }
        if p == "com.apple.FaceTime" {
            return "facetime".to_string();
        }
        if !p.is_empty() {
            return p.to_string();
        }
    }
    match call_type {
        Some(1) => "phone".to_string(),
        Some(8) | Some(16) => "facetime".to_string(),
        _ => "unknown".to_string(),
    }
}

fn derive_video(call_type: Option<i64>) -> Option<bool> {
    match call_type {
        Some(8) => Some(true),
        Some(16) => Some(false),
        _ => None,
    }
}

/// Parse every call record from `db_path` (opened read-only), tolerating
/// missing optional columns across iOS versions.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Call>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZCALLRECORD")?;
    let col = |name: &'static str| -> &'static str { if cols.contains(name) { name } else { "NULL" } };

    let sql = format!(
        "SELECT ZADDRESS, ZDATE, ZDURATION, ZORIGINATED, ZANSWERED, ZCALLTYPE, {}, {}, {} \
         FROM ZCALLRECORD ORDER BY ZDATE",
        col("ZSERVICE_PROVIDER"),
        col("ZLOCATION"),
        col("ZISO_COUNTRY_CODE"),
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        // ZADDRESS is a phone number stored as TEXT on some iOS versions (e.g. 16)
        // and as a BLOB on others, so read it type-tolerantly rather than fixing a
        // Rust type (a fixed `Vec<u8>` errors on a Text column).
        let address: String = match row.get_ref(0)? {
            rusqlite::types::ValueRef::Text(t) => decode_address(Some(t.to_vec())),
            rusqlite::types::ValueRef::Blob(b) => decode_address(Some(b.to_vec())),
            _ => String::new(),
        };
        let date: Option<f64> = row.get(1)?;
        let duration: Option<f64> = row.get(2)?;
        let originated: Option<i64> = row.get(3)?;
        let answered: Option<i64> = row.get(4)?;
        let call_type: Option<i64> = row.get(5)?;
        let provider: Option<String> = row.get(6)?;
        let location: Option<String> = row.get(7)?;
        let country: Option<String> = row.get(8)?;
        Ok(Call {
            number: address,
            date: date.and_then(cocoa_to_iso).unwrap_or_default(),
            duration_seconds: duration.unwrap_or(0.0).round() as i64,
            direction: if originated == Some(1) { "outgoing" } else { "incoming" }.to_string(),
            answered: answered == Some(1),
            service: classify_service(provider.as_deref(), call_type),
            video: derive_video(call_type),
            call_type,
            location: location.filter(|s| !s.is_empty()),
            country: country.filter(|s| !s.is_empty()).map(|s| s.to_uppercase()),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_callhistory;

    #[test]
    fn decode_address_reads_ascii_blob() {
        assert_eq!(decode_address(Some(b"+420776452878".to_vec())), "+420776452878");
        assert_eq!(decode_address(Some(b"a@b.cz\0".to_vec())), "a@b.cz");
        assert_eq!(decode_address(None), "");
    }

    #[test]
    fn classify_service_prefers_provider_then_call_type() {
        assert_eq!(classify_service(Some("com.apple.Telephony"), Some(1)), "phone");
        assert_eq!(classify_service(Some("com.apple.FaceTime"), Some(8)), "facetime");
        assert_eq!(classify_service(Some("net.whatsapp.WhatsApp"), Some(0)), "net.whatsapp.WhatsApp");
        assert_eq!(classify_service(None, Some(1)), "phone");
        assert_eq!(classify_service(None, Some(16)), "facetime");
        assert_eq!(classify_service(None, None), "unknown");
    }

    #[test]
    fn derive_video_maps_known_call_types() {
        assert_eq!(derive_video(Some(8)), Some(true));
        assert_eq!(derive_video(Some(16)), Some(false));
        assert_eq!(derive_video(Some(1)), None);
        assert_eq!(derive_video(None), None);
    }

    #[test]
    fn parses_calls_ordered_by_date() {
        let dir = std::env::temp_dir().join(format!("be-calls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CallHistory.storedata");
        let _ = std::fs::remove_file(&db);
        make_callhistory(&db);

        let calls = parse(&db).unwrap();
        assert_eq!(calls.len(), 2);

        // ORDER BY ZDATE ascending: the FaceTime row (cocoa 50) is first.
        let ft = &calls[0];
        assert_eq!(ft.number, "jana@example.cz");
        assert_eq!(ft.direction, "incoming");
        assert!(!ft.answered);
        assert_eq!(ft.service, "facetime");
        assert_eq!(ft.video, Some(true));
        assert_eq!(ft.call_type, Some(8));
        assert_eq!(ft.country, None);

        let phone = &calls[1];
        assert_eq!(phone.number, "+420776452878");
        assert_eq!(phone.date, "2001-01-01T00:01:40+00:00");
        assert_eq!(phone.duration_seconds, 42);
        assert_eq!(phone.direction, "outgoing");
        assert!(phone.answered);
        assert_eq!(phone.service, "phone");
        assert_eq!(phone.country, Some("CZ".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_when_optional_columns_absent() {
        let dir = std::env::temp_dir().join(format!("be-calls-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CallHistory.storedata");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZCALLRECORD (Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL, ZADDRESS BLOB, ZORIGINATED INTEGER, ZANSWERED INTEGER, ZCALLTYPE INTEGER);
             INSERT INTO ZCALLRECORD VALUES (1, 100.0, 10.0, CAST('+420' AS BLOB), 1, 1, 1);",
        )
        .unwrap();
        drop(conn);

        let calls = parse(&db).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].service, "phone");
        assert_eq!(calls[0].location, None);
        assert_eq!(calls[0].country, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_text_zaddress() {
        // iOS 16 stores ZADDRESS as TEXT, not BLOB. Reading it must not error.
        let dir = std::env::temp_dir().join(format!("be-calls-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CallHistory.storedata");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZCALLRECORD (Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL, ZADDRESS TEXT, ZORIGINATED INTEGER, ZANSWERED INTEGER, ZCALLTYPE INTEGER);
             INSERT INTO ZCALLRECORD VALUES (1, 100.0, 10.0, '+420776452878', 1, 1, 1);",
        )
        .unwrap();
        drop(conn);

        let calls = parse(&db).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].number, "+420776452878");
    }
}

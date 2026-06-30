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
    /// Address-book name resolved from `number`; empty when no contact matched
    /// (or contacts were unavailable). Populated by [`crate::enrich`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub contact_name: String,
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
            contact_name: String::new(),
        })
    })?;
    rows.collect()
}

/// Build a customer-facing summary of the recovered call history.
pub fn summary(items: &[Call]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};
    use std::collections::HashSet;

    let party = |c: &Call| -> String {
        if !c.contact_name.is_empty() {
            c.contact_name.clone()
        } else if !c.number.is_empty() {
            c.number.clone()
        } else {
            "Neznámé".to_string()
        }
    };
    let incoming = items.iter().filter(|c| c.direction == "incoming").count();
    let outgoing = items.iter().filter(|c| c.direction == "outgoing").count();
    let missed = items.iter().filter(|c| !c.answered).count();
    let facetime = items.iter().filter(|c| c.service == "facetime").count();
    let talk_min = (items.iter().map(|c| c.duration_seconds.max(0)).sum::<i64>() / 60) as usize;
    let distinct = items.iter().map(party).collect::<HashSet<_>>().len();
    let top_contacts: Vec<(String, usize)> = tally(items.iter().map(party)).into_iter().take(12).collect();

    Summary::new("calls", "Hovory", "hovorů", items.len())
        .count("Příchozích", incoming)
        .count("Odchozích", outgoing)
        .count("Nezvednutých", missed)
        .count("FaceTime hovorů", facetime)
        .count("Celkem provoláno (min)", talk_min)
        .count("Různých čísel", distinct)
        .period_from(iso_range(items.iter().map(|c| c.date.as_str())))
        .breakdown("Po letech", year_rows(items.iter().map(|c| c.date.as_str())))
        .breakdown("Podle typu", tally(items.iter().map(|c| c.service.clone())))
        .breakdown("Nejčastější kontakty", top_contacts)
        .note("Historie hovorů je na zařízení omezená — starší záznamy se postupně přepisují.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_callhistory;

    fn call(number: &str, date: &str, dur: i64, dir: &str, answered: bool, service: &str, contact: &str) -> Call {
        Call {
            number: number.into(),
            date: date.into(),
            duration_seconds: dur,
            direction: dir.into(),
            answered,
            service: service.into(),
            video: None,
            call_type: None,
            location: None,
            country: None,
            contact_name: contact.into(),
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let calls = vec![
            call("+420111", "2023-05-01T10:00:00+00:00", 120, "incoming", true, "phone", "Jana"),
            call("+420111", "2024-06-01T10:00:00+00:00", 0, "outgoing", false, "phone", "Jana"),
            call("jana@x.cz", "", 60, "incoming", true, "facetime", ""),
        ];
        let s = summary(&calls);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "hovorů");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Příchozích"), Some(2));
        assert_eq!(get("Odchozích"), Some(1));
        assert_eq!(get("Nezvednutých"), Some(1));
        assert_eq!(get("FaceTime hovorů"), Some(1));
        assert_eq!(get("Celkem provoláno (min)"), Some(3)); // (120+0+60)/60
        assert_eq!(get("Různých čísel"), Some(2));
        let yr = s.breakdowns.iter().find(|b| b.title == "Po letech").unwrap();
        assert_eq!(yr.rows, vec![("2023".to_string(), 1), ("2024".to_string(), 1)]);
        let contacts = s.breakdowns.iter().find(|b| b.title == "Nejčastější kontakty").unwrap();
        assert_eq!(contacts.rows[0], ("Jana".to_string(), 2));
        assert!(s.period.is_some()); // derived from the two dated calls
    }

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

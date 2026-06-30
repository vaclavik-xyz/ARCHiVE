//! Read voicemail metadata from an iOS `voicemail.db` (audio files excluded).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::{cocoa_to_iso, unix_to_iso};
use crate::sqlite_util::table_columns;

/// One voicemail record (metadata; audio path filled in separately).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Voicemail {
    /// Primary key in `voicemail.db`; a stable per-backup identifier and the
    /// base name of the audio file (`Library/Voicemail/<rowid>.amr`).
    pub rowid: i64,
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
    /// Output-relative path to the extracted audio (e.g.
    /// `voicemail_audio/2020-09-13_122640_+420…_3.m4a`); `None` until audio
    /// extraction runs, or when the backup has no audio for this row.
    pub audio_file: Option<String>,
    /// Address-book name resolved from `sender`; empty when no contact matched.
    /// Populated by [`crate::enrich`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub contact_name: String,
}

pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Voicemail>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "voicemail")?;
    let expiration_sel = if cols.contains("expiration") { "expiration" } else { "NULL" };

    let sql = format!(
        "SELECT ROWID, sender, date, duration, trashed_date, flags, {expiration_sel} \
         FROM voicemail ORDER BY date"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let rowid: i64 = row.get(0)?;
        let sender: Option<String> = row.get(1)?;
        let date: Option<i64> = row.get(2)?;
        let duration: Option<i64> = row.get(3)?;
        let trashed_date: Option<i64> = row.get(4)?;
        let flags: Option<i64> = row.get(5)?;
        let expiration: Option<i64> = row.get(6)?;

        let trashed = trashed_date.unwrap_or(0) != 0;
        Ok(Voicemail {
            rowid,
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
            audio_file: None,
            contact_name: String::new(),
        })
    })?;
    rows.collect()
}

/// Build a customer-facing summary of the recovered voicemails.
pub fn summary(items: &[Voicemail]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};

    let caller = |v: &Voicemail| -> String {
        if !v.contact_name.is_empty() {
            v.contact_name.clone()
        } else if !v.sender.is_empty() {
            v.sender.clone()
        } else {
            "Neznámé/skryté".to_string()
        }
    };
    let inbox = items.iter().filter(|v| !v.trashed).count();
    let trash = items.iter().filter(|v| v.trashed).count();
    let total_min = (items.iter().map(|v| v.duration_seconds.max(0)).sum::<i64>() / 60) as usize;
    let unknown = items.iter().filter(|v| v.sender.is_empty()).count();
    let matched = items.iter().filter(|v| !v.contact_name.is_empty()).count();
    let top_callers: Vec<(String, usize)> =
        tally(items.iter().map(caller)).into_iter().take(12).collect();

    Summary::new("voicemail", "Hlasové zprávy", "hlasových zpráv", items.len())
        .count("Ve schránce", inbox)
        .count("V koši", trash)
        .count("Celková délka (min)", total_min)
        .count("Od neznámých čísel", unknown)
        .count("Spárováno s kontaktem", matched)
        .period_from(iso_range(items.iter().map(|v| v.date.as_str())))
        .breakdown("Po letech", year_rows(items.iter().map(|v| v.date.as_str())))
        .breakdown("Nejčastější volající", top_callers)
        .breakdown(
            "Podle stavu",
            vec![("Ve schránce".to_string(), inbox), ("V koši".to_string(), trash)],
        )
        .note("Audio (.amr) se kopíruje jen s přepínačem --audio.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_voicemail;

    fn vm(sender: &str, date: &str, dur: i64, trashed: bool, contact: &str) -> Voicemail {
        Voicemail {
            rowid: 1,
            sender: sender.into(),
            date: date.into(),
            duration_seconds: dur,
            trashed,
            trashed_at: None,
            expiration: None,
            flags: 0,
            audio_file: None,
            contact_name: contact.into(),
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let vms = vec![
            vm("+420111", "2022-05-01T10:00:00+00:00", 30, false, "Jana"),
            vm("+420111", "2024-06-01T10:00:00+00:00", 90, true, "Jana"),
            vm("", "", 12, false, ""),
        ];
        let s = summary(&vms);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "hlasových zpráv");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Ve schránce"), Some(2));
        assert_eq!(get("V koši"), Some(1));
        assert_eq!(get("Celková délka (min)"), Some(2)); // (30+90+12)/60
        assert_eq!(get("Od neznámých čísel"), Some(1));
        assert_eq!(get("Spárováno s kontaktem"), Some(2));
        let yr = s.breakdowns.iter().find(|b| b.title == "Po letech").unwrap();
        assert_eq!(yr.rows, vec![("2022".to_string(), 1), ("2024".to_string(), 1)]);
        let callers = s.breakdowns.iter().find(|b| b.title == "Nejčastější volající").unwrap();
        assert_eq!(callers.rows[0], ("Jana".to_string(), 2));
        assert!(s.period.is_some()); // derived from the two dated voicemails
    }

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
        assert_eq!(active.expiration.as_deref(), Some("2020-09-14T12:26:40+00:00")); // Unix 1_600_086_400
        assert_eq!(active.rowid, 1);
        assert_eq!(active.audio_file, None);

        let trashed = &vms[1];
        assert_eq!(trashed.sender, ""); // NULL → empty
        assert!(trashed.trashed);
        assert_eq!(trashed.date, "2020-09-13T12:28:20+00:00"); // Unix 1_600_000_100
        assert_eq!(trashed.duration_seconds, 12);
        assert_eq!(trashed.trashed_at.as_deref(), Some("2020-01-06T10:40:00+00:00")); // Cocoa 600_000_000 + 978_307_200 = Unix 1_578_307_200
        assert_eq!(trashed.expiration, None); // ROWID 2 expiration = 0 → None
        assert_eq!(trashed.flags, 75);
        assert_eq!(trashed.rowid, 2);
        assert_eq!(trashed.audio_file, None);

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
        assert_eq!(vms[0].rowid, 1);
        assert_eq!(vms[0].audio_file, None);
        assert_eq!(vms[0].expiration, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}

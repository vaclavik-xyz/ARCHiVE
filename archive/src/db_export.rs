//! Combined SQLite export: consolidate the extracted data into one queryable
//! `archive.sqlite` so an examiner can run cross-store SQL (joins, time-range
//! filters, `LIKE` searches) instead of opening one CSV per store. It writes the
//! unified chronological `timeline` (every dated event from every in-process
//! extractor) plus structured tables for the richest stores — `contacts`, `calls`
//! and `whatsapp` — whose extra columns (duration, direction, media flags) are the
//! ones worth querying. The file is a plain unencrypted SQLite database.

use rusqlite::{Connection, params};

use crate::calls::Call;
use crate::contacts::Contact;
use crate::timeline::Event;
use crate::whatsapp::WaMessage;

/// Row counts written per table, surfaced in the JSON envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct Counts {
    pub timeline: usize,
    pub contacts: usize,
    pub calls: usize,
    pub whatsapp: usize,
}

const SCHEMA: &str = "\
CREATE TABLE timeline (timestamp TEXT, kind TEXT, summary TEXT);
CREATE TABLE contacts (first TEXT, last TEXT, organization TEXT, phones TEXT, emails TEXT, note TEXT);
CREATE TABLE calls (date TEXT, number TEXT, contact_name TEXT, duration_seconds INTEGER, direction TEXT, answered INTEGER, service TEXT);
CREATE TABLE whatsapp (date TEXT, chat TEXT, contact_name TEXT, from_me INTEGER, text TEXT, has_media INTEGER);
";

fn join_values(items: &[crate::contacts::Labeled]) -> String {
    items.iter().map(|l| l.value.as_str()).collect::<Vec<_>>().join("; ")
}

/// Build a fresh SQLite database at `path` from the extracted data, replacing any
/// existing file. All inserts run in one transaction (fast for large message
/// stores). Returns the per-table row counts.
pub fn write(
    path: &std::path::Path,
    events: &[Event],
    contacts: &[Contact],
    calls: &[Call],
    whatsapp: &[WaMessage],
) -> rusqlite::Result<Counts> {
    // Start from a clean file so re-runs do not append to stale tables.
    let _ = std::fs::remove_file(path);
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("INSERT INTO timeline (timestamp, kind, summary) VALUES (?1, ?2, ?3)")?;
        for e in events {
            stmt.execute(params![e.timestamp, e.kind, e.summary])?;
        }
        let mut stmt =
            tx.prepare("INSERT INTO contacts (first, last, organization, phones, emails, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for c in contacts {
            stmt.execute(params![c.first, c.last, c.organization, join_values(&c.phones), join_values(&c.emails), c.note])?;
        }
        let mut stmt = tx.prepare(
            "INSERT INTO calls (date, number, contact_name, duration_seconds, direction, answered, service) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for c in calls {
            stmt.execute(params![c.date, c.number, c.contact_name, c.duration_seconds, c.direction, c.answered as i64, c.service])?;
        }
        let mut stmt = tx
            .prepare("INSERT INTO whatsapp (date, chat, contact_name, from_me, text, has_media) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for m in whatsapp {
            stmt.execute(params![m.date, m.chat, m.contact_name, m.from_me as i64, m.text, (!m.source_path.is_empty()) as i64])?;
        }
    }
    tx.commit()?;

    Ok(Counts {
        timeline: events.len(),
        contacts: contacts.len(),
        calls: calls.len(),
        whatsapp: whatsapp.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::Labeled;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("be-dbx-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("archive.sqlite")
    }

    #[test]
    fn writes_all_tables_and_is_queryable() {
        let events = vec![Event { timestamp: "2020-01-06T00:00:00+00:00".into(), kind: "call".into(), summary: "outgoing Jan (42s)".into() }];
        let contacts = vec![Contact {
            first: "Jan".into(),
            last: "Novák".into(),
            organization: "Acme".into(),
            phones: vec![Labeled { label: "mobile".into(), value: "+420776452878".into() }],
            emails: vec![Labeled { label: "home".into(), value: "jan@example.com".into() }],
            note: String::new(),
            addresses: vec![],
        }];
        let calls = vec![Call {
            number: "+420776452878".into(),
            date: "2020-01-06T00:00:00+00:00".into(),
            duration_seconds: 42,
            direction: "outgoing".into(),
            answered: true,
            service: "phone".into(),
            video: None,
            call_type: None,
            location: None,
            country: Some("CZ".into()),
            contact_name: "Jan Novák".into(),
        }];

        let whatsapp = vec![WaMessage {
            chat: "Jana".into(),
            chat_jid: "420776452878@s.whatsapp.net".into(),
            sender: String::new(),
            from_me: true,
            date: "2020-02-01T00:00:00+00:00".into(),
            text: "Ahoj".into(),
            source_path: String::new(),
            media_file: None,
            contact_name: "Jana Nováková".into(),
        }];

        let path = tmp("write");
        let counts = write(&path, &events, &contacts, &calls, &whatsapp).unwrap();
        assert_eq!(counts.timeline, 1);
        assert_eq!(counts.contacts, 1);
        assert_eq!(counts.calls, 1);
        assert_eq!(counts.whatsapp, 1);

        // Reopen and verify the rows are queryable with joined handles.
        let conn = Connection::open(&path).unwrap();
        let phones: String = conn.query_row("SELECT phones FROM contacts", [], |r| r.get(0)).unwrap();
        assert_eq!(phones, "+420776452878");
        let dur: i64 = conn.query_row("SELECT duration_seconds FROM calls WHERE answered = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(dur, 42);
        let wa_name: String = conn.query_row("SELECT contact_name FROM whatsapp WHERE from_me = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(wa_name, "Jana Nováková");
        let kinds: i64 = conn.query_row("SELECT count(*) FROM timeline WHERE kind = 'call'", [], |r| r.get(0)).unwrap();
        assert_eq!(kinds, 1);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn rerun_replaces_not_appends() {
        let path = tmp("rerun");
        let ev = vec![Event { timestamp: "t".into(), kind: "note".into(), summary: "x".into() }];
        write(&path, &ev, &[], &[], &[]).unwrap();
        write(&path, &ev, &[], &[], &[]).unwrap(); // second run
        let conn = Connection::open(&path).unwrap();
        let n: i64 = conn.query_row("SELECT count(*) FROM timeline", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1); // replaced, not doubled
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

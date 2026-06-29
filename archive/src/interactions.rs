//! Per-contact communication history from CoreDuet's `interactionC.db` — the
//! "People" sibling of `knowledgeC.db`. Where `device-usage` answers *which apps
//! did the owner use*, this answers *whom did the owner communicate with, through
//! which apps, and when*. The `ZINTERACTIONS` table holds one row per interaction
//! event (a message, call, mail, …) carrying an app bundle id (`ZBUNDLEID`), a
//! direction (`ZDIRECTION`), a timestamp (`ZSTARTDATE`, Cocoa epoch) and a sender
//! link (`ZSENDER` → `ZCONTACTS.Z_PK`).
//!
//! We aggregate per contact, joined on the interaction's **sender** — the robust
//! link that needs no version-specific recipient join table. For incoming events
//! the sender is the other party; outgoing events whose sender resolves to a
//! contact are counted too. Each contact yields a total, an incoming/outgoing
//! split, the distinct apps used, and the first/last interaction time.
//!
//! Schema-tolerant: a missing table, a missing `ZSENDER` join key, or absent
//! optional columns yield an empty/partial result rather than an error.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// Canonical backup domain of the interactions store (the one reported by
/// `inspect`; presence is probed across every [`CANDIDATES`] path).
pub const DOMAIN: &str = "AppDomainGroup-group.com.apple.coreduet";

/// Candidate (domain, path) pairs probed in order — the store has lived under a
/// couple of CoreDuet app-group domains across iOS versions.
pub const CANDIDATES: &[(&str, &str)] = &[
    ("AppDomainGroup-group.com.apple.coreduet", "Library/CoreDuet/People/interactionC.db"),
    ("AppDomainGroup-group.com.apple.coreduetd", "Library/CoreDuet/People/interactionC.db"),
    ("HomeDomain", "Library/CoreDuet/People/interactionC.db"),
];

/// Aggregated interaction history with one contact.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContactInteractions {
    /// Best available display label: `ZDISPLAYNAME`, else `ZNAME`, else the
    /// identifier, else `(unknown)`.
    pub display_name: String,
    /// Contact identifier (`ZIDENTIFIER` — typically a phone number, email, or
    /// contact handle); empty when absent.
    pub identifier: String,
    /// Total interactions linked to this contact (as the interaction's sender).
    pub total: i64,
    /// Interactions flagged incoming (`ZDIRECTION = 0`).
    pub incoming: i64,
    /// Interactions flagged outgoing (`ZDIRECTION = 1`).
    pub outgoing: i64,
    /// Distinct app bundle ids involved, sorted.
    pub apps: Vec<String>,
    /// First / last interaction time, ISO 8601 UTC; empty when unknown.
    pub first: String,
    pub last: String,
}

impl ContactInteractions {
    /// Distinct apps as a comma-separated string, for the HTML view.
    pub fn apps_human(&self) -> String {
        self.apps.join(", ")
    }
}

/// Per-contact accumulator keyed by `ZCONTACTS.Z_PK`.
#[derive(Default)]
struct Acc {
    display_name: String,
    identifier: String,
    total: i64,
    incoming: i64,
    outgoing: i64,
    apps: BTreeSet<String>,
    first: Option<f64>,
    last: Option<f64>,
}

/// Parse and aggregate interactions per contact, most-interacted first. Returns
/// an empty vector when the store lacks `ZINTERACTIONS`/`ZCONTACTS` or the
/// `ZSENDER` join key (an unsupported schema).
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<ContactInteractions>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let icols = match table_columns(&conn, "ZINTERACTIONS") {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let ccols = match table_columns(&conn, "ZCONTACTS") {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    // The sender link is the load-bearing column; without it there is nothing to
    // group by. (`Z_PK` on ZCONTACTS is the primary key and always present.)
    if !icols.contains("ZSENDER") {
        return Ok(Vec::new());
    }

    // Select each optional column when present, else a literal NULL placeholder so
    // the row shape stays fixed across iOS schema variants.
    let col = |cols: &std::collections::HashSet<String>, prefix: &str, name: &str| {
        if cols.contains(name) { format!("{prefix}.{name}") } else { "NULL".to_string() }
    };
    let c_disp = col(&ccols, "c", "ZDISPLAYNAME");
    let c_name = col(&ccols, "c", "ZNAME");
    let c_ident = col(&ccols, "c", "ZIDENTIFIER");
    let i_dir = col(&icols, "i", "ZDIRECTION");
    let i_bundle = col(&icols, "i", "ZBUNDLEID");
    let i_start = col(&icols, "i", "ZSTARTDATE");

    let sql = format!(
        "SELECT c.Z_PK, {c_disp}, {c_name}, {c_ident}, {i_dir}, {i_bundle}, {i_start} \
         FROM ZINTERACTIONS i JOIN ZCONTACTS c ON c.Z_PK = i.ZSENDER \
         WHERE i.ZSENDER IS NOT NULL"
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut by_contact: std::collections::HashMap<i64, Acc> = std::collections::HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let pk: i64 = row.get(0)?;
        let disp: Option<String> = row.get(1)?;
        let name: Option<String> = row.get(2)?;
        let ident: Option<String> = row.get(3)?;
        let dir: Option<i64> = row.get(4)?;
        let bundle: Option<String> = row.get(5)?;
        let start: Option<f64> = row.get(6)?;

        let acc = by_contact.entry(pk).or_default();
        // Resolve the label/identifier once, on first sight of the contact.
        if acc.total == 0 {
            acc.identifier = ident.clone().unwrap_or_default();
            acc.display_name = [disp, name, ident]
                .into_iter()
                .flatten()
                .find(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "(unknown)".to_string());
        }
        acc.total += 1;
        match dir {
            Some(0) => acc.incoming += 1,
            Some(1) => acc.outgoing += 1,
            _ => {}
        }
        if let Some(b) = bundle
            && !b.is_empty()
        {
            acc.apps.insert(b);
        }
        if let Some(s) = start {
            acc.first = Some(acc.first.map_or(s, |f| f.min(s)));
            acc.last = Some(acc.last.map_or(s, |l| l.max(s)));
        }
    }

    let mut out: Vec<ContactInteractions> = by_contact
        .into_values()
        .map(|a| ContactInteractions {
            display_name: a.display_name,
            identifier: a.identifier,
            total: a.total,
            incoming: a.incoming,
            outgoing: a.outgoing,
            apps: a.apps.into_iter().collect(),
            first: a.first.and_then(cocoa_to_iso).unwrap_or_default(),
            last: a.last.and_then(cocoa_to_iso).unwrap_or_default(),
        })
        .collect();
    // Most-interacted first; ties broken by name for a stable, deterministic order.
    out.sort_by(|x, y| y.total.cmp(&x.total).then_with(|| x.display_name.cmp(&y.display_name)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZCONTACTS (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT, ZNAME TEXT, ZDISPLAYNAME TEXT);
             INSERT INTO ZCONTACTS VALUES
                (1, '+420777111222', 'Alice Example', 'Alice'),
                (2, 'bob@example.com', NULL, NULL);
             CREATE TABLE ZINTERACTIONS (Z_PK INTEGER PRIMARY KEY, ZSENDER INTEGER,
                ZDIRECTION INTEGER, ZBUNDLEID TEXT, ZSTARTDATE REAL);
             INSERT INTO ZINTERACTIONS VALUES
                (1, 1, 0, 'com.apple.MobileSMS', 600000000.0),
                (2, 1, 0, 'com.apple.mobilephone', 600000100.0),
                (3, 1, 1, 'com.apple.MobileSMS', 600000200.0),
                (4, 2, 0, 'com.apple.mobilemail', 600000050.0),
                (5, NULL, 0, 'com.apple.MobileSMS', 600000300.0);",
        )
        .unwrap();
    }

    #[test]
    fn aggregates_per_contact_sorted_by_total() {
        let dir = std::env::temp_dir().join(format!("be-ix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("interactionC.db");
        let _ = std::fs::remove_file(&db);
        make_db(&db);

        let rows = parse(&db).unwrap();
        assert_eq!(rows.len(), 2); // the NULL-sender row is excluded

        // Alice: 3 interactions (2 incoming, 1 outgoing) over two apps → first.
        assert_eq!(rows[0].display_name, "Alice");
        assert_eq!(rows[0].identifier, "+420777111222");
        assert_eq!(rows[0].total, 3);
        assert_eq!(rows[0].incoming, 2);
        assert_eq!(rows[0].outgoing, 1);
        assert_eq!(rows[0].apps, vec!["com.apple.MobileSMS", "com.apple.mobilephone"]);
        assert_eq!(rows[0].first, "2020-01-06T10:40:00+00:00");
        assert_eq!(rows[0].last, "2020-01-06T10:43:20+00:00"); // 600000200

        // Bob: name/displayname NULL → label falls back to the identifier.
        assert_eq!(rows[1].display_name, "bob@example.com");
        assert_eq!(rows[1].identifier, "bob@example.com");
        assert_eq!(rows[1].total, 1);
        assert_eq!(rows[1].incoming, 1);
        assert_eq!(rows[1].outgoing, 0);
        assert_eq!(rows[1].apps, vec!["com.apple.mobilemail"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn minimal_schema_defaults_optional_fields() {
        let dir = std::env::temp_dir().join(format!("be-ix-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("interactionC.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZCONTACTS (Z_PK INTEGER PRIMARY KEY, ZIDENTIFIER TEXT);
             INSERT INTO ZCONTACTS VALUES (1, 'x@y');
             CREATE TABLE ZINTERACTIONS (Z_PK INTEGER PRIMARY KEY, ZSENDER INTEGER);
             INSERT INTO ZINTERACTIONS VALUES (1, 1), (2, 1);",
        )
        .unwrap();
        drop(conn);

        let rows = parse(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total, 2);
        assert_eq!(rows[0].incoming, 0); // no ZDIRECTION column
        assert_eq!(rows[0].outgoing, 0);
        assert!(rows[0].apps.is_empty()); // no ZBUNDLEID column
        assert_eq!(rows[0].first, ""); // no ZSTARTDATE column
        assert_eq!(rows[0].last, "");
        assert_eq!(rows[0].display_name, "x@y"); // identifier fallback
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsupported_schema_yields_empty() {
        let dir = std::env::temp_dir().join(format!("be-ix-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("interactionC.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        // ZINTERACTIONS present but without the ZSENDER join key.
        conn.execute_batch(
            "CREATE TABLE ZINTERACTIONS (Z_PK INTEGER, ZFOO TEXT);
             CREATE TABLE ZCONTACTS (Z_PK INTEGER);",
        )
        .unwrap();
        drop(conn);
        assert!(parse(&db).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn candidates_cover_known_coreduet_domains() {
        assert!(CANDIDATES.iter().any(|(d, _)| *d == "AppDomainGroup-group.com.apple.coreduet"));
        assert!(CANDIDATES.iter().all(|(_, p)| p.ends_with("interactionC.db")));
    }
}

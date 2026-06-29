//! Cross-version schema test matrix.
//!
//! iOS point releases add, drop and rename SQLite columns, so every extractor has
//! to cope with more than one schema generation. Two complementary matrices lock
//! that resilience in:
//!
//! 1. **Classifier matrix** — for every store in [`schema_check::EXPECTATIONS`],
//!    build a *real* in-memory SQLite database in four generations (full schema;
//!    minimal = required columns only, as an older iOS that lacks the optional
//!    ones; a drifted schema with one required column dropped; and one with the
//!    tolerated `table_optional` tables absent) and assert `schema_check` reaches
//!    the right verdict against live `PRAGMA table_info` output — not the mocked
//!    column sets the unit tests use.
//!
//! 2. **Extractor matrix** — for the gating-heavy extractors that did not yet have
//!    a schema-variant test (accounts, data-usage, whatsapp, voice-memos,
//!    device-usage), build a minimal and a full schema on disk and assert the real
//!    `parse()` succeeds on both. An empty-but-valid table is enough: it proves the
//!    extractor's SQL still *prepares* against that schema generation, which is the
//!    property that breaks when a column the query names unconditionally is gone.
//!    The "minimal" schema keeps each Core Data table's structural `Z_PK` (always
//!    present in reality, like SQLite's rowid) and drops only the optional attribute
//!    columns an older iOS would lack — which is the variance the gating handles.

#![cfg(test)]

use std::collections::HashSet;

use rusqlite::Connection;

use crate::schema_check::{EXPECTATIONS, TableNeed, check_table, store_status};
use crate::sqlite_util::table_columns;

/// Create `table` with `cols` (all TEXT — only the names matter here), quoting
/// every identifier so reserved words like `key`/`value` are legal.
fn make_table(conn: &Connection, table: &str, cols: &[&str]) {
    let mut seen = HashSet::new();
    let defs: Vec<String> = cols
        .iter()
        .filter(|c| seen.insert(**c))
        .map(|c| format!("\"{c}\" TEXT"))
        .collect();
    let body = if defs.is_empty() { "\"_filler\" TEXT".to_string() } else { defs.join(", ") };
    conn.execute_batch(&format!("CREATE TABLE \"{table}\" ({body});")).unwrap();
}

/// The live column set as `schema_check` sees it (empty/`None` ⇒ table absent).
fn live(conn: &Connection, table: &str) -> Option<HashSet<String>> {
    let cols = table_columns(conn, table).ok()?;
    if cols.is_empty() { None } else { Some(cols) }
}

fn all_cols(need: &TableNeed) -> Vec<&'static str> {
    need.required.iter().chain(need.optional.iter()).copied().collect()
}

/// Build a DB from a per-table column chooser, then classify every table.
fn classify<'a>(store_needs: &'a [TableNeed], cols_for: impl Fn(&TableNeed) -> Option<Vec<&'a str>>) -> Vec<crate::schema_check::TableReport> {
    let conn = Connection::open_in_memory().unwrap();
    for need in store_needs {
        if let Some(cols) = cols_for(need) {
            make_table(&conn, need.table, &cols);
        }
    }
    store_needs.iter().map(|n| check_table(n, live(&conn, n.table).as_ref())).collect()
}

#[test]
fn matrix_full_schema_is_ok_for_every_store() {
    for store in EXPECTATIONS {
        let reports = classify(store.needs, |n| Some(all_cols(n)));
        assert_eq!(store_status(&reports), "ok", "store `{}` should be ok on a full schema", store.command);
    }
}

#[test]
fn matrix_minimal_schema_stays_ok_and_reports_missing_optionals() {
    // An older iOS that lacks the optional columns: every required column is still
    // present, so the store does not drift, and every optional is reported missing.
    for store in EXPECTATIONS {
        let conn = Connection::open_in_memory().unwrap();
        for need in store.needs {
            make_table(&conn, need.table, need.required);
        }
        for need in store.needs {
            let r = check_table(need, live(&conn, need.table).as_ref());
            assert!(r.missing_required.is_empty(), "{}.{} lost a required column on a minimal schema", store.command, need.table);
            assert_eq!(
                r.missing_optional.len(),
                need.optional.len(),
                "{}.{} should report every optional column missing",
                store.command,
                need.table
            );
        }
    }
}

#[test]
fn matrix_dropping_any_required_column_drifts() {
    // Drop each required column in turn (others full) and assert the store drifts.
    for store in EXPECTATIONS {
        for (ti, target) in store.needs.iter().enumerate() {
            for drop in target.required {
                let reports = classify(store.needs, |n| {
                    let cols: Vec<&str> = if std::ptr::eq(n, &store.needs[ti]) {
                        all_cols(n).into_iter().filter(|c| c != drop).collect()
                    } else {
                        all_cols(n)
                    };
                    Some(cols)
                });
                assert_eq!(
                    store_status(&reports),
                    "drifted",
                    "store `{}` should drift when `{}.{}` is dropped",
                    store.command,
                    target.table,
                    drop
                );
            }
        }
    }
}

#[test]
fn matrix_optional_tables_absent_does_not_drift() {
    // A `table_optional` table (e.g. Health's) being entirely absent is tolerated.
    for store in EXPECTATIONS {
        if !store.needs.iter().any(|n| n.table_optional) {
            continue;
        }
        let reports = classify(store.needs, |n| if n.table_optional { None } else { Some(all_cols(n)) });
        assert_eq!(
            store_status(&reports),
            "ok",
            "store `{}` should stay ok when its tolerated tables are absent",
            store.command
        );
    }
}

// --- Extractor resilience matrix (gating-heavy stores without a variant test) ---

/// Build a DB at `<dir>/<file>` from `(table, columns)` specs and return its path.
fn build_db(dir: &std::path::Path, file: &str, tables: &[(&str, &[&str])]) -> std::path::PathBuf {
    let path = dir.join(file);
    let conn = Connection::open(&path).unwrap();
    for (table, cols) in tables {
        make_table(&conn, table, cols);
    }
    path
}

/// Run `parse` against a minimal (required-only) and a full schema and require
/// both to succeed — the cross-version graceful-degradation guarantee.
fn assert_parses_both<T>(
    label: &str,
    minimal: &[(&str, &[&str])],
    full: &[(&str, &[&str])],
    file: &str,
    parse: impl Fn(&std::path::Path) -> rusqlite::Result<Vec<T>>,
) {
    let dir = std::env::temp_dir().join(format!("be-xver-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let min_db = build_db(&dir, file, minimal);
    assert!(parse(&min_db).is_ok(), "{label}: parse failed on a minimal schema");
    let _ = std::fs::remove_file(&min_db);
    let full_db = build_db(&dir, file, full);
    assert!(parse(&full_db).is_ok(), "{label}: parse failed on a full schema");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn extractor_matrix_accounts() {
    assert_parses_both(
        "accounts",
        &[("ZACCOUNT", &["Z_PK", "ZACCOUNTTYPE"]), ("ZACCOUNTTYPE", &["Z_PK"])],
        &[
            ("ZACCOUNT", &["Z_PK", "ZACCOUNTTYPE", "ZUSERNAME", "ZACCOUNTDESCRIPTION", "ZOWNINGBUNDLEID", "ZDATE", "ZACTIVE"]),
            ("ZACCOUNTTYPE", &["Z_PK", "ZACCOUNTTYPEDESCRIPTION", "ZIDENTIFIER"]),
        ],
        "Accounts3.sqlite",
        crate::accounts::parse,
    );
}

#[test]
fn extractor_matrix_data_usage() {
    assert_parses_both(
        "data-usage",
        &[("ZLIVEUSAGE", &["Z_PK", "ZHASPROCESS"]), ("ZPROCESS", &["Z_PK"])],
        &[
            ("ZLIVEUSAGE", &["Z_PK", "ZHASPROCESS", "ZWWANIN", "ZWWANOUT", "ZWIFIIN", "ZWIFIOUT", "ZTIMESTAMP"]),
            ("ZPROCESS", &["Z_PK", "ZPROCNAME", "ZBUNDLENAME"]),
        ],
        "DataUsage.sqlite",
        crate::data_usage::parse,
    );
}

#[test]
fn extractor_matrix_whatsapp() {
    assert_parses_both(
        "whatsapp",
        &[
            ("ZWAMESSAGE", &["Z_PK", "ZMESSAGEDATE", "ZCHATSESSION", "ZMEDIAITEM"]),
            ("ZWACHATSESSION", &["Z_PK", "ZPARTNERNAME"]),
            ("ZWAMEDIAITEM", &["Z_PK", "ZMEDIALOCALPATH"]),
        ],
        &[
            ("ZWAMESSAGE", &["Z_PK", "ZMESSAGEDATE", "ZCHATSESSION", "ZMEDIAITEM", "ZTEXT", "ZISFROMME", "ZFROMJID"]),
            ("ZWACHATSESSION", &["Z_PK", "ZPARTNERNAME"]),
            ("ZWAMEDIAITEM", &["Z_PK", "ZMEDIALOCALPATH"]),
        ],
        "ChatStorage.sqlite",
        crate::whatsapp::parse,
    );
}

#[test]
fn extractor_matrix_voice_memos() {
    assert_parses_both(
        "voice-memos",
        &[("ZCLOUDRECORDING", &["Z_PK", "ZDATE", "ZDURATION"])],
        &[("ZCLOUDRECORDING", &["Z_PK", "ZDATE", "ZDURATION", "ZCUSTOMLABEL", "ZENCRYPTEDTITLE", "ZPATH"])],
        "CloudRecordings.db",
        crate::voice_memos::parse,
    );
}

#[test]
fn extractor_matrix_device_usage() {
    let tables: &[(&str, &[&str])] = &[("ZOBJECT", &["Z_PK", "ZSTREAMNAME", "ZVALUESTRING", "ZSTARTDATE", "ZENDDATE"])];
    assert_parses_both("device-usage", tables, tables, "knowledgeC.db", crate::device_usage::parse);
}

#[test]
fn extractor_matrix_interactions() {
    assert_parses_both(
        "interactions",
        &[("ZINTERACTIONS", &["Z_PK", "ZSENDER"]), ("ZCONTACTS", &["Z_PK"])],
        &[
            ("ZINTERACTIONS", &["Z_PK", "ZSENDER", "ZDIRECTION", "ZBUNDLEID", "ZSTARTDATE"]),
            ("ZCONTACTS", &["Z_PK", "ZIDENTIFIER", "ZNAME", "ZDISPLAYNAME"]),
        ],
        "interactionC.db",
        crate::interactions::parse,
    );
}

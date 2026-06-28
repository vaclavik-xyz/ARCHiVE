//! Read Apple Reminders from an iOS Reminders Core Data store.
//!
//! The modern store lives in the `AppDomainGroup-group.com.apple.reminders`
//! app-group container at a path with a dynamic UUID, e.g.
//! `…/Stores/Data-<UUID>.sqlite`, so the file cannot be named ahead of time —
//! the controller locates it with [`DOMAIN`] + [`is_store_path`].
//!
//! Reminders is Core Data, so tables are `Z`-prefixed and the layout is
//! **version-dependent**:
//!
//! * *Single-table inheritance* (modern): one `ZREMCDOBJECT` table holds both
//!   reminders and lists, distinguished by the `Z_ENT` discriminator. Because
//!   several entities share the table, colliding attribute names are
//!   de-duplicated with integer suffixes (`ZTITLE1`, `ZNAME2`, …).
//! * *Separate entity tables* (older/macOS-style): `ZREMCDREMINDER` holds
//!   reminders, `ZREMCDLIST`/`ZREMCDBASELIST` hold lists.
//!
//! Rather than hard-code one schema version, we discover the reminder and list
//! tables at runtime by scanning `sqlite_master` and matching tables by their
//! characteristic columns (a title + completed-flag set marks the reminder
//! table; a name-stem column marks the list table). Each version-dependent
//! column is selected only when present, else `NULL`. `Z*DATE` columns are
//! Cocoa/2001-epoch REAL seconds, converted via the datetime helpers.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// Backup domain (app-group container) that holds the Reminders store.
pub const DOMAIN: &str = "AppDomainGroup-group.com.apple.reminders";

/// One reminder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reminder {
    /// Owning list/collection name; empty when unresolved.
    pub list: String,
    /// Reminder title; empty when unset.
    pub title: String,
    /// Free-text notes; empty when none.
    pub notes: String,
    /// Due date as ISO 8601 UTC (Cocoa epoch); `None` when unset/unconvertible.
    pub due: Option<String>,
    /// Whether the reminder is completed.
    pub completed: bool,
    /// Completion date as ISO 8601 UTC (Cocoa epoch); `None` when unset.
    pub completed_date: Option<String>,
    /// Raw priority integer (0 = none; Apple uses 1/5/9 for high/medium/low).
    pub priority: i64,
    /// Creation date as ISO 8601 UTC (Cocoa epoch); `None` when unset.
    pub created: Option<String>,
    /// Whether the reminder is flagged; `false` when the column is absent.
    pub flagged: bool,
}

/// Whether `relative_path` (a backup entry path, relative to the domain) points
/// at the Reminders Core Data store file.
///
/// The store filename carries a dynamic UUID, so we match by shape rather than
/// an exact name: a path whose basename starts with `Data-` and ends with
/// `.sqlite` (not the `-wal`/`-shm` sidecars), sitting under a `Stores/`
/// directory. The intermediate container dir is versioned (`Container_v1`, …),
/// so it is not part of the test.
pub fn is_store_path(relative_path: &str) -> bool {
    // Normalize separators defensively; backups use '/'.
    let path = relative_path.replace('\\', "/");
    let Some(file) = path.rsplit('/').next() else {
        return false;
    };
    let under_stores = path.contains("Stores/");
    under_stores
        && file.starts_with("Data-")
        && file.ends_with(".sqlite")
        && !file.ends_with("-wal")
        && !file.ends_with("-shm")
}

/// Parse every reminder, resolving each one's owning list name and ordering by
/// creation date when that column exists.
///
/// Schema-tolerant: the reminder and list tables are discovered at runtime, and
/// every version-dependent column is selected only when present. A store with
/// no recognizable reminder table yields an empty `Vec` rather than an error.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Reminder>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let z_tables = z_tables(&conn)?;
    let Some(reminder_tbl) = find_reminder_table(&conn, &z_tables)? else {
        return Ok(Vec::new());
    };
    let rcols = table_columns(&conn, &reminder_tbl)?;

    // List table + its name column, resolved separately so a single-table store
    // (reminder_tbl == list_tbl) and a split store both work.
    let list_info = find_list_table(&conn, &z_tables)?;

    // Pick the best-matching column for each version-dependent field by stem.
    let title_sel = pick(&rcols, &["ZTITLE", "ZTITLE1", "ZTITLE2", "ZNAME"]);
    let notes_sel = pick(&rcols, &["ZNOTES", "ZNOTES1", "ZNOTES2"]);
    let due_sel = pick(&rcols, &["ZDUEDATE", "ZDUEDATE1", "ZDUEDATEDATE"]);
    let completed_sel = pick(&rcols, &["ZCOMPLETED", "ZCOMPLETED1", "ZISCOMPLETED"]);
    let completed_date_sel = pick(
        &rcols,
        &["ZCOMPLETIONDATE", "ZCOMPLETEDDATE", "ZCOMPLETIONDATE1", "ZCOMPLETEDDATE1"],
    );
    let priority_sel = pick(&rcols, &["ZPRIORITY", "ZPRIORITY1"]);
    let created_sel = pick(&rcols, &["ZCREATIONDATE", "ZCREATIONDATE1", "ZCREATIONDATE2"]);
    let flagged_sel = pick(&rcols, &["ZFLAGGED", "ZFLAGGED1"]);

    // The reminder→list foreign key (a Z_PK of a row in the list table). Several
    // candidate columns exist across versions; use the first present one.
    let list_fk = pick(&rcols, &["ZLIST", "ZPARENTLIST", "ZLIST1"]);

    // Resolve the list name via a self/standard join when we have both the FK
    // and a list table with a name column; else select NULL.
    let (list_sel, list_join) = match (&list_fk, &list_info) {
        (Some(fk), Some(li)) => (
            format!("l.{}", li.name_col),
            format!("LEFT JOIN {} l ON l.Z_PK = r.{}", li.table, fk),
        ),
        _ => ("NULL".to_string(), String::new()),
    };

    // For a single-table store, restrict rows to actual reminders so list rows
    // (which share the table) are not emitted as reminders. We identify reminder
    // rows by their Z_ENT discriminator, resolved from Z_PRIMARYKEY by name.
    let ent_filter = reminder_entity_filter(&conn, &z_tables, &reminder_tbl, &rcols)?;

    let order = match &created_sel {
        Some(c) => format!("ORDER BY r.{c}"),
        None => String::new(),
    };

    let sql = format!(
        "SELECT {list}, {title}, {notes}, {due}, {completed}, {cdate}, {prio}, {created}, {flagged} \
         FROM {table} r {join} {filter} {order}",
        list = list_sel,
        title = col(&title_sel, "r"),
        notes = col(&notes_sel, "r"),
        due = col(&due_sel, "r"),
        completed = col(&completed_sel, "r"),
        cdate = col(&completed_date_sel, "r"),
        prio = col(&priority_sel, "r"),
        created = col(&created_sel, "r"),
        flagged = col(&flagged_sel, "r"),
        table = reminder_tbl,
        join = list_join,
        filter = ent_filter,
        order = order,
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let list: Option<String> = row.get(0)?;
        let title: Option<String> = row.get(1)?;
        let notes: Option<String> = row.get(2)?;
        let due: Option<f64> = row.get(3)?;
        let completed: Option<i64> = row.get(4)?;
        let completed_date: Option<f64> = row.get(5)?;
        let priority: Option<i64> = row.get(6)?;
        let created: Option<f64> = row.get(7)?;
        let flagged: Option<i64> = row.get(8)?;
        Ok(Reminder {
            list: list.unwrap_or_default(),
            title: title.unwrap_or_default(),
            notes: notes.unwrap_or_default(),
            due: due.and_then(cocoa_to_iso),
            completed: completed == Some(1),
            completed_date: completed_date.and_then(cocoa_to_iso),
            priority: priority.unwrap_or(0),
            created: created.and_then(cocoa_to_iso),
            flagged: flagged == Some(1),
        })
    })?;
    rows.collect()
}

/// A discovered list table and the column holding its display name.
struct ListInfo {
    table: String,
    name_col: String,
}

/// All `Z`-prefixed user tables in the store (skips Core Data's own `Z_*`
/// metadata tables like `Z_PRIMARYKEY` / `Z_METADATA`).
fn z_tables(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(names
        .into_iter()
        .filter(|n| n.starts_with('Z') && !n.starts_with("Z_"))
        .collect())
}

/// Find the table that stores reminders: the one carrying a title-stem column
/// **and** a completed-flag (or due-date) column — the characteristic reminder
/// column set. Prefers a dedicated `ZREMCDREMINDER`-style table when it exists,
/// else falls back to the shared `ZREMCDOBJECT`-style table. Returns `None`
/// when none qualifies.
fn find_reminder_table(
    conn: &Connection,
    z_tables: &[String],
) -> rusqlite::Result<Option<String>> {
    let mut best: Option<String> = None;
    for t in z_tables {
        let cols = table_columns(conn, t)?;
        let has_title = has_any(&cols, &["ZTITLE", "ZTITLE1", "ZTITLE2"]);
        let has_completed = has_any(&cols, &["ZCOMPLETED", "ZISCOMPLETED", "ZCOMPLETED1"]);
        let has_due = has_any(&cols, &["ZDUEDATE", "ZDUEDATE1"]);
        // A reminder row has a title plus a completion notion (and usually a due
        // column). Require title + (completed or due) to avoid matching lists.
        if has_title && (has_completed || has_due) {
            // Prefer the canonical reminder entity name when several qualify.
            if t.to_ascii_uppercase().contains("REMINDER") {
                return Ok(Some(t.clone()));
            }
            best.get_or_insert_with(|| t.clone());
        }
    }
    Ok(best)
}

/// Find the table + column holding list names. Prefers a dedicated list entity
/// table (`ZREMCD*LIST`); otherwise falls back to the shared object table when
/// it carries a name-stem column (single-table inheritance). Returns `None`
/// when no list name is resolvable.
fn find_list_table(
    conn: &Connection,
    z_tables: &[String],
) -> rusqlite::Result<Option<ListInfo>> {
    const NAME_STEMS: [&str; 5] = ["ZNAME", "ZNAME1", "ZNAME2", "ZNAME3", "ZTITLE"];
    let mut fallback: Option<ListInfo> = None;
    for t in z_tables {
        let cols = table_columns(conn, t)?;
        let Some(name_col) = pick(&cols, &NAME_STEMS) else { continue };
        let upper = t.to_ascii_uppercase();
        // A dedicated list table is the strongest match.
        if upper.contains("LIST") {
            return Ok(Some(ListInfo { table: t.clone(), name_col }));
        }
        // Otherwise keep the shared object table as a fallback (single-table).
        if upper.contains("OBJECT") || upper.contains("REMCD") {
            fallback.get_or_insert(ListInfo { table: t.clone(), name_col });
        }
    }
    Ok(fallback)
}

/// Build a `WHERE r.Z_ENT IN (…)` clause restricting a single-table store to
/// reminder rows, or an empty string when no restriction is needed/possible.
///
/// Only applied when the reminder table also serves as the list table (i.e. a
/// shared `ZREMCDOBJECT`): there, list rows live in the same table and would
/// otherwise surface as title-less "reminders". We resolve the reminder
/// entity's `Z_ENT` from `Z_PRIMARYKEY` by name (never hard-coding the integer,
/// which drifts between versions). When the table has no `Z_ENT` column or no
/// reminder entity is found, no filter is applied.
fn reminder_entity_filter(
    conn: &Connection,
    z_tables: &[String],
    reminder_tbl: &str,
    rcols: &HashSet<String>,
) -> rusqlite::Result<String> {
    if !rcols.contains("Z_ENT") {
        return Ok(String::new());
    }
    // Only meaningful for a shared table; if a dedicated list table exists the
    // reminder table is already reminders-only.
    let shared = z_tables
        .iter()
        .all(|t| !t.to_ascii_uppercase().contains("LIST") || t == reminder_tbl);
    if !shared {
        return Ok(String::new());
    }
    // Z_PRIMARYKEY maps entity name (Z_NAME) → discriminator (Z_ENT).
    let ents = reminder_entity_ids(conn)?;
    if ents.is_empty() {
        return Ok(String::new());
    }
    let list = ents.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
    Ok(format!("WHERE r.Z_ENT IN ({list})"))
}

/// Resolve `Z_ENT` values whose entity name marks a reminder, via the
/// `Z_PRIMARYKEY` map (`Z_NAME`, `Z_ENT`). Matches any entity whose name
/// contains "Reminder". Empty when `Z_PRIMARYKEY` is absent or has no match.
fn reminder_entity_ids(conn: &Connection) -> rusqlite::Result<Vec<i64>> {
    // Tolerate a missing Z_PRIMARYKEY (returns an error → treated as empty).
    let mut stmt = match conn.prepare("SELECT Z_ENT, Z_NAME FROM Z_PRIMARYKEY") {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut ids = Vec::new();
    for r in rows {
        let (ent, name) = r?;
        if let Some(n) = name
            && n.to_ascii_uppercase().contains("REMINDER")
        {
            ids.push(ent);
        }
    }
    Ok(ids)
}

/// First name in `candidates` that exists in `cols`.
fn pick(cols: &HashSet<String>, candidates: &[&str]) -> Option<String> {
    candidates.iter().find(|c| cols.contains(**c)).map(|c| c.to_string())
}

/// Whether any of `candidates` exists in `cols`.
fn has_any(cols: &HashSet<String>, candidates: &[&str]) -> bool {
    candidates.iter().any(|c| cols.contains(*c))
}

/// Render a select term for an optional column: `alias.COL` when present, else
/// `NULL` (so the projection's column count stays fixed).
fn col(name: &Option<String>, alias: &str) -> String {
    match name {
        Some(c) => format!("{alias}.{c}"),
        None => "NULL".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_reminders;

    #[test]
    fn is_store_path_matches_uuid_store_only() {
        // Realistic modern store path inside the app-group container.
        assert!(is_store_path(
            "Container_v1/Stores/Data-5070B790-D66D-40F7-8F4A-EC8E0FA88F3A.sqlite"
        ));
        assert!(is_store_path("Stores/Data-ABCD.sqlite"));
        // Sidecars and unrelated files do not match.
        assert!(!is_store_path(
            "Container_v1/Stores/Data-5070B790-D66D.sqlite-wal"
        ));
        assert!(!is_store_path("Container_v1/Stores/Data-5070B790.sqlite-shm"));
        assert!(!is_store_path("Container_v1/Stores/Model.sqlite"));
        assert!(!is_store_path("Library/Preferences/group.com.apple.reminders.plist"));
        assert!(!is_store_path(""));
    }

    #[test]
    fn parses_reminders_across_two_lists_single_table() {
        let dir = std::env::temp_dir().join(format!("be-reminders-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Data-TEST.sqlite");
        let _ = std::fs::remove_file(&db);
        make_reminders(&db);

        let mut items = parse(&db).unwrap();
        // Three reminders across two lists; the two list rows are NOT emitted.
        assert_eq!(items.len(), 3, "list rows must be filtered out by Z_ENT");

        // Order is by creation date; sort by title for deterministic assertions.
        items.sort_by(|a, b| a.title.cmp(&b.title));

        let bread = items.iter().find(|r| r.title == "Koupit chleba").unwrap();
        assert_eq!(bread.list, "Nákup");
        assert!(bread.completed);
        assert_eq!(bread.completed_date.as_deref(), Some("2020-01-06T10:41:40+00:00")); // 600_000_100
        assert_eq!(bread.due, None); // no due date set
        assert!(!bread.flagged);

        let milk = items.iter().find(|r| r.title == "Koupit mléko").unwrap();
        assert_eq!(milk.list, "Nákup");
        assert_eq!(milk.notes, "2 litry");
        assert_eq!(milk.due.as_deref(), Some("2020-01-06T10:40:00+00:00")); // Cocoa 600_000_000
        assert!(!milk.completed);
        assert_eq!(milk.completed_date, None);
        assert_eq!(milk.priority, 1);
        assert_eq!(milk.created.as_deref(), Some("2020-01-06T10:40:00+00:00"));
        assert!(milk.flagged);

        let call = items.iter().find(|r| r.title == "Zavolat doktorovi").unwrap();
        assert_eq!(call.list, "Práce");
        assert_eq!(call.priority, 9);
        assert!(!call.completed);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_when_optional_columns_absent() {
        // A minimal split store (dedicated list + reminder tables) missing
        // notes / completion-date / flagged / priority / Z_PRIMARYKEY columns
        // still parses (select-or-NULL).
        let dir = std::env::temp_dir().join(format!("be-reminders-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Data-MIN.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZREMCDLIST (Z_PK INTEGER PRIMARY KEY, ZNAME TEXT);
             CREATE TABLE ZREMCDREMINDER (Z_PK INTEGER PRIMARY KEY, ZTITLE TEXT,
                ZDUEDATE REAL, ZCOMPLETED INTEGER, ZLIST INTEGER);
             INSERT INTO ZREMCDLIST VALUES (1, 'Domov');
             INSERT INTO ZREMCDREMINDER VALUES (1, 'Uklidit', NULL, 0, 1);",
        )
        .unwrap();
        drop(conn);

        let items = parse(&db).unwrap();
        assert_eq!(items.len(), 1);
        let r = &items[0];
        assert_eq!(r.title, "Uklidit");
        assert_eq!(r.list, "Domov");
        assert_eq!(r.notes, ""); // no notes column → empty
        assert_eq!(r.due, None);
        assert!(!r.completed);
        assert_eq!(r.completed_date, None); // no completion-date column
        assert_eq!(r.priority, 0); // no priority column → 0
        assert!(!r.flagged); // no flagged column → false
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_table_returns_empty() {
        let dir = std::env::temp_dir().join(format!("be-reminders-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Data-NONE.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE ZUNRELATED (Z_PK INTEGER PRIMARY KEY, ZFOO TEXT);")
            .unwrap();
        drop(conn);
        assert!(parse(&db).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}

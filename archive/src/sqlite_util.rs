//! Small shared SQLite helpers for the store parsers.

use std::collections::HashSet;

use rusqlite::Connection;

/// Column names present in `table`, via `PRAGMA table_info`. `table` must be a
/// trusted literal — it is interpolated into the pragma, not bound.
pub fn table_columns(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<String>>>()?;
    Ok(cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_existing_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (a INTEGER, b TEXT);").unwrap();
        let cols = table_columns(&conn, "t").unwrap();
        assert!(cols.contains("a"));
        assert!(cols.contains("b"));
        assert!(!cols.contains("c"));
    }
}

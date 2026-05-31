/*!
 This module contains data structures returned by diagnostic queries on iMessage database tables.
*/

use rusqlite::Connection;

use crate::error::table::TableError;

pub(crate) fn count_query(db: &Connection, sql: &str) -> Result<usize, TableError> {
    let count = db.prepare(sql)?.query_row([], |row| row.get::<_, i64>(0))?;

    usize::try_from(count)
        .map_err(|_| TableError::QueryError(rusqlite::Error::IntegralValueOutOfRange(0, count)))
}

pub(crate) fn table_exists(db: &Connection, table_name: &str) -> Result<bool, TableError> {
    let exists = db.query_row(
        "
        SELECT EXISTS(
            SELECT 1
            FROM sqlite_master
            WHERE type = 'table'
              AND name = ?1
        )
        ",
        [table_name],
        |row| row.get::<_, i64>(0),
    )?;

    Ok(exists != 0)
}

pub(crate) fn column_exists(
    db: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, TableError> {
    let mut statement = db.prepare(&format!(
        "PRAGMA table_info({})",
        quote_sqlite_identifier(table_name)
    ))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;

    for column in columns {
        if column? == column_name {
            return Ok(true);
        }
    }

    Ok(false)
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Diagnostic data for the `handle` table
#[derive(Debug)]
pub struct HandleDiagnostic {
    /// The total number of handles in the table
    pub total_handles: usize,
    /// The number of distinct `person_centric_id` values in the handle table, or `None` if the
    /// column is unavailable on this database schema
    pub handles_with_multiple_ids: Option<usize>,
    /// The number of handles that were deduplicated into canonical handles
    pub total_duplicated: usize,
}

/// Diagnostic data for the `message` table
#[derive(Debug)]
pub struct MessageDiagnostic {
    /// The total number of messages in the table
    pub total_messages: usize,
    /// The number of messages not associated with any chat
    pub messages_without_chat: usize,
    /// The number of messages that belong to more than one chat
    pub messages_in_multiple_chats: usize,
    /// The number of recently deleted messages that are still recoverable, or `None` if the
    /// recoverable messages table is unavailable on this database schema
    pub recoverable_messages: Option<usize>,
    /// The raw `date` value of the earliest message, or `None` if the table is empty
    pub first_message_date: Option<i64>,
    /// The raw `date` value of the most recent message, or `None` if the table is empty
    pub last_message_date: Option<i64>,
}

/// Diagnostic data for the `attachment` table
#[derive(Debug)]
pub struct AttachmentDiagnostic {
    /// The total number of attachments in the table
    pub total_attachments: usize,
    /// The sum of `total_bytes` for all attachments referenced in the table
    pub total_bytes_referenced: u64,
    /// The total size of attachment files present on disk
    pub total_bytes_on_disk: u64,
    /// The number of attachments with missing files (no path or file not found)
    pub missing_files: usize,
    /// The number of attachments with no path provided in the table
    pub no_path_provided: usize,
}

impl AttachmentDiagnostic {
    /// The number of attachments where a path was provided but no file was found at that location
    #[must_use]
    pub fn no_file_located(&self) -> usize {
        self.missing_files.saturating_sub(self.no_path_provided)
    }

    /// The percentage of attachments that are missing, or `None` if there are no attachments
    #[must_use]
    pub fn missing_percent(&self) -> Option<f64> {
        if self.total_attachments > 0 {
            Some(self.missing_files as f64 / self.total_attachments as f64 * 100.0)
        } else {
            None
        }
    }
}

/// Diagnostic data for chat-handle relationships (thread/chat deduplication)
#[derive(Debug)]
pub struct ChatHandleDiagnostic {
    /// The total number of chats in the table
    pub total_chats: usize,
    /// The number of chats that were deduplicated
    pub total_duplicated: usize,
    /// The number of chats that have messages but no associated handles
    pub chats_with_no_handles: usize,
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{column_exists, table_exists};

    #[test]
    fn table_exists_detects_existing_and_missing_tables() {
        let db = Connection::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_table (id INTEGER)", [])
            .unwrap();

        assert!(table_exists(&db, "test_table").unwrap());
        assert!(!table_exists(&db, "missing_table").unwrap());
    }

    #[test]
    fn column_exists_detects_existing_and_missing_columns() {
        let db = Connection::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_table (id INTEGER, name TEXT)", [])
            .unwrap();

        assert!(column_exists(&db, "test_table", "name").unwrap());
        assert!(!column_exists(&db, "test_table", "missing_column").unwrap());
        assert!(!column_exists(&db, "missing_table", "name").unwrap());
    }

    #[test]
    fn column_exists_quotes_table_identifiers() {
        let db = Connection::open_in_memory().unwrap();
        db.execute(
            "CREATE TABLE \"quoted\"\"table\" (\"weird column\" TEXT)",
            [],
        )
        .unwrap();

        assert!(column_exists(&db, "quoted\"table", "weird column").unwrap());
    }
}

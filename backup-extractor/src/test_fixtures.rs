//! Builders for synthetic iOS data-store fixtures used in tests.

use std::path::Path;

use rusqlite::Connection;

/// Build a minimal real `AddressBook.sqlitedb` with two contacts:
/// "Jan Novák" (Acme, mobile + home email) and a company-only row "Firma s.r.o.".
pub fn make_addressbook(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT, Note TEXT);
         CREATE TABLE ABMultiValueLabel (ROWID INTEGER PRIMARY KEY, value TEXT);
         CREATE TABLE ABMultiValue (UID INTEGER PRIMARY KEY, record_id INTEGER, property INTEGER, label INTEGER, value TEXT);
         INSERT INTO ABMultiValueLabel (ROWID, value) VALUES (1, '_$!<Mobile>!$_'), (2, '_$!<Home>!$_');
         INSERT INTO ABPerson (ROWID, First, Last, Organization, Note) VALUES
            (1, 'Jan', 'Novák', 'Acme', 'kamarád'),
            (2, NULL, NULL, 'Firma s.r.o.', NULL);
         INSERT INTO ABMultiValue (UID, record_id, property, label, value) VALUES
            (1, 1, 3, 1, '+420776452878'),
            (2, 1, 4, 2, 'jan@example.cz');",
    )
    .unwrap();
}

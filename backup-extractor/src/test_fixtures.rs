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

/// Build a minimal real `voicemail.db`: an active voicemail (Unix date, not
/// trashed) and a trashed one (Cocoa `trashed_date`, withheld/NULL sender). The
/// mixed epochs are intentional — `date` is Unix, `trashed_date` is Cocoa 2001.
pub fn make_voicemail(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE voicemail (
            ROWID INTEGER PRIMARY KEY, remote_uid INTEGER, date INTEGER, token TEXT,
            sender TEXT, callback_num TEXT, duration INTEGER, expiration INTEGER,
            trashed_date INTEGER, flags INTEGER);
         INSERT INTO voicemail (ROWID, date, sender, duration, expiration, trashed_date, flags) VALUES
            (1, 1600000000, '+420776452878', 30, 1600086400, 0, 0),
            (2, 1600000100, NULL, 12, 0, 600000000, 75);",
    )
    .unwrap();
}

/// Build a minimal real `CallHistory.storedata` (`ZCALLRECORD`): an outgoing,
/// answered phone call (cocoa date 100, 42s, CZ) and an incoming, missed
/// FaceTime-video call (cocoa date 50). `ZADDRESS` is stored as a real BLOB.
pub fn make_callhistory(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZCALLRECORD (
            Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL, ZADDRESS BLOB,
            ZORIGINATED INTEGER, ZANSWERED INTEGER, ZCALLTYPE INTEGER,
            ZSERVICE_PROVIDER TEXT, ZLOCATION TEXT, ZISO_COUNTRY_CODE TEXT);
         INSERT INTO ZCALLRECORD VALUES
            (1, 100.0, 42.0, CAST('+420776452878' AS BLOB), 1, 1, 1, 'com.apple.Telephony', NULL, 'cz'),
            (2, 50.0, 0.0, CAST('jana@example.cz' AS BLOB), 0, 0, 8, 'com.apple.FaceTime', NULL, NULL);",
    )
    .unwrap();
}

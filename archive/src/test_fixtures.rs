//! Builders for synthetic iOS data-store fixtures used in tests.

use std::path::Path;

use rusqlite::Connection;

/// Build a minimal real `AddressBook.sqlitedb` with two contacts:
/// "Jan Novák" (Acme, mobile + home email + work address) and a company-only row "Firma s.r.o.".
/// Note: the address row has ROWID=10 but UID=3, so tests exercise the `parent_id = UID` join.
pub fn make_addressbook(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT, Note TEXT);
         CREATE TABLE ABMultiValueLabel (ROWID INTEGER PRIMARY KEY, value TEXT);
         CREATE TABLE ABMultiValue (ROWID INTEGER PRIMARY KEY, UID INTEGER, record_id INTEGER, property INTEGER, label INTEGER, value TEXT);
         CREATE TABLE ABMultiValueEntryKey (ROWID INTEGER PRIMARY KEY, value TEXT);
         CREATE TABLE ABMultiValueEntry (ROWID INTEGER PRIMARY KEY, parent_id INTEGER, key INTEGER, value TEXT);
         INSERT INTO ABMultiValueLabel (ROWID, value) VALUES (1, '_$!<Mobile>!$_'), (2, '_$!<Home>!$_'), (3, '_$!<Work>!$_');
         INSERT INTO ABMultiValueEntryKey (ROWID, value) VALUES (1,'Street'),(2,'State'),(3,'ZIP'),(4,'City'),(5,'CountryCode'),(8,'Country');
         INSERT INTO ABPerson (ROWID, First, Last, Organization, Note) VALUES
            (1, 'Jan', 'Novák', 'Acme', 'kamarád'),
            (2, NULL, NULL, 'Firma s.r.o.', NULL);
         INSERT INTO ABMultiValue (ROWID, UID, record_id, property, label, value) VALUES
            (1, 101, 1, 3, 1, '+420776452878'),
            (2, 102, 1, 4, 2, 'jan@example.cz'),
            (10, 3, 1, 5, 3, NULL);
         INSERT INTO ABMultiValueEntry (ROWID, parent_id, key, value) VALUES
            (1, 3, 1, 'Hlavní 1'),
            (2, 3, 4, 'Praha'),
            (3, 3, 3, '11000'),
            (4, 3, 8, 'Czechia');",
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

/// Build a minimal real WhatsApp `ChatStorage.sqlite`: one chat session, a media
/// item, and three messages (from-me text, incoming media, from-me text). Cocoa
/// `ZMESSAGEDATE` seconds.
pub fn make_whatsapp(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZWACHATSESSION (Z_PK INTEGER PRIMARY KEY, ZPARTNERNAME TEXT, ZCONTACTJID TEXT);
         CREATE TABLE ZWAMEDIAITEM (Z_PK INTEGER PRIMARY KEY, ZMESSAGE INTEGER, ZMEDIALOCALPATH TEXT);
         CREATE TABLE ZWAMESSAGE (Z_PK INTEGER PRIMARY KEY, ZTEXT TEXT, ZMESSAGEDATE REAL,
            ZISFROMME INTEGER, ZFROMJID TEXT, ZCHATSESSION INTEGER, ZMEDIAITEM INTEGER);
         INSERT INTO ZWACHATSESSION VALUES (1, 'Jana', '420776452878@s.whatsapp.net');
         INSERT INTO ZWAMEDIAITEM VALUES (1, 2, 'Media/420776452878@s.whatsapp.net/7/d/photo.jpg');
         INSERT INTO ZWAMESSAGE VALUES
            (1, 'Ahoj', 600000000.0, 1, NULL, 1, NULL),
            (2, NULL, 600000100.0, 0, '420776452878@s.whatsapp.net', 1, 1),
            (3, 'Měj se', 600000200.0, 1, NULL, 1, NULL);",
    )
    .unwrap();
}

/// Build a minimal real Messages `sms.db` `attachment` table (nanosecond Cocoa
/// dates, as modern iOS uses): an iMessage image (`~/Library/Messages/…`), an
/// SMS video (`~/Library/SMS/…`), and a row whose filename has no recoverable
/// Attachments path.
pub fn make_sms_attachments(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE attachment (ROWID INTEGER PRIMARY KEY, filename TEXT, mime_type TEXT,
            transfer_name TEXT, total_bytes INTEGER, created_date INTEGER);
         INSERT INTO attachment VALUES
            (1, '~/Library/Messages/Attachments/ab/12/GUID1/photo.jpg', 'image/jpeg', 'photo.jpg', 102400, 600000000000000000),
            (2, '~/Library/SMS/Attachments/cd/34/GUID2/clip.mov', 'video/quicktime', 'clip.mov', 2048000, 600000100000000000),
            (3, '/some/weird/path/nofile.bin', 'application/octet-stream', 'nofile.bin', 100, 600000200000000000);",
    )
    .unwrap();
}

/// Build a minimal real `Photos.sqlite` (`ZASSET`): a favorited photo with GPS,
/// a video with a duration and the `-180` no-location sentinel, and a trashed
/// photo with NULL coordinates. Cocoa `ZDATECREATED`.
pub fn make_photos(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZFILENAME TEXT, ZDIRECTORY TEXT,
            ZDATECREATED REAL, ZKIND INTEGER, ZFAVORITE INTEGER, ZTRASHEDSTATE INTEGER,
            ZWIDTH INTEGER, ZHEIGHT INTEGER, ZLATITUDE REAL, ZLONGITUDE REAL, ZDURATION REAL);
         INSERT INTO ZASSET VALUES
            (1, 'IMG_0001.HEIC', 'DCIM/100APPLE', 600000000.0, 0, 1, 0, 4032, 3024, 50.087, 14.42, NULL),
            (2, 'IMG_0002.MOV', 'DCIM/100APPLE', 600000100.0, 1, 0, 0, 1920, 1080, -180.0, -180.0, 12.5),
            (3, 'IMG_0003.JPG', 'DCIM/100APPLE', 600000200.0, 0, 0, 1, 3024, 4032, NULL, NULL, NULL);",
    )
    .unwrap();
}

/// Build a minimal real Apple `NoteStore.sqlite`: one folder, and two notes —
/// the first with the provided gzip-protobuf body blob, the second with no blob
/// (so the snippet fallback is exercised). Cocoa dates.
pub fn make_notes(path: &Path, note1_zdata: &[u8]) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZICNOTEDATA (Z_PK INTEGER PRIMARY KEY, ZNOTE INTEGER, ZDATA BLOB);
         CREATE TABLE ZICCLOUDSYNCINGOBJECT (Z_PK INTEGER PRIMARY KEY, ZTITLE1 TEXT, ZTITLE2 TEXT,
            ZSNIPPET TEXT, ZNOTEDATA INTEGER, ZFOLDER INTEGER, ZCREATIONDATE REAL, ZMODIFICATIONDATE1 REAL);
         INSERT INTO ZICCLOUDSYNCINGOBJECT (Z_PK, ZTITLE2) VALUES (10, 'Práce');
         INSERT INTO ZICNOTEDATA (Z_PK, ZNOTE, ZDATA) VALUES (2, 101, NULL);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ZICNOTEDATA (Z_PK, ZNOTE, ZDATA) VALUES (1, 100, ?1)",
        rusqlite::params![note1_zdata],
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO ZICCLOUDSYNCINGOBJECT (Z_PK, ZTITLE1, ZSNIPPET, ZNOTEDATA, ZFOLDER, ZCREATIONDATE, ZMODIFICATIONDATE1) VALUES
            (100, 'Nákup', 'snippet1', 1, 10, 600000000.0, 600000500.0),
            (101, 'Druhá', 'jen náhled', 2, 10, 600000100.0, 600000600.0);",
    )
    .unwrap();
}

/// Build a minimal real Safari `History.db`: two history items each with one
/// visit (Cocoa `visit_time`), so the items↔visits join can be exercised.
pub fn make_safari_history(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE history_items (id INTEGER PRIMARY KEY, url TEXT, visit_count INTEGER);
         CREATE TABLE history_visits (id INTEGER PRIMARY KEY, history_item INTEGER, visit_time REAL, title TEXT);
         INSERT INTO history_items VALUES (1, 'https://apple.com', 5), (2, 'https://bbc.com', 2);
         INSERT INTO history_visits VALUES
            (1, 1, 600000000.0, 'Apple'),
            (2, 2, 600000100.0, 'BBC News');",
    )
    .unwrap();
}

/// Build a minimal real Safari `Bookmarks.db`: two folders and two leaf
/// bookmarks, so the parent→folder-name resolution and leaf filtering run.
pub fn make_safari_bookmarks(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE bookmarks (id INTEGER PRIMARY KEY, title TEXT, url TEXT, parent INTEGER, type INTEGER);
         INSERT INTO bookmarks VALUES
            (1, 'Favorites', NULL, NULL, 1),
            (2, 'Apple', 'https://apple.com', 1, 0),
            (3, 'News', NULL, 1, 1),
            (4, 'BBC', 'https://bbc.com', 3, 0);",
    )
    .unwrap();
}

/// Build a minimal real `Calendar.sqlitedb`: two calendars and two events
/// (Cocoa dates), one all-day, so the CalendarItem↔Calendar join is covered.
pub fn make_calendar(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE Calendar (ROWID INTEGER PRIMARY KEY, title TEXT);
         CREATE TABLE CalendarItem (ROWID INTEGER PRIMARY KEY, summary TEXT, start_date REAL,
            end_date REAL, all_day INTEGER, calendar_id INTEGER);
         INSERT INTO Calendar VALUES (1, 'Work'), (2, 'Home');
         INSERT INTO CalendarItem VALUES
            (1, 'Standup', 600000000.0, 600001800.0, 0, 1),
            (2, 'Holiday', 600100000.0, 600186400.0, 1, 2);",
    )
    .unwrap();
}

/// Build a minimal real Voice Memos `CloudRecordings.db` (`ZCLOUDRECORDING`):
/// a titled memo and an untitled one. `ZDATE` is the Cocoa/2001 epoch.
pub fn make_voicememos(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZCLOUDRECORDING (
            Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL,
            ZCUSTOMLABEL TEXT, ZENCRYPTEDTITLE TEXT, ZPATH TEXT);
         INSERT INTO ZCLOUDRECORDING (Z_PK, ZDATE, ZDURATION, ZCUSTOMLABEL, ZPATH) VALUES
            (1, 600000000.0, 12.5, 'Schůzka', '20200101 120000.m4a'),
            (2, 600000100.0, 3.0, NULL, 'A1B2C3.m4a');",
    )
    .unwrap();
}

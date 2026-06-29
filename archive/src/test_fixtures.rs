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

/// Build a minimal real `Photos.sqlite` exercising the enriched fields: a hidden,
/// edited, Live-Photo image with a title/original name in two albums; a video
/// sharing a burst UUID with a trashed image. Includes the
/// `ZADDITIONALASSETATTRIBUTES` join, `ZGENERICALBUM`, and a dynamic `Z_28ASSETS`
/// album↔asset join table (number is version-dependent in real DBs).
pub fn make_photos(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZFILENAME TEXT, ZDIRECTORY TEXT,
            ZDATECREATED REAL, ZMODIFICATIONDATE REAL, ZADDEDDATE REAL, ZKIND INTEGER,
            ZKINDSUBTYPE INTEGER, ZFAVORITE INTEGER, ZHIDDEN INTEGER, ZTRASHEDSTATE INTEGER,
            ZHASADJUSTMENTS INTEGER, ZWIDTH INTEGER, ZHEIGHT INTEGER, ZLATITUDE REAL,
            ZLONGITUDE REAL, ZDURATION REAL, ZAVALANCHEUUID TEXT, ZADDITIONALATTRIBUTES INTEGER,
            ZTRASHEDDATE REAL);
         INSERT INTO ZASSET VALUES
            (1, 'IMG_0001.HEIC', 'DCIM/100APPLE', 600000000.0, 600000050.0, 600000010.0, 0, 2, 1, 1, 0, 1, 4032, 3024, 50.087, 14.42, NULL, NULL, 1, NULL),
            (2, 'IMG_0002.MOV', 'DCIM/100APPLE', 600000100.0, NULL, NULL, 1, 0, 0, 0, 0, 0, 1920, 1080, -180.0, -180.0, 12.5, 'BURST1', NULL, NULL),
            (3, 'IMG_0003.JPG', 'DCIM/100APPLE', 600000200.0, NULL, NULL, 0, 0, 0, 0, 1, 0, 3024, 4032, NULL, NULL, NULL, 'BURST1', NULL, 600000300.0);

         CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY, ZORIGINALFILENAME TEXT, ZTITLE TEXT);
         INSERT INTO ZADDITIONALASSETATTRIBUTES VALUES (1, 'IMG_E0001.HEIC', 'Západ slunce');

         CREATE TABLE ZGENERICALBUM (Z_PK INTEGER PRIMARY KEY, ZTITLE TEXT, ZKIND INTEGER);
         INSERT INTO ZGENERICALBUM VALUES (1, 'Dovolená', 2), (2, 'Rodina', 2), (3, NULL, 2);

         CREATE TABLE Z_28ASSETS (Z_28ALBUMS INTEGER, Z_3ASSETS INTEGER);
         INSERT INTO Z_28ASSETS VALUES (1, 1), (2, 1), (3, 1);",
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

/// Build a minimal real modern Reminders Core Data store (`Data-<UUID>.sqlite`)
/// using single-table inheritance: one shared `ZREMCDOBJECT` holds both lists
/// and reminders, distinguished by the `Z_ENT` discriminator and de-duplicated
/// suffixed columns (`ZTITLE1` for reminder titles, `ZNAME2` for list names).
/// A `Z_PRIMARYKEY` maps entity names to `Z_ENT` so the parser resolves the
/// reminder discriminator by name (not a hard-coded integer). Two lists and
/// three reminders (open with due+flagged, completed, high-priority open),
/// across both lists. `Z*DATE` columns are the Cocoa/2001 epoch.
pub fn make_reminders(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        // Core Data discriminator map. Z_ENT integers are arbitrary here on
        // purpose — the parser must read them from Z_PRIMARYKEY by name.
        "CREATE TABLE Z_PRIMARYKEY (Z_ENT INTEGER PRIMARY KEY, Z_NAME TEXT, Z_SUPER INTEGER, Z_MAX INTEGER);
         INSERT INTO Z_PRIMARYKEY (Z_ENT, Z_NAME) VALUES
            (7, 'REMCDReminder'), (25, 'REMCDList'), (30, 'REMCDSmartList');

         CREATE TABLE ZREMCDOBJECT (
            Z_PK INTEGER PRIMARY KEY, Z_ENT INTEGER,
            ZTITLE1 TEXT, ZNOTES TEXT, ZDUEDATE REAL, ZCOMPLETED INTEGER,
            ZCOMPLETIONDATE REAL, ZPRIORITY INTEGER, ZCREATIONDATE REAL,
            ZFLAGGED INTEGER, ZLIST INTEGER, ZNAME2 TEXT);

         -- Two lists (Z_ENT 25): names live in ZNAME2.
         INSERT INTO ZREMCDOBJECT (Z_PK, Z_ENT, ZNAME2) VALUES
            (1, 25, 'Nákup'),
            (2, 25, 'Práce');

         -- Three reminders (Z_ENT 7): titles in ZTITLE1, list FK in ZLIST.
         INSERT INTO ZREMCDOBJECT
            (Z_PK, Z_ENT, ZTITLE1, ZNOTES, ZDUEDATE, ZCOMPLETED, ZCOMPLETIONDATE, ZPRIORITY, ZCREATIONDATE, ZFLAGGED, ZLIST) VALUES
            (10, 7, 'Koupit mléko', '2 litry', 600000000.0, 0, NULL, 1, 600000000.0, 1, 1),
            (11, 7, 'Koupit chleba', NULL, NULL, 1, 600000100.0, 0, 600000050.0, 0, 1),
            (12, 7, 'Zavolat doktorovi', NULL, NULL, 0, NULL, 9, 600000200.0, 0, 2);",
    )
    .unwrap();
}

/// Build a minimal real `Accounts3.sqlite` (Core Data): an active iCloud account
/// (cocoa `ZDATE`), an active Google/Gmail account owned by a third-party bundle,
/// and an inactive iCloud-type account with no username/description/date (so the
/// type-join fallback and empty-field handling are exercised).
pub fn make_accounts(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZACCOUNTTYPEDESCRIPTION TEXT, ZIDENTIFIER TEXT);
         CREATE TABLE ZACCOUNT (Z_PK INTEGER PRIMARY KEY, ZACCOUNTTYPE INTEGER, ZACTIVE INTEGER,
            ZDATE REAL, ZIDENTIFIER TEXT, ZACCOUNTDESCRIPTION TEXT, ZUSERNAME TEXT, ZOWNINGBUNDLEID TEXT);
         INSERT INTO ZACCOUNTTYPE VALUES
            (1, 'iCloud', 'com.apple.account.iCloud'),
            (2, 'Google', 'com.google.account');
         INSERT INTO ZACCOUNT (Z_PK, ZACCOUNTTYPE, ZACTIVE, ZDATE, ZIDENTIFIER, ZACCOUNTDESCRIPTION, ZUSERNAME, ZOWNINGBUNDLEID) VALUES
            (1, 1, 1, 600000000.0, 'uuid-1', 'iCloud', 'jane@icloud.com', NULL),
            (2, 2, 1, 600000100.0, 'uuid-2', 'Gmail', 'jane@gmail.com', 'com.google.Gmail'),
            (3, 1, 0, NULL, 'uuid-3', NULL, NULL, NULL);",
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

#!/usr/bin/env python3
"""Generate a synthetic unencrypted iOS backup for the ARCHiVE UI demo.

Replicates the minimal store schemas used by the Rust test fixtures, wrapped in
a real iTunes-backup envelope (Info.plist / Status.plist / Manifest.db) that
crabapple accepts. The camera roll includes one Manifest.db-only DCIM file so
the `uncatalogued` reconciliation has something to show.
"""
import base64
import hashlib
import os
import plistlib
import shutil
import sqlite3
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/ARCHiVE-demo-zaloha")
DOMAIN = "CameraRollDomain"

# 1x1 blue JPEG (bytes) for demo gallery images.
JPEG = base64.b64decode(
    "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0a"
    "HBwcJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPDs0NDT/wAALCAABAAEBAREA/8QAFAABAAAAAAAA"
    "AAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q=="
)

entries = []  # (domain, relative_path, content_bytes)


def add(domain, rel, content):
    entries.append((domain, rel, content))


def store(schema, rows=()):
    return sqlite3_bytes(schema, rows)


def sqlite3_bytes(schema, rows=()):
    """Build a SQLite database in a temp file and return its bytes."""
    import tempfile

    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
        path = tmp.name
    try:
        conn = sqlite3.connect(path)
        conn.executescript(schema)
        for row in rows:
            conn.execute(row)
        conn.commit()
        conn.close()
        with open(path, "rb") as f:
            return f.read()
    finally:
        os.unlink(path)


# --- Contacts (AddressBook.sqlitedb) ----------------------------------------
contacts = store(
    """
    CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT, Note TEXT);
    CREATE TABLE ABMultiValueLabel (ROWID INTEGER PRIMARY KEY, value TEXT);
    CREATE TABLE ABMultiValue (ROWID INTEGER PRIMARY KEY, UID INTEGER, record_id INTEGER, property INTEGER, label INTEGER, value TEXT);
    CREATE TABLE ABMultiValueEntryKey (ROWID INTEGER PRIMARY KEY, value TEXT);
    CREATE TABLE ABMultiValueEntry (ROWID INTEGER PRIMARY KEY, parent_id INTEGER, key INTEGER, value TEXT);
    INSERT INTO ABMultiValueLabel VALUES (1, '_$!<Mobile>!$_'), (2, '_$!<Home>!$_'), (3, '_$!<Work>!$_');
    INSERT INTO ABMultiValueEntryKey VALUES (1,'Street'),(2,'State'),(3,'ZIP'),(4,'City'),(5,'CountryCode'),(8,'Country');
    """,
    [
        "INSERT INTO ABPerson VALUES (1, 'Jan', 'Novák', 'Acme s.r.o.', 'kamarád z kocourkova')",
        "INSERT INTO ABPerson VALUES (2, 'Petra', 'Dvořáková', NULL, NULL)",
        "INSERT INTO ABPerson VALUES (3, NULL, NULL, 'Pizzeria U Bazénku', NULL)",
        "INSERT INTO ABMultiValue VALUES (1, 101, 1, 3, 1, '+420776452878')",
        "INSERT INTO ABMultiValue VALUES (2, 102, 1, 4, 2, 'jan@acme.cz')",
        "INSERT INTO ABMultiValue VALUES (3, 103, 2, 3, 1, '+420606123456')",
        "INSERT INTO ABMultiValue VALUES (10, 3, 1, 5, 3, NULL)",
        "INSERT INTO ABMultiValueEntry VALUES (1, 3, 1, 'Hlavní 1')",
        "INSERT INTO ABMultiValueEntry VALUES (2, 3, 4, 'Praha')",
        "INSERT INTO ABMultiValueEntry VALUES (3, 3, 3, '11000')",
        "INSERT INTO ABMultiValueEntry VALUES (4, 3, 8, 'Czechia')",
    ],
)
add("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb", contacts)

# --- Call history -----------------------------------------------------------
calls = store(
    """
    CREATE TABLE ZCALLRECORD (
        Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL, ZADDRESS BLOB,
        ZORIGINATED INTEGER, ZANSWERED INTEGER, ZCALLTYPE INTEGER,
        ZSERVICE_PROVIDER TEXT, ZLOCATION TEXT, ZISO_COUNTRY_CODE TEXT);
    """,
    [
        "INSERT INTO ZCALLRECORD VALUES (1, 715000000.0, 42.0, CAST('+420776452878' AS BLOB), 1, 1, 1, 'com.apple.Telephony', NULL, 'cz')",
        "INSERT INTO ZCALLRECORD VALUES (2, 715000100.0, 0.0, CAST('+420606123456' AS BLOB), 0, 0, 1, 'com.apple.Telephony', 'Praha', 'cz')",
        "INSERT INTO ZCALLRECORD VALUES (3, 715000200.0, 127.0, CAST('jana@example.cz' AS BLOB), 1, 1, 8, 'com.apple.FaceTime', NULL, NULL)",
    ],
)
add("HomeDomain", "Library/CallHistoryDB/CallHistory.storedata", calls)

# --- Calendar ---------------------------------------------------------------
calendar = store(
    """
    CREATE TABLE Calendar (ROWID INTEGER PRIMARY KEY, title TEXT);
    CREATE TABLE CalendarItem (ROWID INTEGER PRIMARY KEY, summary TEXT, start_date REAL,
        end_date REAL, all_day INTEGER, calendar_id INTEGER);
    INSERT INTO Calendar VALUES (1, 'Práce'), (2, 'Domov');
    """,
    [
        "INSERT INTO CalendarItem VALUES (1, 'Standa — schůzka', 715002000.0, 715003100.0, 0, 1)",
        "INSERT INTO CalendarItem VALUES (2, 'Výlet Krkonoše', 715100000.0, 715186400.0, 1, 2)",
        "INSERT INTO CalendarItem VALUES (3, 'Stomatolog', 715200000.0, 715203600.0, 0, 1)",
    ],
)
add("HomeDomain", "Library/Calendar/Calendar.sqlitedb", calendar)

# --- Safari -----------------------------------------------------------------
safari = store(
    """
    CREATE TABLE history_items (id INTEGER PRIMARY KEY, url TEXT, visit_count INTEGER);
    CREATE TABLE history_visits (id INTEGER PRIMARY KEY, history_item INTEGER, visit_time REAL, title TEXT);
    INSERT INTO history_items VALUES (1, 'https://www.apple.com', 5), (2, 'https://www.idnes.cz', 2), (3, 'https://github.com', 9);
    """,
    [
        "INSERT INTO history_visits VALUES (1, 1, 715005000.0, 'Apple')",
        "INSERT INTO history_visits VALUES (2, 2, 715006000.0, 'Idnes — zprávy')",
        "INSERT INTO history_visits VALUES (3, 3, 715007000.0, 'GitHub')",
    ],
)
add("AppDomain-com.apple.mobilesafari", "Library/Safari/History.db", safari)

bookmarks = store(
    """
    CREATE TABLE bookmarks (id INTEGER PRIMARY KEY, title TEXT, url TEXT, parent INTEGER, type INTEGER);
    INSERT INTO bookmarks VALUES
        (1, 'Favorites', NULL, NULL, 1),
        (2, 'Apple', 'https://www.apple.com', 1, 0),
        (3, 'News', NULL, 1, 1),
        (4, 'Idnes', 'https://www.idnes.cz', 3, 0);
    """,
)
add("AppDomain-com.apple.mobilesafari", "Library/Safari/Bookmarks.db", bookmarks)

# --- Notes (NULL body → snippet path) ---------------------------------------
notes = store(
    """
    CREATE TABLE ZICCLOUDSYNCINGOBJECT (Z_PK INTEGER PRIMARY KEY, ZTITLE1 TEXT, ZTITLE2 TEXT,
        ZSNIPPET TEXT, ZNOTEDATA INTEGER, ZFOLDER INTEGER, ZCREATIONDATE REAL, ZMODIFICATIONDATE1 REAL);
    CREATE TABLE ZICNOTEDATA (Z_PK INTEGER PRIMARY KEY, ZNOTE INTEGER, ZDATA BLOB);
    INSERT INTO ZICCLOUDSYNCINGOBJECT (Z_PK, ZTITLE2, ZSNIPPET, ZNOTEDATA, ZFOLDER, ZCREATIONDATE, ZMODIFICATIONDATE1) VALUES
        (100, 'Nákup', 'mléko, chleba, rohlíky', 1, 10, 715001000.0, 715001500.0),
        (101, 'Dovolená', 'letenky + hotel potvrzeno', 2, 10, 715002000.0, 715002100.0);
    INSERT INTO ZICNOTEDATA VALUES (1, 100, NULL), (2, 101, NULL);
    """,
)
add("AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite", notes)

# --- Photos (Photos.sqlite + DCIM files) ------------------------------------
photos = store(
    """
    CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZFILENAME TEXT, ZDIRECTORY TEXT,
        ZDATECREATED REAL, ZMODIFICATIONDATE REAL, ZADDEDDATE REAL, ZKIND INTEGER,
        ZKINDSUBTYPE INTEGER, ZFAVORITE INTEGER, ZHIDDEN INTEGER, ZTRASHEDSTATE INTEGER,
        ZHASADJUSTMENTS INTEGER, ZWIDTH INTEGER, ZHEIGHT INTEGER, ZLATITUDE REAL,
        ZLONGITUDE REAL, ZDURATION REAL, ZAVALANCHEUUID TEXT, ZADDITIONALATTRIBUTES INTEGER,
        ZTRASHEDDATE REAL);
    CREATE TABLE ZGENERICALBUM (Z_PK INTEGER PRIMARY KEY, ZTITLE TEXT, ZKIND INTEGER);
    CREATE TABLE Z_28ASSETS (Z_28ALBUMS INTEGER, Z_3ASSETS INTEGER);
    INSERT INTO ZGENERICALBUM VALUES (1, 'Dovolená', 2), (2, 'Rodina', 2);
    INSERT INTO Z_28ASSETS VALUES (1, 1), (1, 2), (2, 3);
    """,
    [
        "INSERT INTO ZASSET VALUES (1, 'IMG_0001.JPG', 'DCIM/100APPLE', 715050000.0, 715050050.0, 715050010.0, 0, 2, 1, 0, 0, 0, 4032, 3024, 50.087, 14.42, NULL, NULL, NULL, NULL)",
        "INSERT INTO ZASSET VALUES (2, 'IMG_0002.JPG', 'DCIM/100APPLE', 715060000.0, NULL, NULL, 0, 0, 0, 0, 0, 0, 4032, 3024, NULL, NULL, NULL, NULL, NULL, NULL)",
        "INSERT INTO ZASSET VALUES (3, 'IMG_0003.MOV', 'DCIM/101APPLE', 715070000.0, NULL, NULL, 1, 0, 0, 0, 0, 0, 1920, 1080, NULL, NULL, 12.5, 'BURST1', NULL, NULL)",
    ],
)
add(DOMAIN, "Media/PhotoData/Photos.sqlite", photos)
add(DOMAIN, "Media/DCIM/100APPLE/IMG_0001.JPG", JPEG)
add(DOMAIN, "Media/DCIM/100APPLE/IMG_0002.JPG", JPEG)
add(DOMAIN, "Media/DCIM/101APPLE/IMG_0003.MOV", JPEG)
# Manifest-only gap: on disk + in Manifest.db, but NO row in Photos.sqlite —
# demos the `uncatalogued` reconciliation (🧩 z Manifest.db badge).
add(DOMAIN, "Media/DCIM/117APPLE/IMG_0999.MOV", JPEG)

# --- Accounts ----------------------------------------------------------------
accounts = store(
    """
    CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY, ZACCOUNTTYPEDESCRIPTION TEXT, ZIDENTIFIER TEXT);
    CREATE TABLE ZACCOUNT (Z_PK INTEGER PRIMARY KEY, ZACCOUNTTYPE INTEGER, ZACTIVE INTEGER,
        ZDATE REAL, ZIDENTIFIER TEXT, ZACCOUNTDESCRIPTION TEXT, ZUSERNAME TEXT, ZOWNINGBUNDLEID TEXT);
    INSERT INTO ZACCOUNTTYPE VALUES (1, 'iCloud', 'com.apple.account.iCloud'), (2, 'Gmail', 'com.google.Gmail');
    """,
    [
        "INSERT INTO ZACCOUNT VALUES (1, 1, 1, 715000000.0, 'uuid-1', 'iCloud', 'karel@icloud.com', NULL)",
        "INSERT INTO ZACCOUNT VALUES (2, 2, 1, 715000100.0, 'uuid-2', 'Gmail', 'karel@gmail.com', 'com.google.Gmail')",
    ],
)
add("HomeDomain", "Library/Accounts/Accounts3.sqlite", accounts)

# --- Reminders ---------------------------------------------------------------
reminders = store(
    """
    CREATE TABLE Z_PRIMARYKEY (Z_ENT INTEGER PRIMARY KEY, Z_NAME TEXT, Z_SUPER INTEGER, Z_MAX INTEGER);
    CREATE TABLE ZREMCDOBJECT (
        Z_PK INTEGER PRIMARY KEY, Z_ENT INTEGER,
        ZTITLE1 TEXT, ZNOTES TEXT, ZDUEDATE REAL, ZCOMPLETED INTEGER,
        ZCOMPLETIONDATE REAL, ZPRIORITY INTEGER, ZCREATIONDATE REAL,
        ZFLAGGED INTEGER, ZLIST INTEGER, ZNAME2 TEXT);
    INSERT INTO Z_PRIMARYKEY VALUES (7, 'REMCDReminder', NULL, 10), (25, 'REMCDList', NULL, 2);
    INSERT INTO ZREMCDOBJECT (Z_PK, Z_ENT, ZNAME2) VALUES (1, 25, 'Nákup'), (2, 25, 'Práce');
    """,
    [
        "INSERT INTO ZREMCDOBJECT (Z_PK, Z_ENT, ZTITLE1, ZNOTES, ZDUEDATE, ZCOMPLETED, ZCOMPLETIONDATE, ZPRIORITY, ZCREATIONDATE, ZFLAGGED, ZLIST) VALUES (10, 7, 'Koupit mléko', '2 litry', 715002000.0, 0, NULL, 1, 715001000.0, 0, 1)",
        "INSERT INTO ZREMCDOBJECT (Z_PK, Z_ENT, ZTITLE1, ZNOTES, ZDUEDATE, ZCOMPLETED, ZCOMPLETIONDATE, ZPRIORITY, ZCREATIONDATE, ZFLAGGED, ZLIST) VALUES (11, 7, 'Odpovědět na mail', NULL, NULL, 1, 715001500.0, 0, 715001100.0, 0, 2)",
    ],
)
add("AppDomainGroup-group.com.apple.reminders", "Stores/Data-Demo.sqlite", reminders)

# --- WhatsApp (schema only; media skipped in demo) ----------------------------
wa = store(
    """
    CREATE TABLE ZWACHATSESSION (Z_PK INTEGER PRIMARY KEY, ZPARTNERNAME TEXT, ZCONTACTJID TEXT);
    CREATE TABLE ZWAMEDIAITEM (Z_PK INTEGER PRIMARY KEY, ZMESSAGE INTEGER, ZMEDIALOCALPATH TEXT);
    CREATE TABLE ZWAMESSAGE (Z_PK INTEGER PRIMARY KEY, ZTEXT TEXT, ZMESSAGEDATE REAL,
        ZISFROMME INTEGER, ZFROMJID TEXT, ZCHATSESSION INTEGER, ZMEDIAITEM INTEGER);
    INSERT INTO ZWACHATSESSION VALUES (1, 'Jana Nováková', '420606123456@s.whatsapp.net');
    INSERT INTO ZWACHATSESSION VALUES (2, 'Skupina chata', 'demo-group@g.us');
    """,
    [
        "INSERT INTO ZWAMESSAGE VALUES (1, 'Ahoj, jak se máš?', 715020000.0, 1, NULL, 1, NULL)",
        "INSERT INTO ZWAMESSAGE VALUES (2, NULL, 715020100.0, 0, '420606123456@s.whatsapp.net', 1, NULL)",
        "INSERT INTO ZWAMESSAGE VALUES (3, 'Zítra v 8? 👍', 715020200.0, 0, 'demo-group@g.us', 2, NULL)",
        "INSERT INTO ZWAMESSAGE VALUES (4, 'Jo, potvzeno', 715020300.0, 1, NULL, 2, NULL)",
    ],
)
add("AppDomainGroup-group.net.whatsapp.WhatsApp.shared", "ChatStorage.sqlite", wa)

# --- Voice memos -------------------------------------------------------------
memos = store(
    """
    CREATE TABLE ZCLOUDRECORDING (
        Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL,
        ZCUSTOMLABEL TEXT, ZENCRYPTEDTITLE TEXT, ZPATH TEXT);
    INSERT INTO ZCLOUDRECORDING VALUES (1, 715030000.0, 12.5, 'Schůzka', NULL, '20200101 120000.m4a'),
                                       (2, 715030100.0, 3.0, NULL, NULL, 'A1B2C3.m4a');
    """,
)
add("AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db", memos)


def file_id(domain, rel):
    return hashlib.sha1(f"{domain}-{rel}".encode()).hexdigest()


def mbfile_blob(size, mode, mtime):
    """NSKeyedArchiver binary plist of an MBFile entry (what crabapple parses
    from the Manifest.db `file` column): LastModified/Flags/GroupID/Size/Mode/
    InodeNumber/ProtectionClass in the root object."""
    top = {
        "LastModified": mtime,
        "Flags": 1,
        "GroupID": 501,
        "LastStatusChange": mtime,
        "Birth": mtime,
        "Size": size,
        "Mode": mode,
        "UserID": 501,
        "InodeNumber": 1,
        "ProtectionClass": 1,
        "$class": plistlib.UID(1),
    }
    return plistlib.dumps(
        {
            "$version": 100000,
            "$objects": ["$null", top, {"$classname": "MBFile", "$classes": ["MBFile", "NSObject"]}],
            "$top": {"root": plistlib.UID(1)},
            "$archiver": "NSKeyedArchiver",
        },
        fmt=plistlib.FMT_BINARY,
    )


def main():
    if os.path.exists(OUT):
        print(f"demo backup already exists: {OUT}")
        return
    os.makedirs(OUT)

    # Store files + manifest rows.
    rows = []
    for domain, rel, content in entries:
        fid = file_id(domain, rel)
        dest = os.path.join(OUT, fid[:2], fid)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "wb") as f:
            f.write(content)
        blob = mbfile_blob(len(content), 0o100644, 715050000)
        rows.append((fid, domain, rel, 1, blob))
        # Parent directory entries (flags=2), like a real backup.
        parts = rel.split("/")[:-1]
        for i in range(len(parts)):
            drel = "/".join(parts[: i + 1])
            did = file_id(domain, drel)
            if not any(r[1] == domain and r[2] == drel for r in rows):
                dblob = mbfile_blob(0, 0o040755, 715050000)
                rows.append((did, domain, drel, 2, dblob))

    # Manifest.db
    manifest = os.path.join(OUT, "Manifest.db")
    conn = sqlite3.connect(manifest)
    conn.executescript(
        "CREATE TABLE Files (fileID TEXT PRIMARY KEY, domain TEXT, relativePath TEXT, "
        "flags INTEGER, file BLOB);"
    )
    conn.executemany("INSERT INTO Files VALUES (?, ?, ?, ?, ?)", rows)
    conn.commit()
    conn.close()

    # Info.plist (lockdown info crabapple reads for the device sheet).
    info = {
        "Lockdown": {
            "Device Name": "Karlův iPhone",
            "Product Type": "iPhone14,2",
            "Product Version": "17.5.1",
            "Serial Number": "F2LDEMO99XZ",
            "Phone Number": "+420 776 452 878",
            "Build Version": "21E90",
            "Target Type": "Disk",
        },
        "Last Backup Date": __import__("datetime").datetime(2026, 9, 17, 12, 0, 0),
        "iTunes Version": "12.13.0.9",
        "Target Type": "Disk",
        "GUID": "DEMO-DEMO-DEMO",
        "Sync Settings ID": "DEMO-DEMO-DEMO",
        "iOS Application": True,
    }
    with open(os.path.join(OUT, "Info.plist"), "wb") as f:
        plistlib.dump(info, f, fmt=plistlib.FMT_BINARY)

    # Status.plist
    status = {
        "BackupState": "new",
        "SnapshotState": "finished",
        "UUID": "DEMO-UUID-0001",
        "IsFullBackup": True,
        "Date": __import__("datetime").datetime(2026, 9, 17, 12, 0, 0),
    }
    with open(os.path.join(OUT, "Status.plist"), "wb") as f:
        plistlib.dump(status, f, fmt=plistlib.FMT_BINARY)

    # Manifest.plist (unencrypted) — Lockdown here is what crabapple reads
    # for the device sheet (DeviceName/ProductVersion/Serial).
    lockdown = {
        "UniqueDeviceID": "00008101-DEMO0001A3C42E01",
        "DeviceName": "Karlův iPhone",
        "Device Name": "Karlův iPhone",
        "ProductType": "iPhone14,2",
        "Product Type": "iPhone14,2",
        "ProductVersion": "17.5.1",
        "Product Version": "17.5.1",
        "SerialNumber": "F2LDEMO99XZ",
        "Serial Number": "F2LDEMO99XZ",
        "PhoneNumber": "+420 776 452 878",
        "Phone Number": "+420 776 452 878",
        "BuildVersion": "21E90",
        "Build Version": "21E90",
        "Target Type": "Disk",
        "TargetType": "Disk",
    }
    manifest_plist = {
        "IsEncrypted": False,
        "Date": __import__("datetime").datetime(2026, 9, 17, 12, 0, 0),
        "Version": "3.3",
        "WasPasscodeSet": False,
        "Lockdown": lockdown,
        "BackupKeyBag": b"",  # unencrypted backups carry no keybag
        "Applications": {},   # no third-party apps in the demo
        "Applications2": {},
    }
    with open(os.path.join(OUT, "Manifest.plist"), "wb") as f:
        plistlib.dump(manifest_plist, f, fmt=plistlib.FMT_BINARY)

    n_files = len(entries)
    print(f"demo backup created: {OUT}")
    print(f"store files: {n_files}, manifest rows: {len(rows)}")


if __name__ == "__main__":
    main()

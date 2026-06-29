//! Read configured accounts from an iOS `Accounts3.sqlite` (Core Data store).
//!
//! Lists every account set up on the device — Apple ID/iCloud, Google, Exchange,
//! IMAP, CalDAV/CardDAV, social, VPN, … — with its type, user-visible
//! description, username and added date. Passwords are **not** here (credentials
//! live in the keychain), so this is non-sensitive account *metadata*: a
//! migration checklist of which accounts must be re-added on a new device.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One configured account.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Account {
    /// Human account-type label, e.g. "iCloud", "Exchange", "Google" (from
    /// `ZACCOUNTTYPE`); falls back to the type identifier, then "unknown".
    pub account_type: String,
    /// Reverse-DNS account-type identifier, e.g. "com.apple.account.iCloud".
    pub type_identifier: String,
    /// User-visible account description/label, e.g. "iCloud" or "Work".
    pub description: String,
    /// Account username (often an email address); empty when not set.
    pub username: String,
    /// Owning app bundle id, when the account belongs to a third-party app.
    pub bundle_id: String,
    /// Date the account was added, ISO 8601 UTC; empty if unknown/unconvertible.
    pub date: String,
    /// Whether the account is active/enabled; `None` when not recorded.
    pub active: Option<bool>,
}

/// Parse every account from `db_path` (opened read-only), tolerating columns that
/// come and go across iOS versions.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Account>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let acc = table_columns(&conn, "ZACCOUNT")?;
    let typ = table_columns(&conn, "ZACCOUNTTYPE")?;

    // Select a column only if present, else NULL — `ZACCOUNT`/`ZACCOUNTTYPE` and
    // the `ZACCOUNT.ZACCOUNTTYPE -> ZACCOUNTTYPE.Z_PK` FK are stable; individual
    // attribute columns are not.
    let a = |name: &str| -> String {
        if acc.contains(name) { format!("a.{name}") } else { "NULL".into() }
    };
    let t = |name: &str| -> String {
        if typ.contains(name) { format!("t.{name}") } else { "NULL".into() }
    };

    let order = if typ.contains("ZACCOUNTTYPEDESCRIPTION") {
        "t.ZACCOUNTTYPEDESCRIPTION"
    } else {
        "a.Z_PK"
    };
    let sql = format!(
        "SELECT {username}, {desc}, {bundle}, {date}, {active}, {tdesc}, {tident} \
         FROM ZACCOUNT a LEFT JOIN ZACCOUNTTYPE t ON a.ZACCOUNTTYPE = t.Z_PK \
         ORDER BY {order}, {username}",
        username = a("ZUSERNAME"),
        desc = a("ZACCOUNTDESCRIPTION"),
        bundle = a("ZOWNINGBUNDLEID"),
        date = a("ZDATE"),
        active = a("ZACTIVE"),
        tdesc = t("ZACCOUNTTYPEDESCRIPTION"),
        tident = t("ZIDENTIFIER"),
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let username: Option<String> = row.get(0)?;
        let description: Option<String> = row.get(1)?;
        let bundle: Option<String> = row.get(2)?;
        let date: Option<f64> = row.get(3)?;
        let active: Option<i64> = row.get(4)?;
        let tdesc: Option<String> = row.get(5)?;
        let tident: Option<String> = row.get(6)?;
        let account_type = tdesc
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| tident.clone().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "unknown".to_string());
        Ok(Account {
            account_type,
            type_identifier: tident.unwrap_or_default(),
            description: description.unwrap_or_default(),
            username: username.unwrap_or_default(),
            bundle_id: bundle.unwrap_or_default(),
            date: date.and_then(cocoa_to_iso).unwrap_or_default(),
            active: active.map(|v| v != 0),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_accounts;

    fn fixture(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("be-accounts-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("Accounts3.sqlite")
    }

    #[test]
    fn parses_accounts_with_type_join() {
        let db = fixture("join");
        let _ = std::fs::remove_file(&db);
        make_accounts(&db);

        let accounts = parse(&db).unwrap();
        assert_eq!(accounts.len(), 3);

        let icloud = accounts.iter().find(|a| a.username == "jane@icloud.com").unwrap();
        assert_eq!(icloud.account_type, "iCloud");
        assert_eq!(icloud.type_identifier, "com.apple.account.iCloud");
        assert_eq!(icloud.active, Some(true));
        assert!(icloud.date.starts_with("2020-"), "date was {}", icloud.date);

        let gmail = accounts.iter().find(|a| a.username == "jane@gmail.com").unwrap();
        assert_eq!(gmail.account_type, "Google");
        assert_eq!(gmail.bundle_id, "com.google.Gmail");

        // Row with no username/description still resolves its type via the join,
        // and an inactive account with no date yields active=false / empty date.
        let bare = accounts.iter().find(|a| a.username.is_empty()).unwrap();
        assert_eq!(bare.account_type, "iCloud");
        assert_eq!(bare.description, "");
        assert_eq!(bare.active, Some(false));
        assert_eq!(bare.date, "");

        std::fs::remove_dir_all(db.parent().unwrap()).ok();
    }

    #[test]
    fn tolerates_missing_optional_columns() {
        // A minimal store missing ZACTIVE/ZDATE/ZOWNINGBUNDLEID and the whole
        // ZACCOUNTTYPE table must still parse (type falls back to "unknown").
        let db = fixture("min");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZACCOUNT (Z_PK INTEGER PRIMARY KEY, ZACCOUNTTYPE INTEGER, ZUSERNAME TEXT, ZACCOUNTDESCRIPTION TEXT);
             CREATE TABLE ZACCOUNTTYPE (Z_PK INTEGER PRIMARY KEY);
             INSERT INTO ZACCOUNT (Z_PK, ZACCOUNTTYPE, ZUSERNAME, ZACCOUNTDESCRIPTION) VALUES (1, 1, 'bob', 'Work');",
        )
        .unwrap();
        drop(conn);

        let accounts = parse(&db).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].username, "bob");
        assert_eq!(accounts[0].account_type, "unknown");
        assert_eq!(accounts[0].active, None);
        assert_eq!(accounts[0].date, "");

        std::fs::remove_dir_all(db.parent().unwrap()).ok();
    }
}

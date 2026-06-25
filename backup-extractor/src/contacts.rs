//! Read contacts from an iOS `AddressBook.sqlitedb`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// A labelled value such as a phone number or email address.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}

/// One address-book entry. Addresses are deferred to a later increment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contact {
    pub first: String,
    pub last: String,
    pub organization: String,
    pub phones: Vec<Labeled>,
    pub emails: Vec<Labeled>,
    pub note: String,
}

// AddressBook `property` codes.
const PROP_PHONE: i64 = 3;
const PROP_EMAIL: i64 = 4;

/// Strip Apple's label wrapper, e.g. `_$!<Mobile>!$_` -> `Mobile`. Other labels
/// (or `NULL`) are returned trimmed/empty unchanged.
fn clean_label(raw: Option<String>) -> String {
    let raw = raw.unwrap_or_default();
    raw.strip_prefix("_$!<")
        .and_then(|s| s.strip_suffix(">!$_"))
        .unwrap_or(&raw)
        .to_string()
}

/// Parse every contact from `db_path` (opened read-only).
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Contact>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let mut people_stmt = conn.prepare(
        "SELECT ROWID, First, Last, Organization, Note FROM ABPerson",
    )?;
    let rows = people_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        ))
    })?;

    let mut mv_stmt = conn.prepare(
        "SELECT mv.property, mv.value, l.value
         FROM ABMultiValue mv
         LEFT JOIN ABMultiValueLabel l ON l.ROWID = mv.label
         WHERE mv.record_id = ?1",
    )?;

    let mut contacts = Vec::new();
    for person in rows {
        let (rowid, first, last, organization, note) = person?;
        let mut phones = Vec::new();
        let mut emails = Vec::new();
        let values = mv_stmt.query_map([rowid], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for value in values {
            let (property, val, label) = value?;
            let entry = Labeled { label: clean_label(label), value: val };
            match property {
                PROP_PHONE => phones.push(entry),
                PROP_EMAIL => emails.push(entry),
                _ => {}
            }
        }
        contacts.push(Contact { first, last, organization, phones, emails, note });
    }
    Ok(contacts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_addressbook;

    #[test]
    fn parses_people_phones_and_emails() {
        let dir = std::env::temp_dir().join(format!("be-ab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("AddressBook.sqlitedb");
        let _ = std::fs::remove_file(&db);
        make_addressbook(&db);

        let people = parse(&db).unwrap();

        assert_eq!(people.len(), 2);

        let jan = people.iter().find(|c| c.first == "Jan").unwrap();
        assert_eq!(jan.last, "Novák");
        assert_eq!(jan.organization, "Acme");
        assert_eq!(jan.note, "kamarád");
        assert_eq!(jan.phones, vec![Labeled { label: "Mobile".into(), value: "+420776452878".into() }]);
        assert_eq!(jan.emails, vec![Labeled { label: "Home".into(), value: "jan@example.cz".into() }]);

        let company = people.iter().find(|c| c.organization == "Firma s.r.o.").unwrap();
        assert_eq!(company.first, "");
        assert!(company.phones.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}

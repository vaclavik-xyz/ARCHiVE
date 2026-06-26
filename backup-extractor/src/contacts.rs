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

/// One postal address slot (Home/Work) for a contact.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Address {
    pub label: String,
    pub street: String,
    pub city: String,
    pub state: String,
    pub zip: String,
    pub country: String,
    pub country_code: String,
}

/// One address-book entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contact {
    pub first: String,
    pub last: String,
    pub organization: String,
    pub phones: Vec<Labeled>,
    pub emails: Vec<Labeled>,
    pub note: String,
    pub addresses: Vec<Address>,
}

// AddressBook `property` codes.
const PROP_PHONE: i64 = 3;
const PROP_EMAIL: i64 = 4;
const PROP_ADDRESS: i64 = 5;

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

    let mut people_stmt = conn.prepare("SELECT ROWID, First, Last, Organization, Note FROM ABPerson")?;
    let people = people_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // mv: one row per phone/email/address slot; UID links an address slot to its parts.
    let mut mv_stmt = conn.prepare(
        "SELECT mv.UID, mv.property, mv.value, l.value
         FROM ABMultiValue mv
         LEFT JOIN ABMultiValueLabel l ON l.ROWID = mv.label
         WHERE mv.record_id = ?1",
    )?;
    // entry: the parts of one address slot, keyed by component name (resolved by name, not id).
    let mut entry_stmt = conn.prepare(
        "SELECT k.value, e.value
         FROM ABMultiValueEntry e
         JOIN ABMultiValueEntryKey k ON k.ROWID = e.key
         WHERE e.parent_id = ?1",
    )?;

    let mut contacts = Vec::new();
    for (rowid, first, last, organization, note) in people {
        let mut phones = Vec::new();
        let mut emails = Vec::new();
        let mut addresses = Vec::new();

        let mv_rows = mv_stmt
            .query_map([rowid], |row| {
                Ok((
                    row.get::<_, i64>(0)?,                                 // UID
                    row.get::<_, i64>(1)?,                                 // property
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(), // value
                    row.get::<_, Option<String>>(3)?,                     // label
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for (uid, property, value, label) in mv_rows {
            match property {
                PROP_PHONE => phones.push(Labeled { label: clean_label(label), value }),
                PROP_EMAIL => emails.push(Labeled { label: clean_label(label), value }),
                PROP_ADDRESS => {
                    let mut addr = Address { label: clean_label(label), ..Address::default() };
                    let parts = entry_stmt.query_map([uid], |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        ))
                    })?;
                    for part in parts {
                        let (key, val) = part?;
                        match key.to_lowercase().as_str() {
                            "street" => addr.street = val,
                            "city" => addr.city = val,
                            "state" => addr.state = val,
                            "zip" => addr.zip = val,
                            "country" => addr.country = val,
                            "countrycode" => addr.country_code = val,
                            _ => {}
                        }
                    }
                    addresses.push(addr);
                }
                _ => {}
            }
        }

        contacts.push(Contact { first, last, organization, phones, emails, note, addresses });
    }
    Ok(contacts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_addressbook;

    #[test]
    fn parses_addresses_joined_on_uid() {
        let dir = std::env::temp_dir().join(format!("be-ab-addr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("AddressBook.sqlitedb");
        let _ = std::fs::remove_file(&db);
        make_addressbook(&db);

        let people = parse(&db).unwrap();
        let jan = people.iter().find(|c| c.first == "Jan").unwrap();
        assert_eq!(jan.addresses.len(), 1);
        let a = &jan.addresses[0];
        assert_eq!(a.label, "Work");
        assert_eq!(a.street, "Hlavní 1");
        assert_eq!(a.city, "Praha");
        assert_eq!(a.zip, "11000");
        assert_eq!(a.country, "Czechia");
        assert_eq!(a.state, "");
        assert_eq!(a.country_code, "");

        let company = people.iter().find(|c| c.organization == "Firma s.r.o.").unwrap();
        assert!(company.addresses.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

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

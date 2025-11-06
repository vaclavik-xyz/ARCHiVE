use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use imessage_database::{
    error::table::TableError, tables::table::get_connection, util::dirs::home,
};
use rusqlite::{Connection, Result};

pub const DEFAULT_PATH_IOS: &str = "31/31bb7ba8914766d4ba40d6dfb6113c8b614be442";

// MARK: Name
#[derive(Clone, Debug, PartialEq, Eq)]
/// Simple first/last name struct
pub struct Name {
    /// First name
    pub first: String,
    /// Last name
    pub last: String,
}

impl Name {
    /// Create from optional first/last name
    fn from_opt(first: Option<String>, last: Option<String>) -> Option<Self> {
        // Return None if both are None
        if first.is_none() && last.is_none() {
            return None;
        }

        Some(Name {
            first: first.unwrap_or_default(),
            last: last.unwrap_or_default(),
        })
    }

    /// Simple scoring: 1 point for first name, 1 point for last name
    fn score(&self) -> u8 {
        u8::from(!self.first.is_empty()) + u8::from(!self.last.is_empty())
    }
}

// MARK: Index
#[derive(Debug, Default)]
/// Contacts index for looking up names by phone/email
pub struct ContactsIndex {
    by_phone: HashMap<String, Name>,
    by_email: HashMap<String, Name>,
}

impl ContactsIndex {
    /// Build a contacts index
    ///
    /// - If `path` is `Some`, we only look at that database.
    /// - If `path` is `None`, scans macOS Contacts sources under
    ///   `~/Library/Application Support/AddressBook/Sources/*/AddressBook-v22.abcddb`
    ///
    /// Supports building from both macOS (`AddressBook-v22.abcddb`) and iOS (`AddressBook.sqlitedb`) databases.
    pub fn build(path: Option<&Path>) -> Result<Self, TableError> {
        if let Some(path) = path {
            let conn = get_connection(path)?;
            if table_exists(&conn, "ABPersonFullTextSearch_content") {
                return Ok(Self::build_from_ios(&conn)?);
            }
            return Ok(Self::build_from_macos(&conn)?);
        }

        let mut combined_by_phone: HashMap<String, Name> = HashMap::new();
        let mut combined_by_email: HashMap<String, Name> = HashMap::new();

        for db_path in find_macos_addressbook_db_paths() {
            if let Ok(local_conn) = Connection::open(&db_path) {
                let sub = Self::build_from_macos(&local_conn)?;

                for (k, v) in sub.by_phone {
                    upsert_best(&mut combined_by_phone, k, &v);
                }
                for (k, v) in sub.by_email {
                    upsert_best(&mut combined_by_email, k, &v);
                }
            }
        }

        Ok(Self {
            by_phone: combined_by_phone,
            by_email: combined_by_email,
        })
    }

    /// Build contacts index from macOS Contacts database
    fn build_from_macos(conn: &Connection) -> Result<Self> {
        let mut by_phone = HashMap::new();
        let mut by_email = HashMap::new();

        let mut stmt = conn.prepare(
            "SELECT r.ZFIRSTNAME, r.ZLASTNAME, p.ZFULLNUMBER, e.ZADDRESSNORMALIZED
             FROM ZABCDRECORD AS r
             LEFT JOIN ZABCDPHONENUMBER AS p ON r.Z_PK = p.ZOWNER
             LEFT JOIN ZABCDEMAILADDRESS AS e ON r.Z_PK = e.ZOWNER",
        )?;

        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name = Name::from_opt(
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            );

            if let Some(name) = name {
                if let Some(email_raw) = row.get::<_, Option<String>>(3)? {
                    // Some macOS rows are like "<addr@dom>"
                    for email in parse_email_list(&email_raw) {
                        upsert_best(&mut by_email, email, &name);
                    }
                }

                if let Some(phone_raw) = row.get::<_, Option<String>>(2)? {
                    for key in phone_keys(&phone_raw) {
                        upsert_best(&mut by_phone, key, &name);
                    }
                }
            }
        }

        Ok(Self { by_phone, by_email })
    }

    /// Build contacts index from iOS backup database
    fn build_from_ios(conn: &Connection) -> Result<Self> {
        // iOS backup contacts: ABPersonFullTextSearch_content with columns:
        // c0First (TEXT), c1Last (TEXT), c16Phone (TEXT: space-separated variants), c17Email (TEXT: space-separated)
        let mut by_phone = HashMap::new();
        let mut by_email = HashMap::new();

        let mut stmt = conn.prepare(
            "SELECT c0First, c1Last, c16Phone, c17Email
             FROM ABPersonFullTextSearch_content",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name = Name::from_opt(
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            );

            if let Some(name) = name {
                if let Some(phones_blob) = row.get::<_, Option<String>>(2)? {
                    for token in phones_blob.split_whitespace() {
                        for key in phone_keys(token) {
                            upsert_best(&mut by_phone, key, &name);
                        }
                    }
                }

                if let Some(emails_blob) = row.get::<_, Option<String>>(3)? {
                    for email in emails_blob.split_whitespace() {
                        if let Some(norm) = normalize_email(email) {
                            upsert_best(&mut by_email, norm, &name);
                        }
                    }
                }
            }
        }

        Ok(Self { by_phone, by_email })
    }

    /// Returns first/last name if found
    pub fn lookup(&self, id: &str) -> Option<Name> {
        if looks_like_email(id) {
            normalize_email(id).and_then(|k| self.by_email.get(&k).cloned())
        } else {
            for k in phone_keys(id) {
                if let Some(n) = self.by_phone.get(&k) {
                    return Some(n.clone());
                }
            }
            None
        }
    }

    /// Mutate the provided participants map to replace names with full names where possible
    ///
    /// The participants map is of the form: id => name and comes from `imessage_database::tables::handle::Handle::cache`
    pub fn mutate_participants(&self, participants: &mut HashMap<i32, String>) {
        for (_id, name) in participants.iter_mut() {
            // Contacts may have multiple parts due to message service duplication; try each until we find a match
            for part in name.split(' ') {
                // Try to look up this part
                if let Some(contact_name) = self.lookup(part) {
                    // Found a match; update full name
                    *name = format!("{} {}", contact_name.first, contact_name.last)
                        .trim()
                        .to_string();
                    break;
                }
            }
        }
    }
}

/// Check if a table or view exists in the database
fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1 LIMIT 1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

/// Upsert a Name into the map if it has a better score than existing
fn upsert_best(map: &mut HashMap<String, Name>, key: String, incoming: &Name) {
    match map.get_mut(&key) {
        Some(existing) => {
            if incoming.score() > existing.score() {
                *existing = incoming.clone();
            }
        }
        None => {
            map.insert(key, incoming.clone());
        }
    }
}

// MARK: Email
/// Simple heuristic to determine if the identifier looks like an email
fn looks_like_email(s: &str) -> bool {
    s.contains('@')
}

/// Normalize email: trim, lowercase, remove angle-brackets
fn normalize_email(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Guard for angle-brackets
    let s = s.trim_start_matches('<').trim_end_matches('>');
    if s.is_empty() {
        return None;
    }
    Some(s.to_lowercase())
}

/// Parse a space-separated list of emails
fn parse_email_list(raw: &str) -> Vec<String> {
    // macOS may store a single value; guard for angle-brackets
    if raw.contains(' ') {
        raw.split_whitespace().filter_map(normalize_email).collect()
    } else {
        normalize_email(raw).into_iter().collect()
    }
}

// MARK: Phone
/// Generate possible phone number keys from a raw phone number
fn phone_keys(raw: &str) -> Vec<String> {
    if raw.contains("urn:") {
        return vec![];
    }

    // The digits include the country code portion of the number
    let digits = to_phone_digits(raw);
    if digits.is_empty() {
        return vec![];
    }

    // Create keys with and without '+' prefix for country code
    let mut keys = vec![digits.clone(), format!("+{digits}")];

    // If the original was 12 chars starting with +1, add a variant without the `+1` (USA) country code
    if digits.len() == 11 && raw.starts_with("+1") {
        let last_10 = &digits[digits.len() - 10..];
        keys.push(last_10.to_string());
        keys.push(format!("+{last_10}"));
    }

    keys.dedup();
    keys
}

/// Extract digits from a raw phone number string
fn to_phone_digits(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            out.push(ch);
        }
    }
    out
}

// MARK: macOS
/// Find all `AddressBook` databases under the macOS Contacts Sources directory
fn find_macos_addressbook_db_paths() -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(macos_sources_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let db_path = path.join("AddressBook-v22.abcddb");
                if db_path.is_file() {
                    results.push(db_path);
                }
            }
        }
    }
    results
}

/// Resolve the standard macOS Contacts Sources directory: `~/Library/Application Support/AddressBook/Sources`
fn macos_sources_dir() -> PathBuf {
    let home = home();
    let p: PathBuf = Path::new(&home)
        .join("Library")
        .join("Application Support")
        .join("AddressBook")
        .join("Sources");
    p
}
// MARK: Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phone_lookup_us_with_country_code_with_plus() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "John".to_string(),
            last: "Doe".to_string(),
        };
        // Simulate building from db with "+12345678901" (US number with +1 and 10 digits)
        let db_phone = "+12345678901";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // Lookup with +1 should match
        assert_eq!(index.lookup("+12345678901"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_us_with_country_code_without_plus() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "John".to_string(),
            last: "Doe".to_string(),
        };
        // Simulate building from db with "+12345678901" (US number with +1 and 10 digits)
        let db_phone = "+12345678901";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // Lookup without + should match
        assert_eq!(index.lookup("12345678901"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_us_with_country_code_without_plus1() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "John".to_string(),
            last: "Doe".to_string(),
        };
        // Simulate building from db with "+12345678901" (US number with +1 and 10 digits)
        let db_phone = "+12345678901";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // Lookup without +1 should match (US variant)
        assert_eq!(index.lookup("2345678901"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_us_with_country_code_with_plus_without1() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "John".to_string(),
            last: "Doe".to_string(),
        };
        // Simulate building from db with "+12345678901" (US number with +1 and 10 digits)
        let db_phone = "+12345678901";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // Lookup with + but without 1 should match
        assert_eq!(index.lookup("+2345678901"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_us_without_country_code_with_plus1() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Jane".to_string(),
            last: "Smith".to_string(),
        };
        // Simulate building from db with "1234567890" (no +1)
        let db_phone = "1234567890";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // Lookup with +1 should match
        assert_eq!(index.lookup("+1234567890"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_us_without_country_code_without_plus() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Jane".to_string(),
            last: "Smith".to_string(),
        };
        // Simulate building from db with "1234567890" (no +1)
        let db_phone = "1234567890";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // Lookup without + should match
        assert_eq!(index.lookup("1234567890"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_us_without_country_code_miss_without_plus() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Jane".to_string(),
            last: "Smith".to_string(),
        };
        // Simulate building from db with "1234567890" (no +1)
        let db_phone = "1234567890";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // For 10-digit db entry, no US variants added, so these should not match
        assert_eq!(index.lookup("34567890"), None);
    }

    #[test]
    fn test_phone_lookup_us_without_country_code_miss_with_plus() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Jane".to_string(),
            last: "Smith".to_string(),
        };
        // Simulate building from db with "1234567890" (no +1)
        let db_phone = "1234567890";
        let keys = phone_keys(db_phone);
        for key in keys {
            index.by_phone.insert(key, name.clone());
        }

        // For 10-digit db entry, no US variants added, so these should not match
        assert_eq!(index.lookup("+34567890"), None);
    }

    #[test]
    fn test_phone_lookup_uk_with_plus44() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Alice".to_string(),
            last: "Brown".to_string(),
        };
        // UK number +44 20 1234 5678
        index
            .by_phone
            .insert("+442012345678".to_string(), name.clone());

        // Lookup with +44 should match
        assert_eq!(index.lookup("+442012345678"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_uk_without_plus() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Alice".to_string(),
            last: "Brown".to_string(),
        };
        // UK number +44 20 1234 5678
        index
            .by_phone
            .insert("+442012345678".to_string(), name.clone());

        // Lookup without + should match
        assert_eq!(index.lookup("442012345678"), Some(name.clone()));
    }

    #[test]
    fn test_phone_lookup_miss_without_plus() {
        let index = ContactsIndex::default();
        // No entries, should miss
        assert_eq!(index.lookup("1234567890"), None);
    }

    #[test]
    fn test_phone_lookup_miss_with_plus() {
        let index = ContactsIndex::default();
        // No entries, should miss
        assert_eq!(index.lookup("+1234567890"), None);
    }

    #[test]
    fn test_email_lookup_match_exact() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Bob".to_string(),
            last: "Wilson".to_string(),
        };
        index
            .by_email
            .insert("bob@example.com".to_string(), name.clone());

        // Exact match
        assert_eq!(index.lookup("bob@example.com"), Some(name.clone()));
    }

    #[test]
    fn test_email_lookup_match_case_insensitive() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Bob".to_string(),
            last: "Wilson".to_string(),
        };
        index
            .by_email
            .insert("bob@example.com".to_string(), name.clone());

        // Case insensitive
        assert_eq!(index.lookup("BOB@EXAMPLE.COM"), Some(name.clone()));
    }

    #[test]
    fn test_email_lookup_match_with_angle_brackets() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Bob".to_string(),
            last: "Wilson".to_string(),
        };
        index
            .by_email
            .insert("bob@example.com".to_string(), name.clone());

        // With angle brackets
        assert_eq!(index.lookup("<bob@example.com>"), Some(name.clone()));
    }

    #[test]
    fn test_email_lookup_match_trimmed() {
        let mut index = ContactsIndex::default();
        let name = Name {
            first: "Bob".to_string(),
            last: "Wilson".to_string(),
        };
        index
            .by_email
            .insert("bob@example.com".to_string(), name.clone());

        // Trimmed
        assert_eq!(index.lookup(" bob@example.com "), Some(name.clone()));
    }

    #[test]
    fn test_email_lookup_miss_no_entries() {
        let index = ContactsIndex::default();
        // No entries
        assert_eq!(index.lookup("not@here.com"), None);
    }

    #[test]
    fn test_email_lookup_miss_phone_like() {
        let index = ContactsIndex::default();
        // Phone-like but looks like email
        assert_eq!(index.lookup("123@456"), None);
    }

    #[test]
    fn test_phone_keys_contains_digits() {
        // Test phone_keys function
        let keys = phone_keys("+12345678901");
        assert!(keys.contains(&"12345678901".to_string()));
    }

    #[test]
    fn test_phone_keys_contains_with_plus() {
        // Test phone_keys function
        let keys = phone_keys("+12345678901");
        assert!(keys.contains(&"+12345678901".to_string()));
    }

    #[test]
    fn test_phone_keys_contains_last_10() {
        // Test phone_keys function
        let keys = phone_keys("+12345678901");
        assert!(keys.contains(&"2345678901".to_string()));
    }

    #[test]
    fn test_phone_keys_contains_last_10_with_plus() {
        // Test phone_keys function
        let keys = phone_keys("+12345678901");
        assert!(keys.contains(&"+2345678901".to_string()));
    }
}

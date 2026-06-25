# Backup Extractor Implementation Plan (Increment 1: core + contacts)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `backup-core` library and a `backup-extractor` CLI that extracts **contacts** from an on-disk iOS backup (encrypted or not) to `csv`, `json`, `vcf`, and `html`.

**Architecture:** `backup-core` wraps `crabapple` and is the only crate that opens/decrypts a backup and yields plain files. `backup-extractor` is a `clap` CLI whose `contacts` subcommand reads `AddressBook.sqlitedb` (via `backup-core`) into a `Vec<Contact>` model and renders it through format functions. SQLite is read read-only with `rusqlite`.

**Tech Stack:** Rust (edition 2024), `crabapple` 0.4.7, `rusqlite` 0.40 (bundled), `clap` 4.6, `serde`/`serde_json`, `csv`, `askama` 0.16.

## Global Constraints

- Workspace root `Cargo.toml` already has `members = ["imessage-database", "imessage-exporter"]`; add `"backup-core"` and `"backup-extractor"`.
- Edition `2024` for both new crates (match the workspace).
- Pin dependency versions with `=` to match the existing style (e.g. `clap = "=4.6.1"`, `rusqlite = { version = "=0.40.0", features = ["bundled"] }`, `askama = "=0.16.0"`).
- `backup-core` is the ONLY place that may reference `crabapple` or backup/encryption details.
- A missing store (data not in the backup) is a clean exit, never a panic.
- crabapple opens unencrypted backups too; pass `Authentication::Password(pw.unwrap_or_default())` — the password is ignored when the backup is not encrypted.
- This increment defers: addresses on contacts, the `pdf` format, and the calls/photos/notes extractors. Each is a follow-on plan reusing these patterns.

---

## File Structure

- `backup-core/Cargo.toml` — crate manifest (deps: `crabapple`).
- `backup-core/src/lib.rs` — `BackupError`, `Backup`, `DeviceInfo`, `Backup::open`, `device_info`, `fetch`.
- `backup-extractor/Cargo.toml` — crate manifest (deps: `backup-core`, `rusqlite`, `clap`, `serde`, `serde_json`, `csv`, `askama`).
- `backup-extractor/src/main.rs` — CLI definition and dispatch.
- `backup-extractor/src/contacts.rs` — `Contact`, `Labeled`, `parse(db_path) -> Vec<Contact>`.
- `backup-extractor/src/format.rs` — `Format` enum + `csv`/`json`/`vcard`/`html` writers for contacts.
- `backup-extractor/templates/contacts.html` — askama template.
- `backup-extractor/src/test_fixtures.rs` — `make_addressbook(path)` builds a real `AddressBook.sqlitedb` fixture for tests.

---

## Task 1: Scaffold the two crates

**Files:**
- Create: `backup-core/Cargo.toml`, `backup-core/src/lib.rs`
- Create: `backup-extractor/Cargo.toml`, `backup-extractor/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Produces: two buildable crates registered in the workspace.

- [ ] **Step 1: Add the crates to the workspace**

In `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["imessage-database", "imessage-exporter", "backup-core", "backup-extractor"]
```

- [ ] **Step 2: Write `backup-core/Cargo.toml`**

```toml
[package]
name = "backup-core"
version = "0.0.0"
edition = "2024"
license = "GPL-3.0-or-later"

[dependencies]
crabapple = "=0.4.7"
```

- [ ] **Step 3: Write a minimal `backup-core/src/lib.rs`**

```rust
//! Open, decrypt, and read files from an on-disk iOS backup.

/// Errors from opening or reading a backup.
#[derive(Debug)]
pub enum BackupError {
    /// The backup could not be opened or decrypted (wrong/missing password,
    /// corrupt manifest, …). Carries a human-readable reason.
    Open(String),
    /// An I/O error while materializing a decrypted file.
    Io(std::io::Error),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Open(why) => write!(f, "could not open backup: {why}"),
            BackupError::Io(why) => write!(f, "backup I/O error: {why}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<std::io::Error> for BackupError {
    fn from(value: std::io::Error) -> Self {
        BackupError::Io(value)
    }
}
```

- [ ] **Step 4: Write `backup-extractor/Cargo.toml`**

```toml
[package]
name = "backup-extractor"
version = "0.0.0"
edition = "2024"
license = "GPL-3.0-or-later"

[dependencies]
backup-core = { path = "../backup-core" }
clap = { version = "=4.6.1", features = ["cargo", "derive"] }
rusqlite = { version = "=0.40.0", features = ["bundled"] }
serde = { version = "=1.0.228", features = ["derive"] }
serde_json = "=1.0.145"
csv = "=1.3.1"
askama = "=0.16.0"
```

- [ ] **Step 5: Write a minimal `backup-extractor/src/main.rs`**

```rust
fn main() {
    println!("backup-extractor");
}
```

- [ ] **Step 6: Build and commit**

Run: `cargo build -p backup-core -p backup-extractor`
Expected: `Finished` with no errors.

```bash
git add Cargo.toml backup-core backup-extractor
git commit -m "feat: scaffold backup-core and backup-extractor crates"
```

---

## Task 2: `backup-core` — open a backup and read device info

**Files:**
- Modify: `backup-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct DeviceInfo { pub device_name: String, pub product_version: String, pub udid: String }`
  - `pub struct Backup` (opaque; wraps `crabapple::Backup`)
  - `pub fn Backup::open(dir: &std::path::Path, password: Option<&str>) -> Result<Backup, BackupError>`
  - `pub fn Backup::device_info(&self) -> &DeviceInfo`

- [ ] **Step 1: Write the failing test**

Append to `backup-core/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Integration test against a real backup. Set BACKUP_EXTRACTOR_TEST_BACKUP
    // to a backup directory (and BACKUP_EXTRACTOR_TEST_PASSWORD if encrypted).
    // Skipped when the env var is unset so CI stays green without fixtures.
    #[test]
    fn opens_real_backup_and_reads_device_info() {
        let Ok(dir) = std::env::var("BACKUP_EXTRACTOR_TEST_BACKUP") else {
            eprintln!("skipping: set BACKUP_EXTRACTOR_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("BACKUP_EXTRACTOR_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref())
            .expect("open backup");
        let info = backup.device_info();
        assert!(!info.product_version.is_empty(), "iOS version should be set");
    }

    #[test]
    fn error_display_is_readable() {
        let e = BackupError::Open("bad password".into());
        assert_eq!(e.to_string(), "could not open backup: bad password");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p backup-core`
Expected: FAIL to compile — `Backup`, `DeviceInfo`, `Backup::open`, `device_info` are undefined.

- [ ] **Step 3: Implement `Backup::open` and `device_info`**

Insert into `backup-core/src/lib.rs` (above the tests):

```rust
use std::path::Path;

use crabapple::{Authentication, Backup as RawBackup};

/// Device metadata read from the backup's lockdown record.
pub struct DeviceInfo {
    pub device_name: String,
    pub product_version: String,
    pub udid: String,
}

/// An opened (and, if needed, unlocked) iOS backup.
pub struct Backup {
    raw: RawBackup,
    info: DeviceInfo,
}

impl Backup {
    /// Open a backup directory. `password` is required for encrypted backups and
    /// ignored for unencrypted ones.
    pub fn open(dir: &Path, password: Option<&str>) -> Result<Self, BackupError> {
        let dir_str = dir.to_str().ok_or_else(|| {
            BackupError::Open(format!("non-UTF-8 backup path {}", dir.display()))
        })?;
        let auth = Authentication::Password(password.unwrap_or_default().to_string());
        let raw = RawBackup::open(dir_str, &auth)
            .map_err(|why| BackupError::Open(why.to_string()))?;

        let lockdown = raw.lockdown();
        let info = DeviceInfo {
            device_name: lockdown.device_name.clone(),
            product_version: lockdown.product_version.clone(),
            udid: raw.udid().map_err(|why| BackupError::Open(why.to_string()))?,
        };
        Ok(Backup { raw, info })
    }

    pub fn device_info(&self) -> &DeviceInfo {
        &self.info
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p backup-core`
Expected: PASS (`error_display_is_readable` passes; the integration test prints "skipping" unless the env var is set).

- [ ] **Step 5: Commit**

```bash
git add backup-core/src/lib.rs
git commit -m "feat(backup-core): open backup and read device info"
```

---

## Task 3: `backup-core` — fetch a single file by domain + path

**Files:**
- Modify: `backup-core/src/lib.rs`

**Interfaces:**
- Consumes: `Backup`, `BackupError` from Task 2.
- Produces: `pub fn Backup::fetch(&self, domain: &str, relative_path: &str, dest: &std::path::Path) -> Result<Option<std::path::PathBuf>, BackupError>` — decrypts the matching entry to `dest` and returns its path, or `Ok(None)` when the backup has no such file.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `backup-core/src/lib.rs`:

```rust
    #[test]
    fn fetch_returns_none_for_absent_file() {
        let Ok(dir) = std::env::var("BACKUP_EXTRACTOR_TEST_BACKUP") else {
            eprintln!("skipping: set BACKUP_EXTRACTOR_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("BACKUP_EXTRACTOR_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let out = std::env::temp_dir().join("be-fetch-none.bin");
        let got = backup
            .fetch("NoSuchDomain", "no/such/file", &out)
            .expect("fetch should not error");
        assert!(got.is_none(), "absent file must return None");
    }

    #[test]
    fn fetch_writes_address_book_when_present() {
        let Ok(dir) = std::env::var("BACKUP_EXTRACTOR_TEST_BACKUP") else {
            eprintln!("skipping: set BACKUP_EXTRACTOR_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("BACKUP_EXTRACTOR_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let out = std::env::temp_dir().join("be-fetch-ab.sqlitedb");
        let _ = std::fs::remove_file(&out);
        if let Some(path) = backup
            .fetch("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb", &out)
            .unwrap()
        {
            // SQLite files start with the "SQLite format 3\0" magic.
            let head = std::fs::read(&path).unwrap();
            assert!(head.starts_with(b"SQLite format 3\0"), "decrypted a real DB");
        } else {
            eprintln!("backup has no AddressBook; skipping content assertion");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p backup-core`
Expected: FAIL to compile — `fetch` is undefined.

- [ ] **Step 3: Implement `fetch`**

Add the `use std::io::Write;` import at the top of `backup-core/src/lib.rs`, and this method to `impl Backup`:

```rust
    /// Decrypt the file at `domain` + `relative_path` to `dest` and return its
    /// path, or `Ok(None)` when the backup contains no such file.
    pub fn fetch(
        &self,
        domain: &str,
        relative_path: &str,
        dest: &Path,
    ) -> Result<Option<PathBuf>, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let Some(entry) = entries
            .into_iter()
            .find(|e| e.domain == domain && e.relative_path == relative_path)
        else {
            return Ok(None);
        };
        let bytes = self
            .raw
            .decrypt_entry(&entry)
            .map_err(|why| BackupError::Open(why.to_string()))?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::File::create(dest)?.write_all(&bytes)?;
        Ok(Some(dest.to_path_buf()))
    }
```

Add `use std::path::PathBuf;` to the existing path import (`use std::path::{Path, PathBuf};`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p backup-core`
Expected: PASS (integration tests skip without the env var).

- [ ] **Step 5: Commit**

```bash
git add backup-core/src/lib.rs
git commit -m "feat(backup-core): fetch and decrypt a file by domain and path"
```

---

## Task 4: Contacts model and parser

**Files:**
- Create: `backup-extractor/src/contacts.rs`, `backup-extractor/src/test_fixtures.rs`
- Modify: `backup-extractor/src/main.rs` (declare modules)

**Interfaces:**
- Produces:
  - `pub struct Labeled { pub label: String, pub value: String }`
  - `pub struct Contact { pub first: String, pub last: String, pub organization: String, pub phones: Vec<Labeled>, pub emails: Vec<Labeled>, pub note: String }` (both `#[derive(serde::Serialize)]`)
  - `pub fn parse(db_path: &std::path::Path) -> rusqlite::Result<Vec<Contact>>`
  - test helper `pub fn make_addressbook(path: &std::path::Path)` (under `#[cfg(test)]`)

- [ ] **Step 1: Write the fixture builder**

Create `backup-extractor/src/test_fixtures.rs`:

```rust
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
```

- [ ] **Step 2: Write the failing parser test**

Create `backup-extractor/src/contacts.rs` with only the test first:

```rust
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

        let mut people = parse(&db).unwrap();
        people.sort_by(|a, b| a.organization.cmp(&b.organization));

        assert_eq!(people.len(), 2);

        let firma = &people[1]; // "Firma s.r.o." sorts after "Acme"? -> sort by org
        let jan = people.iter().find(|c| c.first == "Jan").unwrap();
        assert_eq!(jan.last, "Novák");
        assert_eq!(jan.organization, "Acme");
        assert_eq!(jan.note, "kamarád");
        assert_eq!(jan.phones, vec![Labeled { label: "Mobile".into(), value: "+420776452878".into() }]);
        assert_eq!(jan.emails, vec![Labeled { label: "Home".into(), value: "jan@example.cz".into() }]);

        let company = people.iter().find(|c| c.organization == "Firma s.r.o.").unwrap();
        assert_eq!(company.first, "");
        assert!(company.phones.is_empty());
        let _ = firma;

        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p backup-extractor contacts`
Expected: FAIL to compile — `parse`, `Contact`, `Labeled` are undefined.

- [ ] **Step 4: Implement the model and parser**

Prepend to `backup-extractor/src/contacts.rs` (above the tests):

```rust
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
```

- [ ] **Step 5: Declare the modules**

Replace `backup-extractor/src/main.rs` with:

```rust
mod contacts;
mod format;
#[cfg(test)]
mod test_fixtures;

fn main() {
    println!("backup-extractor");
}
```

(`format` is added in Task 5; create an empty `backup-extractor/src/format.rs` now so this compiles, or reorder to do Task 5 first. To keep this task self-contained, also create `backup-extractor/src/format.rs` containing just `// formatters` for now.)

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p backup-extractor contacts`
Expected: PASS (`parses_people_phones_and_emails`).

- [ ] **Step 7: Commit**

```bash
git add backup-extractor/src/contacts.rs backup-extractor/src/test_fixtures.rs backup-extractor/src/main.rs backup-extractor/src/format.rs
git commit -m "feat(contacts): parse AddressBook.sqlitedb into a Contact model"
```

---

## Task 5: Contacts formatters — csv, json, vcard, html

**Files:**
- Modify: `backup-extractor/src/format.rs`
- Create: `backup-extractor/templates/contacts.html`

**Interfaces:**
- Consumes: `Contact`, `Labeled` from Task 4.
- Produces:
  - `pub enum Format { Csv, Json, Vcf, Html }` with `pub fn from_cli(&str) -> Option<Format>`
  - `pub fn contacts_csv(&[Contact]) -> String`
  - `pub fn contacts_json(&[Contact]) -> String`
  - `pub fn contacts_vcard(&[Contact]) -> String`
  - `pub fn contacts_html(&[Contact]) -> String`

- [ ] **Step 1: Write the failing tests**

Replace `backup-extractor/src/format.rs` with a tests module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{Contact, Labeled};

    fn sample() -> Vec<Contact> {
        vec![Contact {
            first: "Jan".into(),
            last: "Novák".into(),
            organization: "Acme".into(),
            phones: vec![Labeled { label: "Mobile".into(), value: "+420776452878".into() }],
            emails: vec![Labeled { label: "Home".into(), value: "jan@example.cz".into() }],
            note: "kamarád".into(),
        }]
    }

    #[test]
    fn format_from_cli_parses_known() {
        assert_eq!(Format::from_cli("csv"), Some(Format::Csv));
        assert_eq!(Format::from_cli("VCF"), Some(Format::Vcf));
        assert_eq!(Format::from_cli("nope"), None);
    }

    #[test]
    fn csv_has_header_and_row() {
        let out = contacts_csv(&sample());
        assert!(out.starts_with("first,last,organization,phones,emails,note"));
        assert!(out.contains("Jan,Novák,Acme,Mobile: +420776452878,Home: jan@example.cz,kamarád"));
    }

    #[test]
    fn json_roundtrips() {
        let out = contacts_json(&sample());
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["first"], "Jan");
        assert_eq!(back[0]["phones"][0]["value"], "+420776452878");
    }

    #[test]
    fn vcard_is_wellformed() {
        let out = contacts_vcard(&sample());
        assert!(out.contains("BEGIN:VCARD"));
        assert!(out.contains("VERSION:3.0"));
        assert!(out.contains("FN:Jan Novák"));
        assert!(out.contains("N:Novák;Jan;;;"));
        assert!(out.contains("ORG:Acme"));
        assert!(out.contains("TEL;TYPE=Mobile:+420776452878"));
        assert!(out.contains("EMAIL;TYPE=Home:jan@example.cz"));
        assert!(out.trim_end().ends_with("END:VCARD"));
    }

    #[test]
    fn html_lists_the_contact() {
        let out = contacts_html(&sample());
        assert!(out.contains("<html"));
        assert!(out.contains("Jan Novák"));
        assert!(out.contains("+420776452878"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p backup-extractor format`
Expected: FAIL to compile — `Format`, `contacts_csv`, etc. are undefined.

- [ ] **Step 3: Write the askama template**

Create `backup-extractor/templates/contacts.html`:

```html
<!doctype html>
<html lang="cs"><head><meta charset="utf-8"><title>Kontakty</title>
<style>body{font-family:-apple-system,Helvetica,Arial,sans-serif;margin:24px}
.c{border-bottom:1px solid #ddd;padding:8px 0}.n{font-weight:600}.o{color:#666}
.v{margin-left:12px}</style></head><body>
<h1>Kontakty ({{ contacts.len() }})</h1>
{% for c in contacts %}
<div class="c">
  <div class="n">{{ c.first }} {{ c.last }}{% if !c.organization.is_empty() %} <span class="o">— {{ c.organization }}</span>{% endif %}</div>
  {% for p in c.phones %}<div class="v">📞 {{ p.label }}: {{ p.value }}</div>{% endfor %}
  {% for e in c.emails %}<div class="v">✉️ {{ e.label }}: {{ e.value }}</div>{% endfor %}
  {% if !c.note.is_empty() %}<div class="v">📝 {{ c.note }}</div>{% endif %}
</div>
{% endfor %}
</body></html>
```

- [ ] **Step 4: Implement the formatters**

Prepend to `backup-extractor/src/format.rs` (above the tests):

```rust
//! Render contacts to the supported output formats.

use askama::Template;

use crate::contacts::Contact;

/// Output format chosen with `-f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Vcf,
    Html,
}

impl Format {
    pub fn from_cli(value: &str) -> Option<Format> {
        match value.to_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            "vcf" | "vcard" => Some(Format::Vcf),
            "html" => Some(Format::Html),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Vcf => "vcf",
            Format::Html => "html",
        }
    }
}

fn join_labeled(items: &[crate::contacts::Labeled]) -> String {
    items
        .iter()
        .map(|l| format!("{}: {}", l.label, l.value))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn contacts_csv(contacts: &[Contact]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["first", "last", "organization", "phones", "emails", "note"])
        .unwrap();
    for c in contacts {
        wtr.write_record([
            &c.first,
            &c.last,
            &c.organization,
            &join_labeled(&c.phones),
            &join_labeled(&c.emails),
            &c.note,
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn contacts_json(contacts: &[Contact]) -> String {
    serde_json::to_string_pretty(contacts).unwrap()
}

pub fn contacts_vcard(contacts: &[Contact]) -> String {
    let mut out = String::new();
    for c in contacts {
        out.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        out.push_str(&format!("N:{};{};;;\r\n", c.last, c.first));
        let fticket = format!("{} {}", c.first, c.last);
        out.push_str(&format!("FN:{}\r\n", fticket.trim()));
        if !c.organization.is_empty() {
            out.push_str(&format!("ORG:{}\r\n", c.organization));
        }
        for p in &c.phones {
            out.push_str(&format!("TEL;TYPE={}:{}\r\n", p.label, p.value));
        }
        for e in &c.emails {
            out.push_str(&format!("EMAIL;TYPE={}:{}\r\n", e.label, e.value));
        }
        if !c.note.is_empty() {
            out.push_str(&format!("NOTE:{}\r\n", c.note));
        }
        out.push_str("END:VCARD\r\n");
    }
    out
}

#[derive(Template)]
#[template(path = "contacts.html")]
struct ContactsTemplate<'a> {
    contacts: &'a [Contact],
}

pub fn contacts_html(contacts: &[Contact]) -> String {
    ContactsTemplate { contacts }.render().unwrap()
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p backup-extractor format`
Expected: PASS (all four formatter tests).

- [ ] **Step 6: Commit**

```bash
git add backup-extractor/src/format.rs backup-extractor/templates/contacts.html
git commit -m "feat(contacts): csv, json, vcard, and html formatters"
```

---

## Task 6: CLI wiring — `contacts` subcommand end to end

**Files:**
- Modify: `backup-extractor/src/main.rs`

**Interfaces:**
- Consumes: `backup_core::Backup`, `contacts::parse`, `format::{Format, contacts_csv, contacts_json, contacts_vcard, contacts_html}`.
- Produces: a runnable CLI `backup-extractor --backup <dir> [--password <pw>] contacts -f <fmt> -o <out>`.

- [ ] **Step 1: Write the failing test for argument parsing**

Replace `backup-extractor/src/main.rs` with the structure plus a parse test:

```rust
mod contacts;
mod format;
#[cfg(test)]
mod test_fixtures;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::format::Format;

#[derive(Parser)]
#[command(name = "backup-extractor", about = "Extract personal data from an iOS backup")]
struct Cli {
    /// Path to the iOS backup directory.
    #[arg(long)]
    backup: PathBuf,
    /// Password for an encrypted backup (ignored for unencrypted backups).
    #[arg(long)]
    password: Option<String>,
    /// Output directory.
    #[arg(long, short = 'o')]
    out: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Export contacts.
    Contacts {
        /// Output format: csv, json, vcf, html.
        #[arg(long, short = 'f')]
        format: String,
    },
}

fn main() {
    if let Err(why) = run() {
        eprintln!("error: {why}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Contacts { format } => run_contacts(&cli, format),
    }
}

fn run_contacts(cli: &Cli, format: &str) -> Result<(), Box<dyn std::error::Error>> {
    let format = Format::from_cli(format)
        .ok_or_else(|| format!("unknown contacts format `{format}` (use csv, json, vcf, html)"))?;

    let backup = backup_core::Backup::open(&cli.backup, cli.password.as_deref())?;
    std::fs::create_dir_all(&cli.out)?;
    let db = cli.out.join(".AddressBook.sqlitedb");
    let Some(db) = backup.fetch("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb", &db)? else {
        println!("This backup has no contacts; nothing to export.");
        return Ok(());
    };

    let people = contacts::parse(&db)?;
    let _ = std::fs::remove_file(&db);

    let rendered = match format {
        Format::Csv => format::contacts_csv(&people),
        Format::Json => format::contacts_json(&people),
        Format::Vcf => format::contacts_vcard(&people),
        Format::Html => format::contacts_html(&people),
    };
    let out_file = cli.out.join(format!("contacts.{}", format.extension()));
    std::fs::write(&out_file, rendered)?;
    println!("Wrote {} contact(s) to {}", people.len(), out_file.display());
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn parses_contacts_invocation() {
        let cli = Cli::try_parse_from([
            "backup-extractor", "--backup", "/b", "-o", "/out", "contacts", "-f", "vcf",
        ])
        .unwrap();
        assert_eq!(cli.backup, PathBuf::from("/b"));
        match cli.command {
            Command::Contacts { format } => assert_eq!(format, "vcf"),
        }
    }

    #[test]
    fn rejects_missing_backup() {
        assert!(Cli::try_parse_from(["backup-extractor", "-o", "/out", "contacts", "-f", "csv"]).is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail then pass**

Run: `cargo test -p backup-extractor cli`
Expected: PASS once it compiles (the CLI types are defined in this same file). If compilation fails, fix the referenced names to match Tasks 4–5 exactly.

- [ ] **Step 3: Manually verify against a real backup (optional, env-gated)**

Run (only where a real backup exists):
`cargo run -p backup-extractor -- --backup "<backup dir>" --password "<pw>" -o /tmp/be-out contacts -f vcf`
Expected: prints `Wrote N contact(s) to /tmp/be-out/contacts.vcf` and the file opens in Contacts.app.

- [ ] **Step 4: Run the full crate test suite**

Run: `cargo test -p backup-extractor`
Expected: PASS (contacts, format, cli suites).

- [ ] **Step 5: Commit**

```bash
git add backup-extractor/src/main.rs
git commit -m "feat: wire contacts subcommand end to end"
```

---

## Task 7: Docs and review

**Files:**
- Create: `backup-extractor/README.md`

- [ ] **Step 1: Write the README**

Create `backup-extractor/README.md`:

```markdown
# backup-extractor

Extract personal data from an on-disk iOS backup (encrypted or not).

## Usage

```
backup-extractor --backup <backup-dir> [--password <pw>] -o <out> contacts -f <csv|json|vcf|html>
```

`--password` is only needed for encrypted backups. A backup that does not
contain a given data store exits cleanly without writing output.

## Status

- [x] contacts (csv, json, vcf, html)
- [ ] calls
- [ ] photos
- [ ] notes
- [ ] pdf output
```

- [ ] **Step 2: Commit and run roborev**

```bash
git add backup-extractor/README.md
git commit -m "docs: backup-extractor usage and status"
```

Then `roborev show HEAD` and fix any reported findings before opening a PR.

---

## Self-Review notes

- **Spec coverage (this increment):** `backup-core` open/decrypt/fetch (Tasks 2–3 ✓), Contacts extractor + model (Task 4 ✓), structured + readable formats csv/json/vcf/html (Task 5 ✓), CLI mirroring `imessage-exporter` (Task 6 ✓), missing-store clean exit (Task 6 ✓), fixture-based tests (Tasks 4–5 ✓), encrypted-backup support via password (Tasks 2,6 ✓).
- **Deferred to follow-on plans (per spec build order):** calls, photos, notes extractors; the `pdf` formatter and the shared `webkit2pdf` engine; contact addresses (`property 5` + `ABMultiValueEntry`). Each reuses the extractor + formatter pattern established here.
- **Type consistency:** `Contact`/`Labeled` fields and the `Format` variants are referenced identically across Tasks 4–6; `Backup::{open,device_info,fetch}` signatures match between `backup-core` (Tasks 2–3) and the CLI (Task 6).
- **Risk:** crabapple's exact `lockdown()`/`udid()`/`entries()`/`decrypt_entry()` names were taken from docs.rs; if a name differs in 0.4.7, adjust in Task 2/3 only (the rest of the plan is insulated by `backup-core`).

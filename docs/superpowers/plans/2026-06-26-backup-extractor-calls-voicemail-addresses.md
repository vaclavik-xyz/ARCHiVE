# backup-extractor Increment 2 — Calls, Voicemail & Contact Addresses Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three pure-SQLite extractors to `backup-extractor` — call history, voicemail metadata, and contact postal addresses — each following the Increment 1 extractor → formatter → JSON-envelope pattern.

**Architecture:** Each store is a small SQLite database fetched into a secure temp dir and parsed in memory (no streaming, no protobuf). Two new parser modules (`calls.rs`, `voicemail.rs`) plus an extension to `contacts.rs`; rendering lives in `format.rs` with one askama template per record type; CLI wiring in `main.rs`. A shared `datetime.rs` owns the two timestamp-epoch conversions (Cocoa 2001 and Unix), and a shared `sqlite_util.rs` owns runtime column detection so parsers tolerate iOS-version schema variance.

**Tech Stack:** Rust 2024, `rusqlite` (`=0.40.0`, bundled), `serde`/`serde_json`, `csv`, `askama` (`=0.16.0`), `tempfile`, `chrono` (`=0.4.44`, newly declared — already a same-pin dep of `imessage-database`, so no new transitive crates), `backup-core`.

## Global Constraints

- **Agent-first contract unchanged.** A command that parses far enough to run prints exactly one JSON object on stdout; human progress to stderr. Exit codes: `0` success (incl. store-absent), `2` auth-only (locked/wrong-password backup, as a JSON `kind:"auth"` envelope), `1` usage/other (JSON envelope). The one non-JSON exception is a malformed clap invocation (empty stdout, usage on stderr, exit 2) — documented in `AGENTS.md`; do not weaken that wording.
- **Password** from `--password` or `BACKUP_EXTRACTOR_PASSWORD`; never a blocking prompt.
- **All version-pinned deps stay `=`-pinned.**
- **Defensive column detection.** Every parser probes real columns via `PRAGMA table_info(<table>)` (through `sqlite_util::table_columns`) and selects only present columns, mapping absent optional columns to `NULL`. Required core columns missing ⇒ a parse error, not a silent empty result.
- **Determinism.** Calls and voicemail order by timestamp ascending (`ORDER BY ZDATE` / `ORDER BY date`); addresses keep DB order within a contact.
- **No invented semantics.** FaceTime audio/video (`ZCALLTYPE` 8/16) and voicemail `flags` bits are undocumented/version-dependent: surface the raw value and, at most, a clearly best-effort derivation. Never present a guess as certain.
- **Honest-uncertainty fields:** `Call.video` (best-effort) is backed by raw `Call.call_type`; `Voicemail.flags` is raw and not decoded (use `trashed`).
- **Verification per task:** `cargo test -p backup-extractor` (all pass) and `cargo clippy --all-targets -p backup-extractor` (no warnings — `--all-targets` so test usage counts toward dead-code analysis).
- **Templates auto-escape.** askama escapes `{{ }}` by default; never bypass it for backup-derived data.

---

### Task 1: Calls parser + shared `datetime` and `sqlite_util` foundations

**Files:**
- Modify: `backup-extractor/Cargo.toml` (add `chrono`)
- Create: `backup-extractor/src/datetime.rs`
- Create: `backup-extractor/src/sqlite_util.rs`
- Create: `backup-extractor/src/calls.rs`
- Modify: `backup-extractor/src/test_fixtures.rs` (add `make_callhistory`)
- Modify: `backup-extractor/src/main.rs` (add `mod datetime; mod sqlite_util; mod calls;`)

**Interfaces:**
- Produces: `datetime::unix_to_iso(seconds: i64) -> Option<String>`, `datetime::cocoa_to_iso(seconds: f64) -> Option<String>`; `sqlite_util::table_columns(conn: &rusqlite::Connection, table: &str) -> rusqlite::Result<std::collections::HashSet<String>>`; `calls::Call` (struct below) and `calls::parse(db_path: &std::path::Path) -> rusqlite::Result<Vec<calls::Call>>`; `test_fixtures::make_callhistory(path: &std::path::Path)`.
- Consumes: nothing from other Increment-2 tasks.

- [ ] **Step 1: Add the `chrono` dependency**

In `backup-extractor/Cargo.toml`, under `[dependencies]`, add (keep alphabetical-ish with the others):

```toml
chrono = "=0.4.44"
```

- [ ] **Step 2: Write the failing `datetime` tests**

Create `backup-extractor/src/datetime.rs`:

```rust
//! Convert iOS timestamp epochs to ISO 8601 (RFC 3339) UTC strings.

use chrono::DateTime;

/// Seconds between the Cocoa / Core Data reference date (2001-01-01 UTC) and the
/// Unix epoch (1970-01-01 UTC).
const COCOA_EPOCH_OFFSET: i64 = 978_307_200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_zero_is_1970() {
        assert_eq!(unix_to_iso(0).unwrap(), "1970-01-01T00:00:00+00:00");
    }

    #[test]
    fn cocoa_epoch_zero_is_2001() {
        assert_eq!(cocoa_to_iso(0.0).unwrap(), "2001-01-01T00:00:00+00:00");
    }

    #[test]
    fn cocoa_offset_matches_unix() {
        assert_eq!(cocoa_to_iso(0.0), unix_to_iso(COCOA_EPOCH_OFFSET));
    }

    #[test]
    fn cocoa_truncates_fractional_seconds() {
        assert_eq!(cocoa_to_iso(100.9).unwrap(), "2001-01-01T00:01:40+00:00");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p backup-extractor datetime`
Expected: FAIL — `cannot find function unix_to_iso` / `cocoa_to_iso`.

- [ ] **Step 4: Implement the conversions**

In `backup-extractor/src/datetime.rs`, above the `#[cfg(test)]` module:

```rust
/// Seconds since the Unix epoch → ISO 8601 (RFC 3339) UTC, or `None` when out of
/// the representable range.
pub fn unix_to_iso(seconds: i64) -> Option<String> {
    DateTime::from_timestamp(seconds, 0).map(|dt| dt.to_rfc3339())
}

/// Seconds since the Cocoa / Core Data reference date (2001-01-01 UTC) → ISO 8601
/// (RFC 3339) UTC, or `None` when out of range. Fractional seconds are truncated.
pub fn cocoa_to_iso(seconds: f64) -> Option<String> {
    unix_to_iso(seconds.trunc() as i64 + COCOA_EPOCH_OFFSET)
}
```

- [ ] **Step 5: Run the `datetime` tests to verify they pass**

Run: `cargo test -p backup-extractor datetime`
Expected: PASS (4 tests).

- [ ] **Step 6: Write the failing `sqlite_util` test**

Create `backup-extractor/src/sqlite_util.rs`:

```rust
//! Small shared SQLite helpers for the store parsers.

use std::collections::HashSet;

use rusqlite::Connection;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_existing_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (a INTEGER, b TEXT);").unwrap();
        let cols = table_columns(&conn, "t").unwrap();
        assert!(cols.contains("a"));
        assert!(cols.contains("b"));
        assert!(!cols.contains("c"));
    }
}
```

- [ ] **Step 7: Run it to verify it fails**

Run: `cargo test -p backup-extractor sqlite_util`
Expected: FAIL — `cannot find function table_columns`.

- [ ] **Step 8: Implement `table_columns`**

In `backup-extractor/src/sqlite_util.rs`, above the test module:

```rust
/// Column names present in `table`, via `PRAGMA table_info`. `table` must be a
/// trusted literal — it is interpolated into the pragma, not bound.
pub fn table_columns(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<String>>>()?;
    Ok(cols)
}
```

- [ ] **Step 9: Run it to verify it passes**

Run: `cargo test -p backup-extractor sqlite_util`
Expected: PASS.

- [ ] **Step 10: Add the `make_callhistory` fixture**

In `backup-extractor/src/test_fixtures.rs`, append:

```rust
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
```

- [ ] **Step 11: Write the failing `calls` parser tests**

Create `backup-extractor/src/calls.rs`:

```rust
//! Read call history from an iOS `CallHistory.storedata` (Core Data SQLite).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One call-history record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Call {
    /// Remote party: a phone number, or an Apple ID/email for FaceTime calls.
    pub number: String,
    /// Call start time as ISO 8601 (RFC 3339) UTC; empty if unconvertible.
    pub date: String,
    /// Duration in whole seconds (0 for unanswered).
    pub duration_seconds: i64,
    /// `"incoming"` or `"outgoing"`.
    pub direction: String,
    /// Whether the call was answered (false = missed/declined/no-answer).
    pub answered: bool,
    /// `"phone"`, `"facetime"`, a raw third-party bundle id, or `"unknown"`.
    pub service: String,
    /// Best-effort FaceTime video flag (true=video, false=audio); `None` when not
    /// derivable. Version-dependent and undocumented — see `call_type`.
    pub video: Option<bool>,
    /// Raw `ZCALLTYPE` integer, preserved for fidelity.
    pub call_type: Option<i64>,
    /// Optional carrier/region location hint.
    pub location: Option<String>,
    /// Optional ISO 3166-1 alpha-2 country code (uppercased).
    pub country: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_callhistory;

    #[test]
    fn decode_address_reads_ascii_blob() {
        assert_eq!(decode_address(Some(b"+420776452878".to_vec())), "+420776452878");
        assert_eq!(decode_address(Some(b"a@b.cz\0".to_vec())), "a@b.cz");
        assert_eq!(decode_address(None), "");
    }

    #[test]
    fn classify_service_prefers_provider_then_call_type() {
        assert_eq!(classify_service(Some("com.apple.Telephony"), Some(1)), "phone");
        assert_eq!(classify_service(Some("com.apple.FaceTime"), Some(8)), "facetime");
        assert_eq!(classify_service(Some("net.whatsapp.WhatsApp"), Some(0)), "net.whatsapp.WhatsApp");
        assert_eq!(classify_service(None, Some(1)), "phone");
        assert_eq!(classify_service(None, Some(16)), "facetime");
        assert_eq!(classify_service(None, None), "unknown");
    }

    #[test]
    fn derive_video_maps_known_call_types() {
        assert_eq!(derive_video(Some(8)), Some(true));
        assert_eq!(derive_video(Some(16)), Some(false));
        assert_eq!(derive_video(Some(1)), None);
        assert_eq!(derive_video(None), None);
    }

    #[test]
    fn parses_calls_ordered_by_date() {
        let dir = std::env::temp_dir().join(format!("be-calls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CallHistory.storedata");
        let _ = std::fs::remove_file(&db);
        make_callhistory(&db);

        let calls = parse(&db).unwrap();
        assert_eq!(calls.len(), 2);

        // ORDER BY ZDATE ascending: the FaceTime row (cocoa 50) is first.
        let ft = &calls[0];
        assert_eq!(ft.number, "jana@example.cz");
        assert_eq!(ft.direction, "incoming");
        assert!(!ft.answered);
        assert_eq!(ft.service, "facetime");
        assert_eq!(ft.video, Some(true));
        assert_eq!(ft.call_type, Some(8));
        assert_eq!(ft.country, None);

        let phone = &calls[1];
        assert_eq!(phone.number, "+420776452878");
        assert_eq!(phone.date, "2001-01-01T00:01:40+00:00");
        assert_eq!(phone.duration_seconds, 42);
        assert_eq!(phone.direction, "outgoing");
        assert!(phone.answered);
        assert_eq!(phone.service, "phone");
        assert_eq!(phone.country, Some("CZ".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_when_optional_columns_absent() {
        let dir = std::env::temp_dir().join(format!("be-calls-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CallHistory.storedata");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZCALLRECORD (Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL, ZADDRESS BLOB, ZORIGINATED INTEGER, ZANSWERED INTEGER, ZCALLTYPE INTEGER);
             INSERT INTO ZCALLRECORD VALUES (1, 100.0, 10.0, CAST('+420' AS BLOB), 1, 1, 1);",
        )
        .unwrap();
        drop(conn);

        let calls = parse(&db).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].service, "phone");
        assert_eq!(calls[0].location, None);
        assert_eq!(calls[0].country, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 12: Run the parser tests to verify they fail**

Run: `cargo test -p backup-extractor calls`
Expected: FAIL — `decode_address` / `classify_service` / `derive_video` / `parse` not found.

- [ ] **Step 13: Implement the parser and helpers**

In `backup-extractor/src/calls.rs`, between the `Call` struct and the test module:

```rust
fn decode_address(bytes: Option<Vec<u8>>) -> String {
    match bytes {
        Some(b) => String::from_utf8_lossy(&b).trim_end_matches('\0').to_string(),
        None => String::new(),
    }
}

fn classify_service(provider: Option<&str>, call_type: Option<i64>) -> String {
    if let Some(p) = provider {
        if p == "com.apple.Telephony" {
            return "phone".to_string();
        }
        if p == "com.apple.FaceTime" {
            return "facetime".to_string();
        }
        if !p.is_empty() {
            return p.to_string();
        }
    }
    match call_type {
        Some(1) => "phone".to_string(),
        Some(8) | Some(16) => "facetime".to_string(),
        _ => "unknown".to_string(),
    }
}

fn derive_video(call_type: Option<i64>) -> Option<bool> {
    match call_type {
        Some(8) => Some(true),
        Some(16) => Some(false),
        _ => None,
    }
}

/// Parse every call record from `db_path` (opened read-only), tolerating
/// missing optional columns across iOS versions.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Call>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZCALLRECORD")?;
    let col = |name: &str| if cols.contains(name) { name } else { "NULL" };

    let sql = format!(
        "SELECT ZADDRESS, ZDATE, ZDURATION, ZORIGINATED, ZANSWERED, ZCALLTYPE, {}, {}, {} \
         FROM ZCALLRECORD ORDER BY ZDATE",
        col("ZSERVICE_PROVIDER"),
        col("ZLOCATION"),
        col("ZISO_COUNTRY_CODE"),
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let address: Option<Vec<u8>> = row.get(0)?;
        let date: Option<f64> = row.get(1)?;
        let duration: Option<f64> = row.get(2)?;
        let originated: Option<i64> = row.get(3)?;
        let answered: Option<i64> = row.get(4)?;
        let call_type: Option<i64> = row.get(5)?;
        let provider: Option<String> = row.get(6)?;
        let location: Option<String> = row.get(7)?;
        let country: Option<String> = row.get(8)?;
        Ok(Call {
            number: decode_address(address),
            date: date.and_then(cocoa_to_iso).unwrap_or_default(),
            duration_seconds: duration.unwrap_or(0.0).round() as i64,
            direction: if originated == Some(1) { "outgoing" } else { "incoming" }.to_string(),
            answered: answered == Some(1),
            service: classify_service(provider.as_deref(), call_type),
            video: derive_video(call_type),
            call_type,
            location: location.filter(|s| !s.is_empty()),
            country: country.filter(|s| !s.is_empty()).map(|s| s.to_uppercase()),
        })
    })?;
    rows.collect()
}
```

- [ ] **Step 14: Register the modules**

In `backup-extractor/src/main.rs`, with the other `mod` lines at the top:

```rust
mod calls;
mod datetime;
mod sqlite_util;
```

- [ ] **Step 15: Run the full crate tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all tests PASS; clippy reports no warnings.

- [ ] **Step 16: Commit**

```bash
git add backup-extractor/Cargo.toml backup-extractor/src/datetime.rs backup-extractor/src/sqlite_util.rs backup-extractor/src/calls.rs backup-extractor/src/test_fixtures.rs backup-extractor/src/main.rs
git commit -m "feat(calls): parse call history with shared datetime/column-detection helpers"
```

---

### Task 2: Render calls to csv/json/html

**Files:**
- Modify: `backup-extractor/src/format.rs`
- Create: `backup-extractor/templates/calls.html`

**Interfaces:**
- Consumes: `calls::Call` (Task 1).
- Produces: `format::calls_csv(&[calls::Call]) -> String`, `format::calls_json(&[calls::Call]) -> String`, `format::calls_html(&[calls::Call]) -> String`.

- [ ] **Step 1: Create the askama template**

Create `backup-extractor/templates/calls.html`:

```html
<!doctype html>
<html lang="cs"><head><meta charset="utf-8"><title>Hovory</title>
<style>body{font-family:-apple-system,Helvetica,Arial,sans-serif;margin:24px}
table{border-collapse:collapse}td,th{border:1px solid #ddd;padding:4px 8px;text-align:left}
th{background:#f5f5f5}</style></head><body>
<h1>Hovory ({{ calls.len() }})</h1>
<table>
<tr><th>Číslo</th><th>Datum</th><th>Trvání (s)</th><th>Směr</th><th>Přijatý</th><th>Služba</th><th>Video</th><th>Lokace</th><th>Země</th></tr>
{% for c in calls %}
<tr>
<td>{{ c.number }}</td><td>{{ c.date }}</td><td>{{ c.duration_seconds }}</td>
<td>{{ c.direction }}</td><td>{{ c.answered }}</td><td>{{ c.service }}</td>
<td>{% if let Some(v) = c.video %}{{ v }}{% endif %}</td>
<td>{% if let Some(l) = c.location.as_ref() %}{{ l }}{% endif %}</td>
<td>{% if let Some(cc) = c.country.as_ref() %}{{ cc }}{% endif %}</td>
</tr>
{% endfor %}
</table>
</body></html>
```

- [ ] **Step 2: Write the failing formatter tests**

In `backup-extractor/src/format.rs`, inside `#[cfg(test)] mod tests`, add a sample builder and three tests:

```rust
    fn sample_calls() -> Vec<crate::calls::Call> {
        vec![crate::calls::Call {
            number: "+420776452878".into(),
            date: "2026-06-20T14:33:05+00:00".into(),
            duration_seconds: 42,
            direction: "outgoing".into(),
            answered: true,
            service: "phone".into(),
            video: None,
            call_type: Some(1),
            location: None,
            country: Some("CZ".into()),
        }]
    }

    #[test]
    fn calls_csv_has_header_and_row() {
        let out = calls_csv(&sample_calls());
        assert!(out.starts_with(
            "number,date,duration_seconds,direction,answered,service,video,call_type,location,country"
        ));
        assert!(out.contains("+420776452878"));
        assert!(out.contains(",outgoing,true,phone,,1,,CZ"));
    }

    #[test]
    fn calls_json_roundtrips() {
        let out = calls_json(&sample_calls());
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["number"], "+420776452878");
        assert_eq!(back[0]["direction"], "outgoing");
        assert_eq!(back[0]["country"], "CZ");
    }

    #[test]
    fn calls_html_lists_calls() {
        let out = calls_html(&sample_calls());
        assert!(out.contains("<html"));
        assert!(out.contains("+420776452878"));
        assert!(out.contains("outgoing"));
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p backup-extractor calls_`
Expected: FAIL — `calls_csv` / `calls_json` / `calls_html` not found.

- [ ] **Step 4: Implement the formatters**

In `backup-extractor/src/format.rs`, after the contacts formatters, add:

```rust
pub fn calls_csv(calls: &[crate::calls::Call]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "number", "date", "duration_seconds", "direction", "answered", "service",
        "video", "call_type", "location", "country",
    ])
    .unwrap();
    for c in calls {
        wtr.write_record([
            c.number.clone(),
            c.date.clone(),
            c.duration_seconds.to_string(),
            c.direction.clone(),
            c.answered.to_string(),
            c.service.clone(),
            c.video.map(|v| v.to_string()).unwrap_or_default(),
            c.call_type.map(|v| v.to_string()).unwrap_or_default(),
            c.location.clone().unwrap_or_default(),
            c.country.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn calls_json(calls: &[crate::calls::Call]) -> String {
    serde_json::to_string_pretty(calls).unwrap()
}

#[derive(Template)]
#[template(path = "calls.html")]
struct CallsTemplate<'a> {
    calls: &'a [crate::calls::Call],
}

pub fn calls_html(calls: &[crate::calls::Call]) -> String {
    CallsTemplate { calls }.render().unwrap()
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS; no warnings.

- [ ] **Step 6: Commit**

```bash
git add backup-extractor/src/format.rs backup-extractor/templates/calls.html
git commit -m "feat(calls): render call history to csv/json/html"
```

---

### Task 3: Wire the `calls` command and generalize `inspect`

**Files:**
- Modify: `backup-extractor/src/main.rs`

**Interfaces:**
- Consumes: `calls::parse`, `calls::Call`, `format::calls_*` (Tasks 1-2); existing `Format`, `AppError`, `open_error`, `device_json`, `load_contacts`, `KNOWN_STORES`.
- Produces: `Command::Calls { format }`, `run_calls`, `load_calls`, `calls_format` (validator); generalized `inspect` counting.

- [ ] **Step 1: Write the failing CLI tests**

In `backup-extractor/src/main.rs`, inside `#[cfg(test)] mod cli_tests`, add:

```rust
    #[test]
    fn parses_calls_invocation() {
        let cli = Cli::try_parse_from([
            "backup-extractor", "--backup", "/b", "-o", "/out", "calls", "-f", "json",
        ])
        .unwrap();
        match cli.command {
            Command::Calls { format } => assert_eq!(format, "json"),
            _ => panic!("expected Calls"),
        }
    }

    #[test]
    fn calls_format_rejects_vcf_accepts_others() {
        assert_eq!(calls_format("csv").unwrap(), Format::Csv);
        assert_eq!(calls_format("json").unwrap(), Format::Json);
        assert_eq!(calls_format("html").unwrap(), Format::Html);
        assert_eq!(calls_format("vcf").unwrap_err().code, 1);
        assert_eq!(calls_format("nope").unwrap_err().code, 1);
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p backup-extractor cli_tests`
Expected: FAIL — `Command::Calls` and `calls_format` do not exist.

- [ ] **Step 3: Add the `Calls` subcommand**

In the `Command` enum in `backup-extractor/src/main.rs`, add after `Contacts { .. }`:

```rust
    /// Export call history.
    Calls {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
```

- [ ] **Step 4: Route the command**

In `run()`, extend the `match &cli.command` block:

```rust
        Command::Calls { format } => run_calls(&cli, password.as_deref(), format),
```

- [ ] **Step 5: Implement `calls_format`, `load_calls`, `run_calls`**

In `backup-extractor/src/main.rs` (near `load_contacts` / `run_contacts`):

```rust
/// Validate a `calls` format string: csv/json/html only (vcf is meaningless for
/// calls). Returns a usage `AppError` (exit 1) on a bad or unsupported format.
fn calls_format(format: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown calls format `{format}` (use csv, json, html)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for calls (use csv, json, html)"));
    }
    Ok(f)
}

/// Fetch and parse the call history into memory via a secure auto-cleaned temp
/// dir. `Ok(None)` when the backup has no call-history store.
fn load_calls(backup: &backup_core::Backup) -> Result<Option<Vec<calls::Call>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("CallHistory.storedata");
    let Some(db) = backup
        .fetch("HomeDomain", "Library/CallHistoryDB/CallHistory.storedata", &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(None);
    };
    let calls = calls::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(calls))
}

fn run_calls(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = calls_format(format)?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export calls"))?;

    let backup = backup_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(calls) = load_calls(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "calls", "count": 0, "outputs": [],
            "note": "this backup has no call history", "device": device
        }));
    };

    let rendered = match format {
        Format::Csv => format::calls_csv(&calls),
        Format::Json => format::calls_json(&calls),
        Format::Html => format::calls_html(&calls),
        Format::Vcf => unreachable!("calls_format rejects vcf"),
    };
    let out_file = out.join(format!("calls.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} call(s) to {}", calls.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "calls", "count": calls.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}
```

- [ ] **Step 6: Flip `calls` to supported and generalize the inspect count**

In `KNOWN_STORES`, change the `calls` row's second field to `true`:

```rust
    ("calls", true, "HomeDomain", "Library/CallHistoryDB/CallHistory.storedata"),
```

In `run_inspect`, replace the contacts-only count block with a dispatch:

```rust
        let count = if present && supported {
            match name {
                "contacts" => load_contacts(&backup)?.map(|p| p.len()),
                "calls" => load_calls(&backup)?.map(|c| c.len()),
                _ => None,
            }
        } else {
            None
        };
```

- [ ] **Step 7: Run tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS; no warnings.

- [ ] **Step 8: Commit**

```bash
git add backup-extractor/src/main.rs
git commit -m "feat(calls): add calls command and generalize inspect counts"
```

---

### Task 4: Parse voicemail metadata

**Files:**
- Create: `backup-extractor/src/voicemail.rs`
- Modify: `backup-extractor/src/test_fixtures.rs` (add `make_voicemail`)
- Modify: `backup-extractor/src/main.rs` (add `mod voicemail;`)

**Interfaces:**
- Consumes: `datetime::{unix_to_iso, cocoa_to_iso}`, `sqlite_util::table_columns` (Task 1).
- Produces: `voicemail::Voicemail` (struct below) and `voicemail::parse(db_path: &std::path::Path) -> rusqlite::Result<Vec<voicemail::Voicemail>>`; `test_fixtures::make_voicemail(path: &std::path::Path)`.

- [ ] **Step 1: Add the `make_voicemail` fixture**

In `backup-extractor/src/test_fixtures.rs`, append:

```rust
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
            (1, 1600000000, '+420776452878', 30, 0, 0, 0),
            (2, 1600000100, NULL, 12, 0, 600000000, 75);",
    )
    .unwrap();
}
```

- [ ] **Step 2: Write the failing parser tests**

Create `backup-extractor/src/voicemail.rs`:

```rust
//! Read voicemail metadata from an iOS `voicemail.db` (audio files excluded).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::{cocoa_to_iso, unix_to_iso};
use crate::sqlite_util::table_columns;

/// One voicemail record (metadata only).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Voicemail {
    /// Caller phone number; empty when withheld/unknown.
    pub sender: String,
    /// Receipt time as ISO 8601 (RFC 3339) UTC (Unix epoch); empty if unconvertible.
    pub date: String,
    /// Voicemail length in seconds.
    pub duration_seconds: i64,
    /// Whether the voicemail was moved to the Deleted folder.
    pub trashed: bool,
    /// When it was trashed (ISO 8601 UTC, Cocoa epoch); `None` when not trashed.
    pub trashed_at: Option<String>,
    /// Carrier expiry (ISO 8601 UTC, Unix epoch); `None` when unset/absent.
    pub expiration: Option<String>,
    /// Raw `flags` bitmask, preserved (bit meanings are undocumented).
    pub flags: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_voicemail;

    #[test]
    fn parses_voicemail_with_mixed_epochs() {
        let dir = std::env::temp_dir().join(format!("be-vm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("voicemail.db");
        let _ = std::fs::remove_file(&db);
        make_voicemail(&db);

        let vms = parse(&db).unwrap();
        assert_eq!(vms.len(), 2);

        let active = &vms[0];
        assert_eq!(active.sender, "+420776452878");
        assert_eq!(active.date, "2020-09-13T12:26:40+00:00"); // Unix 1_600_000_000
        assert_eq!(active.duration_seconds, 30);
        assert!(!active.trashed);
        assert_eq!(active.trashed_at, None);
        assert_eq!(active.expiration, None);

        let trashed = &vms[1];
        assert_eq!(trashed.sender, ""); // NULL → empty
        assert!(trashed.trashed);
        assert!(trashed.trashed_at.is_some()); // Cocoa 600_000_000 → some ISO
        assert_eq!(trashed.flags, 75);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_when_expiration_column_absent() {
        let dir = std::env::temp_dir().join(format!("be-vm-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("voicemail.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE voicemail (ROWID INTEGER PRIMARY KEY, date INTEGER, sender TEXT, duration INTEGER, trashed_date INTEGER, flags INTEGER);
             INSERT INTO voicemail (ROWID, date, sender, duration, trashed_date, flags) VALUES (1, 1600000000, '+1', 5, 0, 0);",
        )
        .unwrap();
        drop(conn);

        let vms = parse(&db).unwrap();
        assert_eq!(vms.len(), 1);
        assert_eq!(vms[0].expiration, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p backup-extractor voicemail`
Expected: FAIL — `parse` not found.

- [ ] **Step 4: Implement the parser**

In `backup-extractor/src/voicemail.rs`, between the struct and the test module:

```rust
/// Parse every voicemail record from `db_path` (opened read-only), tolerating a
/// missing optional `expiration` column. `date` is Unix epoch; `trashed_date`
/// is Cocoa 2001 epoch (the two columns intentionally use different epochs).
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Voicemail>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "voicemail")?;
    let expiration_sel = if cols.contains("expiration") { "expiration" } else { "NULL" };

    let sql = format!(
        "SELECT sender, date, duration, trashed_date, flags, {expiration_sel} \
         FROM voicemail ORDER BY date"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let sender: Option<String> = row.get(0)?;
        let date: Option<i64> = row.get(1)?;
        let duration: Option<i64> = row.get(2)?;
        let trashed_date: Option<i64> = row.get(3)?;
        let flags: Option<i64> = row.get(4)?;
        let expiration: Option<i64> = row.get(5)?;

        let trashed = trashed_date.unwrap_or(0) != 0;
        Ok(Voicemail {
            sender: sender.unwrap_or_default(),
            date: date.and_then(unix_to_iso).unwrap_or_default(),
            duration_seconds: duration.unwrap_or(0),
            trashed,
            trashed_at: if trashed {
                trashed_date.and_then(|t| cocoa_to_iso(t as f64))
            } else {
                None
            },
            expiration: expiration.filter(|&e| e != 0).and_then(unix_to_iso),
            flags: flags.unwrap_or(0),
        })
    })?;
    rows.collect()
}
```

- [ ] **Step 5: Register the module**

In `backup-extractor/src/main.rs`, with the other `mod` lines:

```rust
mod voicemail;
```

- [ ] **Step 6: Run tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS; no warnings.

- [ ] **Step 7: Commit**

```bash
git add backup-extractor/src/voicemail.rs backup-extractor/src/test_fixtures.rs backup-extractor/src/main.rs
git commit -m "feat(voicemail): parse voicemail metadata with mixed-epoch handling"
```

---

### Task 5: Render voicemail to csv/json/html

**Files:**
- Modify: `backup-extractor/src/format.rs`
- Create: `backup-extractor/templates/voicemail.html`

**Interfaces:**
- Consumes: `voicemail::Voicemail` (Task 4).
- Produces: `format::voicemail_csv`, `format::voicemail_json`, `format::voicemail_html` (each `&[voicemail::Voicemail] -> String`).

- [ ] **Step 1: Create the template**

Create `backup-extractor/templates/voicemail.html`:

```html
<!doctype html>
<html lang="cs"><head><meta charset="utf-8"><title>Hlasové zprávy</title>
<style>body{font-family:-apple-system,Helvetica,Arial,sans-serif;margin:24px}
table{border-collapse:collapse}td,th{border:1px solid #ddd;padding:4px 8px;text-align:left}
th{background:#f5f5f5}</style></head><body>
<h1>Hlasové zprávy ({{ voicemails.len() }})</h1>
<table>
<tr><th>Odesílatel</th><th>Datum</th><th>Trvání (s)</th><th>V koši</th><th>Smazáno</th><th>Expirace</th><th>Flags</th></tr>
{% for v in voicemails %}
<tr>
<td>{{ v.sender }}</td><td>{{ v.date }}</td><td>{{ v.duration_seconds }}</td>
<td>{{ v.trashed }}</td>
<td>{% if let Some(t) = v.trashed_at.as_ref() %}{{ t }}{% endif %}</td>
<td>{% if let Some(e) = v.expiration.as_ref() %}{{ e }}{% endif %}</td>
<td>{{ v.flags }}</td>
</tr>
{% endfor %}
</table>
</body></html>
```

- [ ] **Step 2: Write the failing formatter tests**

In `backup-extractor/src/format.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
    fn sample_voicemails() -> Vec<crate::voicemail::Voicemail> {
        vec![crate::voicemail::Voicemail {
            sender: "+420776452878".into(),
            date: "2020-09-13T12:26:40+00:00".into(),
            duration_seconds: 30,
            trashed: false,
            trashed_at: None,
            expiration: None,
            flags: 0,
        }]
    }

    #[test]
    fn voicemail_csv_has_header_and_row() {
        let out = voicemail_csv(&sample_voicemails());
        assert!(out.starts_with(
            "sender,date,duration_seconds,trashed,trashed_at,expiration,flags"
        ));
        assert!(out.contains("+420776452878,2020-09-13T12:26:40+00:00,30,false,,,0"));
    }

    #[test]
    fn voicemail_json_roundtrips() {
        let out = voicemail_json(&sample_voicemails());
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["sender"], "+420776452878");
        assert_eq!(back[0]["trashed"], false);
    }

    #[test]
    fn voicemail_html_lists_items() {
        let out = voicemail_html(&sample_voicemails());
        assert!(out.contains("<html"));
        assert!(out.contains("+420776452878"));
    }
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p backup-extractor voicemail_`
Expected: FAIL — `voicemail_csv` / `voicemail_json` / `voicemail_html` not found.

- [ ] **Step 4: Implement the formatters**

In `backup-extractor/src/format.rs`, after the calls formatters:

```rust
pub fn voicemail_csv(items: &[crate::voicemail::Voicemail]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "sender", "date", "duration_seconds", "trashed", "trashed_at", "expiration", "flags",
    ])
    .unwrap();
    for v in items {
        wtr.write_record([
            v.sender.clone(),
            v.date.clone(),
            v.duration_seconds.to_string(),
            v.trashed.to_string(),
            v.trashed_at.clone().unwrap_or_default(),
            v.expiration.clone().unwrap_or_default(),
            v.flags.to_string(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn voicemail_json(items: &[crate::voicemail::Voicemail]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "voicemail.html")]
struct VoicemailTemplate<'a> {
    voicemails: &'a [crate::voicemail::Voicemail],
}

pub fn voicemail_html(items: &[crate::voicemail::Voicemail]) -> String {
    VoicemailTemplate { voicemails: items }.render().unwrap()
}
```

- [ ] **Step 5: Run tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS; no warnings.

- [ ] **Step 6: Commit**

```bash
git add backup-extractor/src/format.rs backup-extractor/templates/voicemail.html
git commit -m "feat(voicemail): render voicemail to csv/json/html"
```

---

### Task 6: Wire the `voicemail` command and register the store

**Files:**
- Modify: `backup-extractor/src/main.rs`

**Interfaces:**
- Consumes: `voicemail::parse`, `voicemail::Voicemail`, `format::voicemail_*` (Tasks 4-5); existing `Format`, `AppError`, `open_error`, `device_json`, `KNOWN_STORES`, the inspect dispatch (Task 3).
- Produces: `Command::Voicemail { format }`, `run_voicemail`, `load_voicemail`, `voicemail_format`; `voicemail` row in `KNOWN_STORES`; voicemail arm in the inspect dispatch.

- [ ] **Step 1: Write the failing CLI tests**

In `backup-extractor/src/main.rs`, inside `#[cfg(test)] mod cli_tests`, add:

```rust
    #[test]
    fn parses_voicemail_invocation() {
        let cli = Cli::try_parse_from([
            "backup-extractor", "--backup", "/b", "-o", "/out", "voicemail", "-f", "csv",
        ])
        .unwrap();
        match cli.command {
            Command::Voicemail { format } => assert_eq!(format, "csv"),
            _ => panic!("expected Voicemail"),
        }
    }

    #[test]
    fn voicemail_format_rejects_vcf() {
        assert_eq!(voicemail_format("json").unwrap(), Format::Json);
        assert_eq!(voicemail_format("vcf").unwrap_err().code, 1);
    }

    #[test]
    fn known_stores_lists_voicemail_supported() {
        let vm = KNOWN_STORES.iter().find(|(n, ..)| *n == "voicemail").unwrap();
        assert!(vm.1, "voicemail must be supported");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p backup-extractor cli_tests`
Expected: FAIL — `Command::Voicemail` / `voicemail_format` not found; no voicemail in `KNOWN_STORES`.

- [ ] **Step 3: Add the `Voicemail` subcommand and route it**

In the `Command` enum, after `Calls { .. }`:

```rust
    /// Export voicemail metadata.
    Voicemail {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
```

In `run()`'s match:

```rust
        Command::Voicemail { format } => run_voicemail(&cli, password.as_deref(), format),
```

- [ ] **Step 4: Implement `voicemail_format`, `load_voicemail`, `run_voicemail`**

In `backup-extractor/src/main.rs`:

```rust
/// Validate a `voicemail` format string: csv/json/html only.
fn voicemail_format(format: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown voicemail format `{format}` (use csv, json, html)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for voicemail (use csv, json, html)"));
    }
    Ok(f)
}

/// Fetch and parse voicemail metadata into memory via a secure auto-cleaned temp
/// dir. `Ok(None)` when the backup has no voicemail store.
fn load_voicemail(backup: &backup_core::Backup) -> Result<Option<Vec<voicemail::Voicemail>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("voicemail.db");
    let Some(db) = backup
        .fetch("HomeDomain", "Library/Voicemail/voicemail.db", &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(None);
    };
    let items = voicemail::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(items))
}

fn run_voicemail(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = voicemail_format(format)?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export voicemail"))?;

    let backup = backup_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_voicemail(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "voicemail", "count": 0, "outputs": [],
            "note": "this backup has no voicemail", "device": device
        }));
    };

    let rendered = match format {
        Format::Csv => format::voicemail_csv(&items),
        Format::Json => format::voicemail_json(&items),
        Format::Html => format::voicemail_html(&items),
        Format::Vcf => unreachable!("voicemail_format rejects vcf"),
    };
    let out_file = out.join(format!("voicemail.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} voicemail(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "voicemail", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}
```

- [ ] **Step 5: Register the store and count it in `inspect`**

In `KNOWN_STORES`, add a row after `calls` (keep `photos`/`notes` last):

```rust
    ("voicemail", true, "HomeDomain", "Library/Voicemail/voicemail.db"),
```

In the `run_inspect` count dispatch, add the voicemail arm:

```rust
                "voicemail" => load_voicemail(&backup)?.map(|v| v.len()),
```

- [ ] **Step 6: Run tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS; no warnings.

- [ ] **Step 7: Commit**

```bash
git add backup-extractor/src/main.rs
git commit -m "feat(voicemail): add voicemail command and register the store"
```

---

### Task 7: Parse contact postal addresses

**Files:**
- Modify: `backup-extractor/src/contacts.rs`
- Modify: `backup-extractor/src/test_fixtures.rs` (extend `make_addressbook`)
- Modify: `backup-extractor/src/format.rs` (fix existing `Contact` literals in tests)

**Interfaces:**
- Consumes: existing `contacts::{Contact, Labeled, clean_label}`.
- Produces: `contacts::Address` (struct below) and a new `pub addresses: Vec<Address>` field on `contacts::Contact`; updated `contacts::parse`.

- [ ] **Step 1: Extend the fixture with address tables**

Replace the body of `make_addressbook` in `backup-extractor/src/test_fixtures.rs` with (note `ABMultiValue` now has a real `ROWID` distinct from `UID`, so the test exercises the `parent_id = UID` join — for the address row `ROWID=10` but `UID=3`):

```rust
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
```

- [ ] **Step 2: Write the failing address tests**

In `backup-extractor/src/contacts.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
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
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p backup-extractor contacts`
Expected: FAIL — no `addresses` field on `Contact` / `Address` not defined.

- [ ] **Step 4: Add the `Address` type and the `addresses` field**

In `backup-extractor/src/contacts.rs`, add the property code constant next to the others and the new struct after `Contact`:

```rust
const PROP_ADDRESS: i64 = 5;
```

```rust
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
```

Add the field to `Contact` (after `note`):

```rust
    pub addresses: Vec<Address>,
```

- [ ] **Step 5: Rewrite `parse` to also collect addresses**

Replace the existing `pub fn parse` body in `backup-extractor/src/contacts.rs` with:

```rust
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
```

- [ ] **Step 6: Fix the existing `Contact` literals in `format.rs` tests**

The new `addresses` field breaks the three `Contact { .. }` literals in `backup-extractor/src/format.rs` tests (`sample()`, `vcard_escapes_special_chars_and_resists_injection`, `vcard_fn_falls_back_to_organization`). Add `addresses: vec![],` as the last field to **each** of those three literals so the crate compiles.

- [ ] **Step 7: Run tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS (including the existing `parses_people_phones_and_emails`, which is unaffected); no warnings.

- [ ] **Step 8: Commit**

```bash
git add backup-extractor/src/contacts.rs backup-extractor/src/test_fixtures.rs backup-extractor/src/format.rs
git commit -m "feat(contacts): parse postal addresses (UID join, name-keyed components)"
```

---

### Task 8: Render contact addresses across all formats

**Files:**
- Modify: `backup-extractor/src/contacts.rs` (add `Address::display_line`)
- Modify: `backup-extractor/src/format.rs` (CSV column, vCard `ADR`)
- Modify: `backup-extractor/templates/contacts.html` (address block)

**Interfaces:**
- Consumes: `contacts::{Contact, Address}` (Task 7).
- Produces: `contacts::Address::display_line(&self) -> String`; updated `contacts_csv` / `contacts_vcard` / `contacts.html`.

- [ ] **Step 1: Write the failing rendering tests**

In `backup-extractor/src/format.rs`, inside `#[cfg(test)] mod tests`, add a sample with an address and three tests:

```rust
    fn sample_with_address() -> Vec<Contact> {
        vec![Contact {
            first: "Jan".into(),
            last: "Novák".into(),
            organization: String::new(),
            phones: vec![],
            emails: vec![],
            note: String::new(),
            addresses: vec![crate::contacts::Address {
                label: "Work".into(),
                street: "Hlavní 1".into(),
                city: "Praha".into(),
                state: String::new(),
                zip: "11000".into(),
                country: "Czechia".into(),
                country_code: String::new(),
            }],
        }]
    }

    #[test]
    fn contacts_csv_includes_addresses_column() {
        let out = contacts_csv(&sample_with_address());
        assert!(out.starts_with("first,last,organization,phones,emails,addresses,note"));
        assert!(out.contains("Work: Hlavní 1, Praha, 11000, Czechia"));
    }

    #[test]
    fn vcard_emits_escaped_adr() {
        let out = contacts_vcard(&sample_with_address());
        assert!(out.contains("ADR;TYPE=Work:;;Hlavní 1;Praha;;11000;Czechia"));
    }

    #[test]
    fn vcard_adr_resists_injection() {
        let contacts = vec![Contact {
            first: "X".into(),
            last: String::new(),
            organization: String::new(),
            phones: vec![],
            emails: vec![],
            note: String::new(),
            addresses: vec![crate::contacts::Address {
                label: "Home".into(),
                street: "A;B\nADR:evil".into(),
                city: String::new(),
                state: String::new(),
                zip: String::new(),
                country: String::new(),
                country_code: String::new(),
            }],
        }];
        let out = contacts_vcard(&contacts);
        assert!(out.contains("A\\;B\\nADR:evil"));
        assert!(!out.contains("\nADR:evil")); // the injected newline did not start a property
    }

    #[test]
    fn contacts_html_shows_address() {
        let out = contacts_html(&sample_with_address());
        assert!(out.contains("Hlavní 1"));
        assert!(out.contains("Work"));
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p backup-extractor contacts_csv_includes_addresses_column vcard_emits_escaped_adr vcard_adr_resists_injection contacts_html_shows_address`
Expected: FAIL — `display_line` missing, CSV lacks the column, no `ADR` line.

- [ ] **Step 3: Add `Address::display_line`**

In `backup-extractor/src/contacts.rs`, after the `Address` struct:

```rust
impl Address {
    /// One-line human rendering of the address parts (no label), omitting empty
    /// components. Used by the CSV column and the HTML view.
    pub fn display_line(&self) -> String {
        [&self.street, &self.city, &self.state, &self.zip, &self.country]
            .into_iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    }
}
```

- [ ] **Step 4: Add the CSV column and the vCard `ADR`**

In `backup-extractor/src/format.rs`, add a join helper near `join_labeled`:

```rust
fn join_addresses(addresses: &[crate::contacts::Address]) -> String {
    addresses
        .iter()
        .map(|a| {
            let line = a.display_line();
            if a.label.is_empty() { line } else { format!("{}: {}", a.label, line) }
        })
        .collect::<Vec<_>>()
        .join("; ")
}
```

Update `contacts_csv` to add the `addresses` column between `emails` and `note`:

```rust
    wtr.write_record(["first", "last", "organization", "phones", "emails", "addresses", "note"])
        .unwrap();
    for c in contacts {
        wtr.write_record([
            &c.first,
            &c.last,
            &c.organization,
            &join_labeled(&c.phones),
            &join_labeled(&c.emails),
            &join_addresses(&c.addresses),
            &c.note,
        ])
        .unwrap();
    }
```

In `contacts_vcard`, after the emails loop and before the note block, add:

```rust
        for a in &c.addresses {
            out.push_str(&format!(
                "ADR;TYPE={}:;;{};{};{};{};{}\r\n",
                vcard_param(&a.label),
                vcard_escape(&a.street),
                vcard_escape(&a.city),
                vcard_escape(&a.state),
                vcard_escape(&a.zip),
                vcard_escape(&a.country),
            ));
        }
```

- [ ] **Step 5: Add the address block to the HTML template**

In `backup-extractor/templates/contacts.html`, add a line after the emails loop (between the `c.emails` loop and the `c.note` block):

```html
  {% for a in &c.addresses %}<div class="v">🏠 {{ a.label }}: {{ a.display_line() }}</div>{% endfor %}
```

- [ ] **Step 6: Run tests and clippy**

Run: `cargo test -p backup-extractor` then `cargo clippy --all-targets -p backup-extractor`
Expected: all PASS; no warnings.

- [ ] **Step 7: Commit**

```bash
git add backup-extractor/src/contacts.rs backup-extractor/src/format.rs backup-extractor/templates/contacts.html
git commit -m "feat(contacts): render postal addresses in csv/json/vcard/html"
```

---

### Task 9: Document calls, voicemail, and contact addresses in `AGENTS.md`

**Files:**
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: the built binary's actual output (verify against it).
- Produces: agent-facing documentation for the new commands and field.

- [ ] **Step 1: Build the binary and capture real output for verification**

Run: `cargo build -p backup-extractor` and confirm the binary exists at `target/debug/backup-extractor`. The exact JSON shapes documented below must match what `run_calls` / `run_voicemail` emit (envelope `{ ok, command, count, outputs, device }`); cross-check the field tables in `calls.rs` / `voicemail.rs`.

- [ ] **Step 2: Update the `inspect` stores example**

In `AGENTS.md`, replace the `stores` array in the `inspect` example so `calls` and `voicemail` are supported and present, keeping `photos`/`notes` unsupported:

```json
  "stores": [
    { "type": "contacts", "present": true, "supported": true, "count": 1234 },
    { "type": "calls", "present": true, "supported": true, "count": 5678 },
    { "type": "voicemail", "present": true, "supported": true, "count": 42 },
    { "type": "photos", "present": true, "supported": false, "count": null },
    { "type": "notes", "present": false, "supported": false, "count": null }
  ]
```

- [ ] **Step 3: Extend the `contacts` section with addresses**

After the `contacts` envelope example in `AGENTS.md`, add:

```markdown
Each contact also carries an `addresses` array (postal addresses): objects with
`label`, `street`, `city`, `state`, `zip`, `country`, `country_code` (empty
strings for absent parts). In vCard output these become `ADR` properties; in CSV
they are joined into one `addresses` column.
```

- [ ] **Step 4: Add the `calls` command section**

In `AGENTS.md`, after the `contacts` section, add:

````markdown
### `calls` — export call history

```
backup-extractor --backup <DIR> [--password <PW>] -o <OUT> calls -f <FORMAT>
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/calls.<ext>`. stdout envelope:

```json
{
  "ok": true,
  "command": "calls",
  "count": 5678,
  "outputs": ["<OUT>/calls.json"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

Each call object: `number` (phone number, or an Apple ID/email for FaceTime),
`date` (ISO 8601 UTC), `duration_seconds`, `direction` (`incoming`/`outgoing`),
`answered` (bool), `service` (`phone`/`facetime`/raw bundle id/`unknown`),
`video` (best-effort FaceTime video flag — **version-dependent and undocumented**,
may be `null`), `call_type` (raw `ZCALLTYPE` integer, the honest backing for
`video`), `location`, `country`. No call history → `count: 0`, `outputs: []`,
plus a `note`.
````

- [ ] **Step 5: Add the `voicemail` command section**

After the `calls` section, add:

````markdown
### `voicemail` — export voicemail metadata

```
backup-extractor --backup <DIR> [--password <PW>] -o <OUT> voicemail -f <FORMAT>
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/voicemail.<ext>`. Audio files (`.amr`) are not extracted. stdout envelope:

```json
{
  "ok": true,
  "command": "voicemail",
  "count": 42,
  "outputs": ["<OUT>/voicemail.json"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

Each voicemail object: `sender` (caller number; empty when withheld), `date`
(ISO 8601 UTC), `duration_seconds`, `trashed` (bool — moved to Deleted),
`trashed_at` (ISO 8601 UTC or `null`), `expiration` (ISO 8601 UTC or `null`),
`flags` (raw bitmask, **not decoded** — use `trashed` for deletion status). No
voicemail → `count: 0`, `outputs: []`, plus a `note`.
````

- [ ] **Step 6: Verify the documented shapes against the binary**

If a test backup is available, run `inspect`, `calls -f json`, and `voicemail -f json` and confirm the stdout matches the documented envelopes (field names, `ok`, `command`, `count`, `outputs`, `device`). If no backup is available, re-read `run_calls` / `run_voicemail` / `run_inspect` and confirm the `serde_json::json!` literals match the documentation exactly.

- [ ] **Step 7: Commit**

```bash
git add AGENTS.md
git commit -m "docs: document calls, voicemail, and contact addresses in AGENTS.md"
```

---

## Plan Self-Review

**Spec coverage:** datetime util (T1) ✓; calls parse/format/CLI (T1-T3) ✓; voicemail parse/format/CLI (T4-T6) ✓; contact addresses parse/format (T7-T8) ✓; inspect generalization + KNOWN_STORES voicemail row (T3, T6) ✓; defensive column detection (T1 `sqlite_util`, used in T1/T4) ✓; honest-uncertainty fields (`video`/`call_type` T1, `flags` T4, documented T9) ✓; agent contract / AGENTS.md (T9) ✓; synthetic fixtures + determinism + ordering ✓. Deferred items (audio, group participants, map/receiver, photos/notes/pdf) are intentionally absent.

**Type consistency:** `Call`/`Voicemail`/`Address` field names match between parser, formatter samples, and templates. `datetime::{unix_to_iso, cocoa_to_iso}` signatures match their call sites. `Format` enum is reused; `calls_format`/`voicemail_format` reject `Format::Vcf`. The `Contact` struct change (T7) is reconciled in `format.rs` test literals (T7 Step 6) before any address rendering (T8).

**Placeholder scan:** every code/test step contains concrete code and exact commands; no TBD/TODO/"similar to" placeholders.

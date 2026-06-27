# Voice Memos Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `archive voice-memos` — extract Voice Memos audio + metadata from an iOS backup.

**Architecture:** Mirror the existing `voicemail` + `voicemail_audio` extractors. Add one reusable `archive-core` primitive (`Backup::list`) for directory enumeration, promote the shared audio helpers into `src/audio.rs`, then add a parser, extraction, formatters, CLI command, and docs.

**Tech Stack:** Rust (edition 2024), rusqlite (read-only SQLite), serde, csv, askama (HTML templates), tempfile, ffmpeg (optional, transcode only).

## Global Constraints

- Agent-first contract: exactly one JSON object on stdout; progress on stderr; exit `0` ok / `1` usage|other / `2` auth; clap errors are the only non-JSON exit-2 case.
- Password: `--password` or `ARCHIVE_PASSWORD`; never prompt.
- No new mandatory dependency: ffmpeg only for non-raw `--audio-format`; raw copy + metadata stay dependency-free.
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
- Spec: `docs/superpowers/specs/2026-06-27-voice-memos-extraction-design.md` — exact domain paths, column names, filename scheme, envelope shape live there; use them verbatim.
- Reference implementations to mirror: `archive/src/voicemail.rs`, `archive/src/voicemail_audio.rs`, `archive/src/format.rs`, `archive/src/main.rs`, `archive/templates/voicemail.html`, `archive/src/test_fixtures.rs`.

---

### Task 1: `archive-core::Backup::list` directory primitive

**Files:**
- Modify: `archive-core/src/lib.rs` (add `list` method after `has`)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub fn list(&self, domain: &str, prefix: &str) -> Result<Vec<String>, BackupError>` — sorted `relative_path`s in `domain` starting with `prefix`.

- [ ] **Step 1: Implement `list`** (after `has`, before `fetch`):

```rust
    /// Relative paths of every backup entry in `domain` whose `relative_path`
    /// starts with `prefix` (empty `prefix` lists the whole domain). Sorted for
    /// deterministic output. Read-only; decrypts nothing (manifest scan only).
    pub fn list(&self, domain: &str, prefix: &str) -> Result<Vec<String>, BackupError> {
        let entries = self
            .raw
            .entries()
            .map_err(|why| BackupError::Open(why.to_string()))?;
        let mut paths: Vec<String> = entries
            .iter()
            .filter(|e| e.domain == domain && e.relative_path.starts_with(prefix))
            .map(|e| e.relative_path.clone())
            .collect();
        paths.sort();
        Ok(paths)
    }
```

- [ ] **Step 2: Gated integration test** (real backup; skips when env unset — matches the `fetch` tests):

```rust
    #[test]
    fn list_filters_by_domain_and_prefix() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = Backup::open(std::path::Path::new(&dir), pw.as_deref()).unwrap();
        let got = backup.list("HomeDomain", "Library/").unwrap();
        assert!(got.iter().all(|p| p.starts_with("Library/")), "all under prefix");
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted, "list returns sorted paths");
    }
```

- [ ] **Step 3:** `cargo test -p archive-core` → passes (new test skips without a backup). `cargo clippy -p archive-core --all-targets` clean.
- [ ] **Step 4: Commit** — `feat(archive-core): add Backup::list directory primitive`.

---

### Task 2: Promote shared audio helpers into `src/audio.rs`

**Goal:** Extract the format/ffmpeg/filename helpers out of `voicemail_audio.rs` so `voice_memos` can reuse them. **No behavior change** — existing voicemail tests must stay green.

**Files:**
- Create: `archive/src/audio.rs`
- Modify: `archive/src/voicemail_audio.rs` (re-import from `crate::audio`), `archive/src/main.rs` (`mod audio;`)

**Interfaces:**
- Produces (in `crate::audio`): `enum AudioFormat { Amr, M4a, Wav }` (+ `from_cli`, `extension`, `needs_ffmpeg`); `fn compact_date(iso: &str) -> String`; `fn sanitize_component(s: &str) -> String` (the old `sanitize_sender`, renamed — keeps `[A-Za-z0-9+]`, empty → `"unknown"`); `fn ffmpeg_available() -> bool`; `fn transcode_args(input, output, format) -> Option<Vec<OsString>>`; `fn run_transcode(raw, dest, format) -> bool`.

- [ ] **Step 1:** Move `AudioFormat` (and its impls), `compact_date`, `sanitize_sender`→`sanitize_component`, `ffmpeg_available`, `transcode_args`, `run_transcode` from `voicemail_audio.rs` into new `archive/src/audio.rs` (with their unit tests). Make them `pub`.
- [ ] **Step 2:** In `voicemail_audio.rs`: delete the moved items; add `use crate::audio::{AudioFormat, compact_date, sanitize_component, run_transcode};`. `audio_filename` stays in `voicemail_audio.rs` but calls `crate::audio::compact_date` / `sanitize_component`. Update `extract_audio`'s `run_transcode` call to `crate::audio::run_transcode`.
- [ ] **Step 3:** `mod audio;` in `main.rs` (before `mod voicemail_audio;`).
- [ ] **Step 4:** `cargo test -p archive` → all existing voicemail tests still pass (unchanged behavior). `cargo clippy -p archive --all-targets` clean.
- [ ] **Step 5: Commit** — `refactor(archive): extract shared audio helpers into audio module`.

**Note for implementer:** this is a pure move/rename refactor. If a `pub use` re-export in `voicemail_audio` keeps the diff smaller while leaving the public surface identical, that is acceptable; the goal is zero behavior change and shared ownership of the helpers.

---

### Task 3: `voice_memos.rs` parser + fixture

**Files:**
- Create: `archive/src/voice_memos.rs` (struct + `parse`)
- Modify: `archive/src/test_fixtures.rs` (add `make_voicememos`)
- Modify: `archive/src/main.rs` (`mod voice_memos;`)

**Interfaces:**
- Produces: `pub struct VoiceMemo { pub title: String, pub date: String, pub duration_seconds: i64, pub source_file: String, pub audio_file: Option<String> }` (derives `Debug, Clone, PartialEq, Serialize`); `pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<VoiceMemo>>`.
- Consumes: `crate::datetime::cocoa_to_iso`, `crate::sqlite_util::table_columns`.

- [ ] **Step 1: Fixture** `make_voicememos(path)` in `test_fixtures.rs` — minimal `ZCLOUDRECORDING`, two rows (one empty title), Cocoa `ZDATE`:

```rust
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
```

- [ ] **Step 2: Write failing parser test** in `voice_memos.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_voicememos;

    #[test]
    fn parses_two_memos_with_cocoa_dates() {
        let dir = std::env::temp_dir().join(format!("be-vm-memos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CloudRecordings.db");
        let _ = std::fs::remove_file(&db);
        make_voicememos(&db);

        let memos = parse(&db).unwrap();
        assert_eq!(memos.len(), 2);
        assert_eq!(memos[0].title, "Schůzka");
        assert_eq!(memos[0].date, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000 + 978_307_200
        assert_eq!(memos[0].duration_seconds, 13); // 12.5 rounded
        assert_eq!(memos[0].source_file, "20200101 120000.m4a");
        assert_eq!(memos[0].audio_file, None);
        assert_eq!(memos[1].title, ""); // NULL label → empty
        assert_eq!(memos[1].source_file, "A1B2C3.m4a");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: Run** `cargo test -p archive voice_memos` → FAIL (no `parse`).
- [ ] **Step 4: Implement `parse`** (schema-tolerant; mirror `voicemail::parse`):

```rust
//! Read Voice Memos metadata from an iOS `CloudRecordings.db`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One Voice Memo record (audio path filled in by the extraction step).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceMemo {
    /// User-visible title (`ZCUSTOMLABEL`); empty when unnamed.
    pub title: String,
    /// Recording start as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub date: String,
    /// Length in seconds (rounded).
    pub duration_seconds: i64,
    /// `ZPATH` basename, e.g. `A1B2C3.m4a`; joins audio ↔ metadata. Empty if unknown.
    pub source_file: String,
    /// Output-relative path to the extracted audio (`voice_memos/<name>`);
    /// `None` until extraction runs or when the file is absent from the backup.
    pub audio_file: Option<String>,
}

pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<VoiceMemo>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZCLOUDRECORDING")?;
    let label_sel = if cols.contains("ZCUSTOMLABEL") {
        "ZCUSTOMLABEL"
    } else if cols.contains("ZENCRYPTEDTITLE") {
        "ZENCRYPTEDTITLE"
    } else {
        "NULL"
    };
    let path_sel = if cols.contains("ZPATH") { "ZPATH" } else { "NULL" };

    let sql = format!(
        "SELECT {label_sel}, ZDATE, ZDURATION, {path_sel} \
         FROM ZCLOUDRECORDING ORDER BY ZDATE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let title: Option<String> = row.get(0)?;
        let date: Option<f64> = row.get(1)?;
        let duration: Option<f64> = row.get(2)?;
        let path: Option<String> = row.get(3)?;
        Ok(VoiceMemo {
            title: title.unwrap_or_default(),
            date: date.and_then(cocoa_to_iso).unwrap_or_default(),
            duration_seconds: duration.unwrap_or(0.0).round() as i64,
            source_file: path.map(|p| basename(&p)).unwrap_or_default(),
            audio_file: None,
        })
    })?;
    rows.collect()
}

/// Last path component of a (possibly `/`-containing) `ZPATH`.
fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}
```

- [ ] **Step 5: Run** `cargo test -p archive voice_memos` → PASS. Add a `basename` unit test (`"a/b/c.m4a"` → `"c.m4a"`, `"x.m4a"` → `"x.m4a"`).
- [ ] **Step 6: Commit** — `feat(archive): parse Voice Memos metadata from CloudRecordings.db`.

---

### Task 4: Voice Memos formatters + HTML template

**Files:**
- Modify: `archive/src/format.rs` (add `voice_memos_csv/json/html`)
- Create: `archive/templates/voice-memos.html`
- Test: `format.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::voice_memos::VoiceMemo`.
- Produces: `pub fn voice_memos_csv/json/html(items: &[VoiceMemo]) -> String`.

- [ ] **Step 1: CSV** — columns `title, date, duration_seconds, source_file, audio_file` (mirror `voicemail_csv`).
- [ ] **Step 2: JSON** — `serde_json::to_string_pretty`.
- [ ] **Step 3: HTML** — askama template + `VoiceMemosTemplate`. Template `archive/templates/voice-memos.html` mirrors `voicemail.html`: a table; the Audio cell is `{% if let Some(a) = v.audio_file.as_ref() %}<audio controls src="{{ a }}"></audio>{% endif %}`. Columns: Title, Date, Duration, Audio.
- [ ] **Step 4: Tests** (mirror voicemail formatter tests):
  - `voice_memos_csv_has_header_and_row`
  - `voice_memos_json_roundtrips`
  - `voice_memos_html_renders_audio_player_when_present`
  - `voice_memos_html_escapes_audio_file_attribute` (crafted `"><script>` → asserts `&#60;script&#62;` present, raw `"><script>` absent).
- [ ] **Step 5: Run** `cargo test -p archive format` → PASS.
- [ ] **Step 6: Commit** — `feat(archive): render Voice Memos to csv/json/html`.

---

### Task 5: Voice Memos extraction (`extract_voice_memos`)

**Files:**
- Modify: `archive/src/voice_memos.rs` (add extraction below the parser)
- Test: same file

**Interfaces:**
- Consumes: `archive_core::Backup::{list, fetch}`, `crate::audio::{AudioFormat, compact_date, sanitize_component, ffmpeg_available, run_transcode}`.
- Produces: `pub struct VmSummary { pub format: String, pub dir: String, pub extracted: usize, pub missing: usize }`; `pub fn extract_voice_memos(backup: &archive_core::Backup, items: &mut Vec<VoiceMemo>, out: &Path, format: Option<AudioFormat>) -> std::io::Result<VmSummary>`.

Constants: `const VM_DIR: &str = "voice_memos";` `const DOMAINS: [&str; 2] = ["AppDomainGroup-group.com.apple.VoiceMemos", "MediaDomain"];` `const AUDIO_EXTS: [&str; 7] = ["m4a","caf","wav","aifc","aiff","mp4","m4r"];`

**Algorithm (per spec §Extraction):**
1. Find the first domain in `DOMAINS` whose `list(domain, "Recordings/")` returns any entry with an audio extension → that's the active domain + file list. If none, `extracted=0, missing=items.len()` (records kept, no dir created) — but still create no directory.
2. Output filename: `format!("{}_{}_{}", compact_date(&date), sanitize_component(&title), n)` + `.<ext>`, `n` a 1-based running counter.
3. For each audio path: match a record by `source_file == basename(path)`; else synthesize `VoiceMemo { title:"", date:"", duration_seconds:0, source_file: basename, audio_file: None }` and push to `items`.
4. Raw mode (`format == None`): `fetch(domain, path, <out>/voice_memos/<name>.<nativeExt>)`; set `audio_file`, `extracted++`. Native ext = the path's own extension.
5. Transcode mode (`Some(fmt)`): fetch to scratch temp, `run_transcode`; on success write `.<fmtExt>`; on failure keep raw native copy (best-effort, mirror `voicemail_audio::extract_audio`).
6. Records whose `source_file` matched nothing on disk → `missing++`.
7. `format` field of summary = `fmt.extension()` or `"raw"`.

- [ ] **Step 1: Write the gated end-to-end test** (mirror `voicemail_audio` real-backup test): asserts `extracted + missing == items.len()` and every linked file exists non-empty; skips without `ARCHIVE_TEST_BACKUP`.
- [ ] **Step 2: Write a pure-logic unit test for the output-name builder** — extract the name building into a small `fn output_name(date, title, n, ext) -> String` and unit-test it (`"2020-01-06T10:40:00+00:00","Schůzka",1,"m4a"` → `"2020-01-06_104000_Schůzka_1.m4a"`; empty title/date → `"unknown_unknown_2.m4a"`).
- [ ] **Step 3: Implement** `output_name`, `VmSummary`, `extract_voice_memos` per the algorithm (reuse `crate::audio` helpers and the best-effort/temp-dir flow from `voicemail_audio::extract_audio`).
- [ ] **Step 4: Run** `cargo test -p archive voice_memos` → unit test PASS (gated test skips). `cargo clippy -p archive --all-targets` clean.
- [ ] **Step 5: Commit** — `feat(archive): extract Voice Memos audio with optional transcode`.

---

### Task 6: CLI wiring (`voice-memos` command) + inspect

**Files:**
- Modify: `archive/src/main.rs`

**Interfaces:**
- Consumes: `voice_memos::{parse, extract_voice_memos, VoiceMemo, VmSummary}`, `format::voice_memos_*`, `audio::AudioFormat`.

- [ ] **Step 1: Add the subcommand** to `enum Command`:

```rust
    /// Export Voice Memos metadata and audio (audio on by default; --no-audio to skip).
    VoiceMemos {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip audio extraction (metadata only).
        #[arg(long)]
        no_audio: bool,
        /// Transcode audio to this format (m4a or wav, needs ffmpeg). Default: raw native copy.
        #[arg(long)]
        audio_format: Option<String>,
    },
```

Wire it in `run()`'s match → `run_voice_memos(&cli, password.as_deref(), format, *no_audio, audio_format.as_deref())`.

- [ ] **Step 2:** `voice_memos_format(format)` validator (csv/json/html, reject vcf) — mirror `voicemail_format`.
- [ ] **Step 3:** `load_voice_memos(backup) -> Result<Option<Vec<VoiceMemo>>, AppError>` — fetch `CloudRecordings.db` from the group domain (try `MediaDomain` `Recordings/Recordings.db` as fallback) into a temp dir, parse. `Ok(None)` when neither store present.
- [ ] **Step 4:** `resolve_vm_audio(no_audio, audio_format) -> Result<Option<Option<AudioFormat>>, AppError>` where outer `None` = `--no-audio` (skip extraction), `Some(inner)` = extract with `inner` (`None` raw / `Some(fmt)` transcode). Rules: `--audio-format` with `--no-audio` → usage error; unknown format → usage error; `amr` → usage error (`"voice-memos supports --audio-format m4a or wav"`); non-raw + no ffmpeg → fail fast (exit 1). Unit-test the usage-error branches (mirror `resolve_audio_format` tests).
- [ ] **Step 5:** `run_voice_memos` — mirror `run_voicemail`: validate format, resolve audio, require `--out`, open backup, `load_voice_memos`; store-absent → `count:0` note envelope; extract when not `--no-audio`; render to `<out>/voice-memos.<ext>`; build envelope with `count`, `outputs`, `device`, and an `audio` object (`format`, `dir`, `extracted`, `missing`) when extraction ran.
- [ ] **Step 6:** Add to `KNOWN_STORES`: `("voice-memos", true, "AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db")`. In `run_inspect`'s count match, add `"voice-memos" => load_voice_memos(&backup).ok().flatten().map(|v| v.len())`.
- [ ] **Step 7: CLI tests** (mirror existing): `parses_voice_memos_invocation`; `voice_memos_format_rejects_vcf`; `resolve_vm_audio` usage-error cases (audio-format with no-audio; amr; unknown).
- [ ] **Step 8: Run** `cargo test -p archive` → all pass. `cargo clippy -p archive --all-targets` clean.
- [ ] **Step 9: Commit** — `feat(archive): add voice-memos command and inspect entry`.

---

### Task 7: Docs

**Files:**
- Modify: `AGENTS.md` (new `voice-memos` command section; note the audio-on-by-default divergence from voicemail; update the `inspect` example's store list to include voice-memos)
- Modify: `archive/README.md` (quick-start line + Status checklist: move voice memos to done)

- [ ] **Step 1:** AGENTS.md — add a `### voice-memos` section mirroring the `voicemail` one: invocation, `--no-audio`/`--audio-format m4a|wav`, the `audio` envelope, output `voice-memos.<ext>` + `voice_memos/` dir, per-record fields (`title`, `date`, `duration_seconds`, `source_file`, `audio_file`). State that audio is **on by default** (unlike voicemail) and why.
- [ ] **Step 2:** `archive/README.md` — add quick-start example; update Status: `- [x] voice-memos — csv, json, html (metadata) + audio extraction (native copy or ffmpeg m4a/wav)`.
- [ ] **Step 3: Commit** — `docs: document voice-memos command`.

---

## Self-Review

- **Spec coverage:** list primitive (T1), shared audio (T2), parser (T3), formatters (T4), extraction incl. synthesize-from-disk + fallback domain + transcode best-effort (T5), CLI + inspect + envelope + fail-fast (T6), docs incl. default-on divergence (T7). All spec sections mapped.
- **Type consistency:** `VoiceMemo` fields identical across T3/T4/T5/T6; `VmSummary` fields (`format,dir,extracted,missing`) match the envelope built in T6; `extract_voice_memos` signature consumes `Option<AudioFormat>` and the CLI's `resolve_vm_audio` produces exactly that inner option.
- **No placeholders:** novel logic (columns, domains, filename scheme, epoch) is spelled out; boilerplate explicitly mirrors named reference files.
```

# Voicemail Audio Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add optional voicemail audio extraction (raw `.amr`, or transcoded `.m4a`/`.wav`) to the `archive voicemail` command, linked from each metadata record.

**Architecture:** The `voicemail` parser starts carrying each row's `ROWID`. A new `voicemail_audio` module maps `ROWID → HomeDomain/Library/Voicemail/<rowid>.amr`, fetches each file via the existing `archive_core::Backup::fetch`, optionally transcodes it with `ffmpeg`, writes it into `<out>/voicemail_audio/`, and fills each record's `audio_file`. The CLI gains `--audio` / `--audio-format`; formatters surface `audio_file`.

**Tech Stack:** Rust (edition 2024), `archive` bin crate, `archive-core` (crabapple backup access), `rusqlite`, `serde`/`serde_json`, `csv`, `askama` 0.16, `tempfile`; `ffmpeg` shelled out only for `m4a`/`wav`.

## Global Constraints

- **Agent-first contract:** exactly one JSON object on stdout; human progress on stderr; exit `0` success / `1` handled error / `2` auth; clap argument errors are the only non-JSON exit-2 case.
- **Password:** `--password` flag or `ARCHIVE_PASSWORD` env; never prompts.
- **No new mandatory dependency:** `ffmpeg` is required **only** when `--audio-format` is `m4a` or `wav`; the default `amr` path and all metadata paths stay dependency-free.
- **Crate independence:** `archive` must not depend on the `imessage-*` crates (build its own ffmpeg discovery/commands).
- **Naming:** audio filenames are `<date>_<sender>_<rowid>.<ext>`, where `date` = `YYYY-MM-DD_HHMMSS` (or `unknown`), `sender` keeps `[A-Za-z0-9+]` and maps other chars to `_` (or `unknown` when empty).
- **Audio subdir:** `<out>/voicemail_audio/`.
- **Best-effort:** a row with no audio in the backup → `audio_file: null`, counted `missing`, continue; a per-file transcode failure → keep the raw `.amr` and link it.
- **Fail-fast:** `m4a`/`wav` requested while ffmpeg is absent → exit 1 before any extraction; `--audio-format` without `--audio` → usage error (exit 1).
- **Licensing/process:** GPL-3.0-or-later; conventional commits; never squash.

---

## File Structure

- `archive/src/voicemail.rs` (modify) — `Voicemail` gains `rowid` + `audio_file`; parser selects `ROWID`.
- `archive/src/voicemail_audio.rs` (create) — `AudioFormat`, filename builder, ffmpeg args/discovery, `extract_audio`, `AudioSummary`.
- `archive/src/main.rs` (modify) — `mod voicemail_audio;`, `--audio`/`--audio-format` flags, `resolve_audio_format`, `run_voicemail` wiring + envelope.
- `archive/src/format.rs` (modify) — `voicemail_csv` adds `audio_file` column; test sample updated.
- `archive/templates/voicemail.html` (modify) — `Audio` column with `<audio controls>` player.
- `AGENTS.md`, `archive/README.md` (modify) — document the flags, envelope, ffmpeg note.

---

### Task 1: Parser carries `rowid` and `audio_file`

**Files:**
- Modify: `archive/src/voicemail.rs`
- Modify: `archive/src/format.rs` (the `sample_voicemails()` test literal must add the new fields to keep compiling)
- Test: `archive/src/voicemail.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `voicemail::Voicemail` now has `pub rowid: i64` (first field) and `pub audio_file: Option<String>` (last field). `parse(&Path) -> rusqlite::Result<Vec<Voicemail>>` populates `rowid` from `ROWID` and always sets `audio_file: None`.

- [ ] **Step 1: Extend the struct and the SELECT.** In `archive/src/voicemail.rs`, replace the struct and `parse` body.

Struct (add `rowid` first, `audio_file` last):

```rust
/// One voicemail record (metadata; audio path filled in separately).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Voicemail {
    /// Primary key in `voicemail.db`; a stable per-backup identifier and the
    /// base name of the audio file (`Library/Voicemail/<rowid>.amr`).
    pub rowid: i64,
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
    /// Output-relative path to the extracted audio (e.g.
    /// `voicemail_audio/2020-09-13_122640_+420…_3.m4a`); `None` until audio
    /// extraction runs, or when the backup has no audio for this row.
    pub audio_file: Option<String>,
}
```

`parse` — add `ROWID` as the first selected column and shift the row indices:

```rust
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Voicemail>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "voicemail")?;
    let expiration_sel = if cols.contains("expiration") { "expiration" } else { "NULL" };

    let sql = format!(
        "SELECT ROWID, sender, date, duration, trashed_date, flags, {expiration_sel} \
         FROM voicemail ORDER BY date"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let rowid: i64 = row.get(0)?;
        let sender: Option<String> = row.get(1)?;
        let date: Option<i64> = row.get(2)?;
        let duration: Option<i64> = row.get(3)?;
        let trashed_date: Option<i64> = row.get(4)?;
        let flags: Option<i64> = row.get(5)?;
        let expiration: Option<i64> = row.get(6)?;

        let trashed = trashed_date.unwrap_or(0) != 0;
        Ok(Voicemail {
            rowid,
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
            audio_file: None,
        })
    })?;
    rows.collect()
}
```

- [ ] **Step 2: Keep `format.rs` compiling.** In `archive/src/format.rs`, update the `sample_voicemails()` literal (in `#[cfg(test)] mod tests`) to add `rowid: 3,` as the first field and `audio_file: None,` as the last field:

```rust
fn sample_voicemails() -> Vec<crate::voicemail::Voicemail> {
    vec![crate::voicemail::Voicemail {
        rowid: 3,
        sender: "+420776452878".into(),
        date: "2020-09-13T12:26:40+00:00".into(),
        duration_seconds: 30,
        trashed: false,
        trashed_at: None,
        expiration: None,
        flags: 0,
        audio_file: None,
    }]
}
```

- [ ] **Step 3: Extend the parser tests.** In `archive/src/voicemail.rs` `mod tests`, add assertions to `parses_voicemail_with_mixed_epochs` (right after `assert_eq!(vms.len(), 2);` and within the existing `active`/`trashed` blocks):

```rust
        assert_eq!(active.rowid, 1);
        assert_eq!(active.audio_file, None);
```
```rust
        assert_eq!(trashed.rowid, 2);
        assert_eq!(trashed.audio_file, None);
```

And in `parses_when_expiration_column_absent`, after `assert_eq!(vms.len(), 1);`:

```rust
        assert_eq!(vms[0].rowid, 1);
        assert_eq!(vms[0].audio_file, None);
```

- [ ] **Step 4: Run tests, expect PASS.**

Run: `cargo test -p archive voicemail`
Expected: PASS (parser tests now assert `rowid`/`audio_file`).

- [ ] **Step 5: Confirm the whole crate compiles and is clippy-clean.**

Run: `cargo clippy -p archive --all-targets`
Expected: 0 errors, 0 warnings.

- [ ] **Step 6: Commit.**

```bash
git add archive/src/voicemail.rs archive/src/format.rs
git commit -m "feat(voicemail): carry rowid and audio_file on Voicemail records"
```

---

### Task 2: `voicemail_audio` module — formats, filenames, ffmpeg helpers

**Files:**
- Create: `archive/src/voicemail_audio.rs`
- Modify: `archive/src/main.rs` (add `#[allow(dead_code)] mod voicemail_audio;` near the other `mod` lines, after `mod voicemail;`)
- Test: `archive/src/voicemail_audio.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `enum AudioFormat { Amr, M4a, Wav }` with `from_cli(&str) -> Option<AudioFormat>`, `extension(self) -> &'static str`, `needs_ffmpeg(self) -> bool`.
  - `audio_filename(date: &str, sender: &str, rowid: i64, ext: &str) -> String`.
  - `transcode_args(input: &Path, output: &Path, format: AudioFormat) -> Option<Vec<OsString>>`.
  - `ffmpeg_available() -> bool`.

- [ ] **Step 1: Declare the module.** In `archive/src/main.rs`, add after `mod voicemail;` (line 6):

```rust
#[allow(dead_code)]
mod voicemail_audio;
```

(The `#[allow(dead_code)]` is removed in Task 4 once `run_voicemail` uses the module.)

- [ ] **Step 2: Write the module with failing tests first.** Create `archive/src/voicemail_audio.rs`:

```rust
//! Voicemail audio: output format, filenames, and ffmpeg transcoding helpers.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// Output format for extracted voicemail audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Raw `.amr` copied straight from the backup (no transcoding).
    Amr,
    /// AAC in an `.m4a` container (transcoded via ffmpeg).
    M4a,
    /// PCM `.wav` (transcoded via ffmpeg).
    Wav,
}

impl AudioFormat {
    /// Parse a CLI value (`amr`/`m4a`/`wav`, case-insensitive).
    pub fn from_cli(s: &str) -> Option<AudioFormat> {
        match s.to_ascii_lowercase().as_str() {
            "amr" => Some(AudioFormat::Amr),
            "m4a" => Some(AudioFormat::M4a),
            "wav" => Some(AudioFormat::Wav),
            _ => None,
        }
    }

    /// File extension (no leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Amr => "amr",
            AudioFormat::M4a => "m4a",
            AudioFormat::Wav => "wav",
        }
    }

    /// Whether producing this format requires transcoding via ffmpeg.
    pub fn needs_ffmpeg(self) -> bool {
        self != AudioFormat::Amr
    }
}

/// Build a readable, collision-free audio filename `<date>_<sender>_<rowid>.<ext>`.
/// `date` is the record's ISO timestamp compacted to `YYYY-MM-DD_HHMMSS` (or
/// `unknown`); `sender` keeps `[A-Za-z0-9+]` and maps other characters to `_`
/// (or `unknown` when empty). `rowid` guarantees uniqueness.
pub fn audio_filename(date: &str, sender: &str, rowid: i64, ext: &str) -> String {
    format!("{}_{}_{}.{ext}", compact_date(date), sanitize_sender(sender), rowid)
}

/// "2020-09-13T12:26:40+00:00" -> "2020-09-13_122640"; anything unexpected -> "unknown".
fn compact_date(iso: &str) -> String {
    if iso.len() < 19 || !iso.is_char_boundary(10) || !iso.is_char_boundary(19) {
        return "unknown".to_string();
    }
    let date = &iso[..10]; // YYYY-MM-DD
    let time: String = iso[11..19].chars().filter(|c| c.is_ascii_digit()).collect(); // HHMMSS
    if date.len() == 10 && time.len() == 6 {
        format!("{date}_{time}")
    } else {
        "unknown".to_string()
    }
}

fn sanitize_sender(sender: &str) -> String {
    if sender.is_empty() {
        return "unknown".to_string();
    }
    sender
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '+' { c } else { '_' })
        .collect()
}

/// Build the ffmpeg argument vector to transcode `input` (a fetched `.amr`)
/// into `output` for `format`. `Amr` needs no transcoding and returns `None`.
pub fn transcode_args(input: &Path, output: &Path, format: AudioFormat) -> Option<Vec<OsString>> {
    let mut args: Vec<OsString> = vec!["-y".into(), "-i".into(), input.as_os_str().to_owned()];
    match format {
        AudioFormat::Amr => return None,
        AudioFormat::M4a => {
            args.push("-c:a".into());
            args.push("aac".into());
        }
        AudioFormat::Wav => {}
    }
    args.push(output.as_os_str().to_owned());
    Some(args)
}

/// Whether an `ffmpeg` binary is on PATH (probed by running `ffmpeg -version`).
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn from_cli_parses_known_formats_case_insensitively() {
        assert_eq!(AudioFormat::from_cli("amr"), Some(AudioFormat::Amr));
        assert_eq!(AudioFormat::from_cli("M4A"), Some(AudioFormat::M4a));
        assert_eq!(AudioFormat::from_cli("Wav"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_cli("ogg"), None);
    }

    #[test]
    fn extension_and_needs_ffmpeg() {
        assert_eq!(AudioFormat::Amr.extension(), "amr");
        assert_eq!(AudioFormat::M4a.extension(), "m4a");
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert!(!AudioFormat::Amr.needs_ffmpeg());
        assert!(AudioFormat::M4a.needs_ffmpeg());
        assert!(AudioFormat::Wav.needs_ffmpeg());
    }

    #[test]
    fn filename_is_readable_for_normal_input() {
        let name = audio_filename("2020-09-13T12:26:40+00:00", "+420776452878", 3, "m4a");
        assert_eq!(name, "2020-09-13_122640_+420776452878_3.m4a");
    }

    #[test]
    fn filename_falls_back_for_empty_sender_and_date() {
        assert_eq!(audio_filename("", "", 7, "amr"), "unknown_unknown_7.amr");
    }

    #[test]
    fn filename_sanitizes_unsafe_sender_chars() {
        // Spaces, slashes, and quotes collapse to underscores; '+' and alnum survive.
        let name = audio_filename("2020-09-13T12:26:40+00:00", "a/b \"c\"", 5, "wav");
        assert_eq!(name, "2020-09-13_122640_a_b__c__5.wav");
    }

    #[test]
    fn filename_is_unique_per_rowid() {
        let a = audio_filename("2020-09-13T12:26:40+00:00", "+420", 1, "amr");
        let b = audio_filename("2020-09-13T12:26:40+00:00", "+420", 2, "amr");
        assert_ne!(a, b);
    }

    #[test]
    fn transcode_args_for_amr_is_none() {
        assert!(transcode_args(Path::new("in.amr"), Path::new("out.amr"), AudioFormat::Amr).is_none());
    }

    #[test]
    fn transcode_args_for_m4a_and_wav() {
        let m4a = transcode_args(Path::new("in.amr"), Path::new("out.m4a"), AudioFormat::M4a).unwrap();
        let m4a: Vec<String> = m4a.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(m4a, vec!["-y", "-i", "in.amr", "-c:a", "aac", "out.m4a"]);

        let wav = transcode_args(Path::new("in.amr"), Path::new("out.wav"), AudioFormat::Wav).unwrap();
        let wav: Vec<String> = wav.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(wav, vec!["-y", "-i", "in.amr", "out.wav"]);
    }
}
```

- [ ] **Step 3: Run the module tests, expect PASS.**

Run: `cargo test -p archive voicemail_audio`
Expected: PASS (8 tests in the module).

- [ ] **Step 4: Confirm clippy-clean (dead_code is suppressed by the module attribute).**

Run: `cargo clippy -p archive --all-targets`
Expected: 0 errors, 0 warnings.

- [ ] **Step 5: Commit.**

```bash
git add archive/src/voicemail_audio.rs archive/src/main.rs
git commit -m "feat(voicemail): add audio format, filename, and ffmpeg helpers"
```

---

### Task 3: `extract_audio` orchestration

**Files:**
- Modify: `archive/src/voicemail_audio.rs` (add `AudioSummary`, `extract_audio`, `run_transcode`)
- Test: `archive/src/voicemail_audio.rs` (gated integration test)

**Interfaces:**
- Consumes: `crate::voicemail::Voicemail` (mutated in place), `archive_core::Backup::fetch`, `audio_filename`, `transcode_args`.
- Produces:
  - `struct AudioSummary { pub format: AudioFormat, pub dir: String, pub extracted: usize, pub missing: usize }`.
  - `extract_audio(backup: &archive_core::Backup, items: &mut [crate::voicemail::Voicemail], out: &Path, format: AudioFormat) -> std::io::Result<AudioSummary>`.

- [ ] **Step 1: Add the orchestration.** Append to `archive/src/voicemail_audio.rs` (before `#[cfg(test)] mod tests`). Also add `use crate::voicemail::Voicemail;` to the module's `use` block at the top.

```rust
/// Per-run audio extraction outcome, surfaced in the JSON envelope.
pub struct AudioSummary {
    /// The output format that was produced.
    pub format: AudioFormat,
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Files written (including raw `.amr` kept as a transcode fallback).
    pub extracted: usize,
    /// Rows with no audio present in the backup.
    pub missing: usize,
}

/// Subdirectory (under the export dir) that receives the audio files.
const AUDIO_DIR: &str = "voicemail_audio";

/// Fetch (and optionally transcode) each voicemail's audio into
/// `<out>/voicemail_audio/`, filling each record's `audio_file` in place.
///
/// Best-effort: a row whose audio is absent in the backup is counted `missing`
/// and left `None`; a per-file transcode failure keeps the raw `.amr` and links
/// it. Only directory creation / temp-dir failures are fatal (returned as I/O
/// errors). The caller must have verified ffmpeg availability for non-`amr`
/// formats before calling (see `resolve_audio_format`).
pub fn extract_audio(
    backup: &archive_core::Backup,
    items: &mut [Voicemail],
    out: &Path,
    format: AudioFormat,
) -> std::io::Result<AudioSummary> {
    let audio_dir = out.join(AUDIO_DIR);
    std::fs::create_dir_all(&audio_dir)?;
    let scratch = tempfile::TempDir::new()?;

    let mut extracted = 0usize;
    let mut missing = 0usize;

    for item in items.iter_mut() {
        let src = format!("Library/Voicemail/{}.amr", item.rowid);

        if format == AudioFormat::Amr {
            let name = audio_filename(&item.date, &item.sender, item.rowid, "amr");
            let dest = audio_dir.join(&name);
            match backup.fetch("HomeDomain", &src, &dest) {
                Ok(Some(_)) => {
                    item.audio_file = Some(format!("{AUDIO_DIR}/{name}"));
                    extracted += 1;
                }
                Ok(None) => missing += 1,
                Err(why) => {
                    eprintln!("voicemail {}: audio fetch failed: {why}", item.rowid);
                    missing += 1;
                }
            }
            continue;
        }

        // Transcoding path: fetch the raw `.amr` to scratch, then transcode.
        let raw = scratch.path().join(format!("{}.amr", item.rowid));
        match backup.fetch("HomeDomain", &src, &raw) {
            Ok(Some(_)) => {}
            Ok(None) => {
                missing += 1;
                continue;
            }
            Err(why) => {
                eprintln!("voicemail {}: audio fetch failed: {why}", item.rowid);
                missing += 1;
                continue;
            }
        }

        let name = audio_filename(&item.date, &item.sender, item.rowid, format.extension());
        let dest = audio_dir.join(&name);
        if run_transcode(&raw, &dest, format) {
            item.audio_file = Some(format!("{AUDIO_DIR}/{name}"));
            extracted += 1;
        } else {
            // Transcode failed: keep the raw `.amr` as a fallback.
            let amr_name = audio_filename(&item.date, &item.sender, item.rowid, "amr");
            let amr_dest = audio_dir.join(&amr_name);
            if std::fs::copy(&raw, &amr_dest).is_ok() {
                eprintln!("voicemail {}: ffmpeg transcode failed; kept raw .amr", item.rowid);
                item.audio_file = Some(format!("{AUDIO_DIR}/{amr_name}"));
                extracted += 1;
            } else {
                eprintln!("voicemail {}: transcode and raw fallback both failed", item.rowid);
                missing += 1;
            }
        }
    }

    Ok(AudioSummary { format, dir: AUDIO_DIR.to_string(), extracted, missing })
}

/// Run ffmpeg to transcode `raw` into `dest`. Returns `true` on success.
fn run_transcode(raw: &Path, dest: &Path, format: AudioFormat) -> bool {
    let Some(args) = transcode_args(raw, dest, format) else {
        return false;
    };
    Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
```

- [ ] **Step 2: Add a gated end-to-end test.** Inside `mod tests` in `archive/src/voicemail_audio.rs`, add (the backup-touching path is covered here; it skips unless `ARCHIVE_TEST_BACKUP` is set, matching the `archive-core` test convention):

```rust
    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP (and
    // ARCHIVE_TEST_PASSWORD if encrypted). Skipped when unset so CI stays green.
    #[test]
    fn extracts_real_voicemail_audio_as_amr() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup =
            archive_core::Backup::open(Path::new(&dir), pw.as_deref()).expect("open backup");

        // Load the real voicemail metadata.
        let scratch = tempfile::TempDir::new().unwrap();
        let db = scratch.path().join("voicemail.db");
        let Some(db) = backup
            .fetch("HomeDomain", "Library/Voicemail/voicemail.db", &db)
            .expect("fetch voicemail.db")
        else {
            eprintln!("backup has no voicemail store; skipping");
            return;
        };
        let mut items = crate::voicemail::parse(&db).expect("parse voicemail");

        let out = scratch.path().join("out");
        let summary = extract_audio(&backup, &mut items, &out, AudioFormat::Amr).expect("extract");

        // The summary must be internally consistent with the records.
        assert_eq!(summary.dir, "voicemail_audio");
        let linked = items.iter().filter(|v| v.audio_file.is_some()).count();
        assert_eq!(summary.extracted, linked);
        assert_eq!(summary.extracted + summary.missing, items.len());
        // Every linked file exists on disk and is non-empty.
        for v in items.iter().filter_map(|v| v.audio_file.as_ref()) {
            let p = out.join(v.strip_prefix("voicemail_audio/").unwrap());
            assert!(p.is_file(), "linked audio should exist: {}", p.display());
            assert!(std::fs::metadata(&p).unwrap().len() > 0, "audio should be non-empty");
        }
    }
```

- [ ] **Step 3: Run tests, expect PASS (the gated test self-skips locally).**

Run: `cargo test -p archive voicemail_audio`
Expected: PASS; the gated test prints `skipping: set ARCHIVE_TEST_BACKUP to run` unless the env var is set.

- [ ] **Step 4: Confirm clippy-clean.**

Run: `cargo clippy -p archive --all-targets`
Expected: 0 errors, 0 warnings.

- [ ] **Step 5: Commit.**

```bash
git add archive/src/voicemail_audio.rs
git commit -m "feat(voicemail): extract and optionally transcode voicemail audio"
```

---

### Task 4: CLI flags, fail-fast resolution, and envelope wiring

**Files:**
- Modify: `archive/src/main.rs` (Command::Voicemail fields, dispatch, `resolve_audio_format`, `run_voicemail`, remove the `#[allow(dead_code)]` on `mod voicemail_audio;`)
- Test: `archive/src/main.rs` (`#[cfg(test)] mod tests` — update the existing parse test, add `resolve_audio_format` tests)

**Interfaces:**
- Consumes: `voicemail_audio::{AudioFormat, ffmpeg_available, extract_audio, AudioSummary}`.
- Produces: `resolve_audio_format(audio: bool, audio_format: Option<&str>) -> Result<Option<voicemail_audio::AudioFormat>, AppError>`; `run_voicemail(&Cli, Option<&str>, &str, bool, Option<&str>) -> Result<serde_json::Value, AppError>`.

- [ ] **Step 1: Remove the dead_code allow.** In `archive/src/main.rs`, change

```rust
#[allow(dead_code)]
mod voicemail_audio;
```
to
```rust
mod voicemail_audio;
```

- [ ] **Step 2: Add the flags.** Replace the `Voicemail` arm of `enum Command`:

```rust
    /// Export voicemail metadata (optionally extract audio with --audio).
    Voicemail {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
        /// Also extract each voicemail's audio into <out>/voicemail_audio/.
        #[arg(long)]
        audio: bool,
        /// Audio output format: amr (raw, default), m4a, or wav (m4a/wav need ffmpeg).
        #[arg(long)]
        audio_format: Option<String>,
    },
```

- [ ] **Step 3: Update the dispatch.** In `run`, replace the `Command::Voicemail` line:

```rust
        Command::Voicemail { format, audio, audio_format } => {
            run_voicemail(&cli, password.as_deref(), format, *audio, audio_format.as_deref())
        }
```

- [ ] **Step 4: Add `resolve_audio_format`.** In `archive/src/main.rs`, add this function just above `run_voicemail`:

```rust
/// Resolve the requested audio format from `--audio` / `--audio-format`.
/// `Ok(None)` = audio off. Usage error when `--audio-format` is given without
/// `--audio` or names an unknown format; a fatal error (exit 1) when a
/// transcoding format is requested but ffmpeg is not on PATH (fail fast, before
/// any extraction).
fn resolve_audio_format(
    audio: bool,
    audio_format: Option<&str>,
) -> Result<Option<voicemail_audio::AudioFormat>, AppError> {
    use voicemail_audio::AudioFormat;
    if !audio {
        if audio_format.is_some() {
            return Err(AppError::usage("--audio-format requires --audio"));
        }
        return Ok(None);
    }
    let fmt = match audio_format {
        None => AudioFormat::Amr,
        Some(s) => AudioFormat::from_cli(s).ok_or_else(|| {
            AppError::usage(format!("unknown audio format `{s}` (use amr, m4a, wav)"))
        })?,
    };
    if fmt.needs_ffmpeg() && !voicemail_audio::ffmpeg_available() {
        return Err(AppError::other(format!(
            "audio format `{}` requires ffmpeg, which was not found on PATH; \
             install ffmpeg or use --audio-format amr",
            fmt.extension()
        )));
    }
    Ok(Some(fmt))
}
```

- [ ] **Step 5: Rewrite `run_voicemail`.** Replace the whole `run_voicemail` function:

```rust
fn run_voicemail(
    cli: &Cli,
    password: Option<&str>,
    format: &str,
    audio: bool,
    audio_format: Option<&str>,
) -> Result<serde_json::Value, AppError> {
    let format = voicemail_format(format)?;
    // Resolve audio up front so a bad flag combo / missing ffmpeg fails fast.
    let audio_fmt = resolve_audio_format(audio, audio_format)?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export voicemail"))?;

    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_voicemail(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "voicemail", "count": 0, "outputs": [],
            "note": "this backup has no voicemail", "device": device
        }));
    };

    // Extract audio before rendering so `audio_file` is populated in the output.
    let audio_summary = match audio_fmt {
        Some(fmt) => Some(
            voicemail_audio::extract_audio(&backup, &mut items, out, fmt)
                .map_err(|e| AppError::other(e.to_string()))?,
        ),
        None => None,
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

    let mut envelope = serde_json::json!({
        "ok": true, "command": "voicemail", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = audio_summary {
        eprintln!(
            "Extracted {} audio file(s) ({} missing) to {}/voicemail_audio",
            s.extracted,
            s.missing,
            out.display()
        );
        envelope["audio"] = serde_json::json!({
            "format": s.format.extension(),
            "dir": s.dir,
            "extracted": s.extracted,
            "missing": s.missing
        });
    }
    Ok(envelope)
}
```

- [ ] **Step 6: Update the existing parse test and add resolution tests.** In `archive/src/main.rs` `mod tests`, the existing `parses_voicemail_invocation` matches `Command::Voicemail { format }`; update its pattern to the new fields and assert defaults:

```rust
            Command::Voicemail { format, audio, audio_format } => {
                assert_eq!(format, "csv");
                assert!(!audio);
                assert_eq!(audio_format, None);
            }
```

Add new tests:

```rust
    #[test]
    fn resolve_audio_off_by_default() {
        assert!(super::resolve_audio_format(false, None).unwrap().is_none());
    }

    #[test]
    fn resolve_audio_format_without_flag_is_usage_error() {
        let err = super::resolve_audio_format(false, Some("m4a")).unwrap_err();
        assert_eq!(err.code, 1);
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn resolve_audio_defaults_to_amr() {
        let fmt = super::resolve_audio_format(true, None).unwrap();
        assert_eq!(fmt, Some(crate::voicemail_audio::AudioFormat::Amr));
    }

    #[test]
    fn resolve_unknown_audio_format_is_usage_error() {
        let err = super::resolve_audio_format(true, Some("ogg")).unwrap_err();
        assert_eq!(err.code, 1);
        assert_eq!(err.kind, "usage");
    }
```

- [ ] **Step 7: Run tests, expect PASS.**

Run: `cargo test -p archive`
Expected: PASS (all existing + new tests).

- [ ] **Step 8: Confirm clippy-clean (no dead_code now that the module is wired).**

Run: `cargo clippy -p archive --all-targets`
Expected: 0 errors, 0 warnings.

- [ ] **Step 9: Commit.**

```bash
git add archive/src/main.rs
git commit -m "feat(voicemail): wire --audio/--audio-format flags into the CLI"
```

---

### Task 5: Surface `audio_file` in CSV and HTML

**Files:**
- Modify: `archive/src/format.rs` (`voicemail_csv` header + row; tests)
- Modify: `archive/templates/voicemail.html` (Audio column)
- Test: `archive/src/format.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `voicemail::Voicemail::audio_file`. (JSON already serializes `rowid`/`audio_file` via the derive — no change to `voicemail_json`.)

- [ ] **Step 1: Add the CSV column.** In `archive/src/format.rs`, replace `voicemail_csv`'s header and row records:

```rust
pub fn voicemail_csv(items: &[crate::voicemail::Voicemail]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "sender", "date", "duration_seconds", "trashed", "trashed_at", "expiration", "flags",
        "audio_file",
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
            v.audio_file.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}
```

- [ ] **Step 2: Add the HTML player column.** In `archive/templates/voicemail.html`, change the header row to add an `Audio` column and add a trailing cell in the body row.

Header row (line 8) becomes:
```html
<tr><th>Odesílatel</th><th>Datum</th><th>Trvání (s)</th><th>V koši</th><th>Smazáno</th><th>Expirace</th><th>Flags</th><th>Audio</th></tr>
```

After the `<td>{{ v.flags }}</td>` line, add:
```html
<td>{% if let Some(a) = v.audio_file.as_ref() %}<audio controls src="{{ a }}"></audio>{% endif %}</td>
```

- [ ] **Step 3: Update/extend the formatter tests.** In `archive/src/format.rs` `mod tests`, update `voicemail_csv_has_header_and_row` to assert the new column, and add HTML tests. Replace the body of `voicemail_csv_has_header_and_row` and add two tests:

```rust
    #[test]
    fn voicemail_csv_has_header_and_row() {
        let out = voicemail_csv(&sample_voicemails());
        assert!(out.starts_with(
            "sender,date,duration_seconds,trashed,trashed_at,expiration,flags,audio_file"
        ));
        // The sample has no audio, so the row ends with an empty audio_file cell.
        assert!(out.contains("+420776452878,2020-09-13T12:26:40+00:00,30,false,,,0,"));
    }

    #[test]
    fn voicemail_html_renders_audio_player_when_present() {
        let mut items = sample_voicemails();
        items[0].audio_file = Some("voicemail_audio/2020-09-13_122640_+420_3.m4a".into());
        let out = voicemail_html(&items);
        assert!(out.contains(
            "<audio controls src=\"voicemail_audio/2020-09-13_122640_+420_3.m4a\"></audio>"
        ));
    }

    #[test]
    fn voicemail_html_escapes_audio_file_attribute() {
        // A crafted audio_file must not break out of the src attribute.
        let mut items = sample_voicemails();
        items[0].audio_file = Some("\"><script>alert(1)</script>".into());
        let out = voicemail_html(&items);
        assert!(!out.contains("\"><script>"));
        // askama 0.16 escapes <, >, " as numeric entities.
        assert!(out.contains("&#60;script&#62;"));
    }

    #[test]
    fn voicemail_json_includes_rowid_and_audio_file() {
        let mut items = sample_voicemails();
        items[0].audio_file = Some("voicemail_audio/x_3.amr".into());
        let out = voicemail_json(&items);
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["rowid"], 3);
        assert_eq!(back[0]["audio_file"], "voicemail_audio/x_3.amr");
    }
```

- [ ] **Step 4: Run tests, expect PASS.**

Run: `cargo test -p archive format`
Expected: PASS (including the two new HTML tests).

- [ ] **Step 5: Confirm clippy-clean and full crate green.**

Run: `cargo clippy -p archive --all-targets && cargo test -p archive`
Expected: 0 warnings; all tests pass.

- [ ] **Step 6: Commit.**

```bash
git add archive/src/format.rs archive/templates/voicemail.html
git commit -m "feat(voicemail): render audio_file in CSV column and HTML player"
```

---

### Task 6: Document the audio flags

**Files:**
- Modify: `AGENTS.md` (voicemail section)
- Modify: `archive/README.md` (status + usage)

**Interfaces:** none (docs only). The text must match the real CLI/envelope from Tasks 4–5.

- [ ] **Step 1: Update `AGENTS.md`.** In the voicemail section, document the two flags and the `audio` envelope. Add wording equivalent to:

> `archive --backup <dir> -o <out> voicemail -f <csv|json|html> [--audio] [--audio-format <amr|m4a|wav>]`
>
> With `--audio`, each voicemail's audio is fetched from `HomeDomain/Library/Voicemail/<rowid>.amr` into `<out>/voicemail_audio/` and linked from each record's `audio_file` field. `--audio-format` defaults to `amr` (raw copy, no dependencies); `m4a`/`wav` transcode via `ffmpeg` (required only then). `--audio-format` without `--audio` is a usage error.
>
> Each record gains `rowid` (stable per-backup id) and `audio_file` (output-relative path, or `null` when no audio exists for that row). The success envelope adds, when `--audio` ran:
> ```json
> "audio": { "format": "m4a", "dir": "voicemail_audio", "extracted": 10, "missing": 2 }
> ```
> `extracted` counts files written (including raw `.amr` kept when a transcode fails); `missing` counts rows with no audio in the backup.

- [ ] **Step 2: Update `archive/README.md`.** Change the voicemail status line (currently `- [x] voicemail — csv, json, html (metadata; audio files not extracted)`) to reflect audio extraction, and add a usage line. New status line:

```markdown
- [x] voicemail — csv, json, html (metadata) + audio extraction (`--audio`, raw `.amr` or ffmpeg `m4a`/`wav`)
```

Under the quick-start examples, add:

```markdown
# Extract voicemail metadata + audio (raw .amr; pass --audio-format m4a|wav to transcode via ffmpeg)
archive --backup <backup-dir> -o <out> voicemail -f json --audio
```

Also remove `voicemail audio` from the not-done bullet (`- [ ] photos · notes · attachment/voicemail audio · pdf output` → `- [ ] photos · notes · attachment audio · pdf output`).

- [ ] **Step 3: Commit.**

```bash
git add AGENTS.md archive/README.md
git commit -m "docs: document voicemail --audio/--audio-format extraction"
```

---

## Self-Review notes

- **Spec coverage:** CLI flags (T4), `rowid`+`audio_file` model (T1), extraction module + naming + transcoding + best-effort + fail-fast (T2/T3/T4), envelope `audio` block (T4), CSV/JSON/HTML surfacing (T1 json via derive, T5 csv/html), testing strategy (pure helpers T2, gated integration T3, formatter+XSS T5, resolution T4), docs (T6). All spec sections map to a task.
- **No new mandatory dependency:** ffmpeg is only invoked for `m4a`/`wav` and probed via `ffmpeg_available`; the `amr` default and metadata paths never touch it.
- **Type consistency:** `AudioFormat`, `audio_filename`, `transcode_args`, `extract_audio`, `AudioSummary`, `resolve_audio_format`, and the `Voicemail` fields are named identically everywhere they appear across tasks.
- **Best-effort vs fatal:** only directory/temp creation is fatal in `extract_audio`; per-row fetch/transcode failures degrade gracefully. Fail-fast (missing ffmpeg / bad flag combo) happens in `resolve_audio_format` before any work.

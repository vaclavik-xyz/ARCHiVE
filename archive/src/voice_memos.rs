//! Read Voice Memos metadata from an iOS `CloudRecordings.db`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::audio::{compact_date, run_transcode, sanitize_component, AudioFormat};
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

/// Parse the Voice Memos store. Schema-tolerant: the title and path columns vary
/// by iOS version, so they are selected only when present.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<VoiceMemo>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZCLOUDRECORDING")?;
    // Title column varies by iOS version; when both exist, fall back per-row so a
    // row with a NULL `ZCUSTOMLABEL` still keeps its `ZENCRYPTEDTITLE` name.
    let label_sel = match (cols.contains("ZCUSTOMLABEL"), cols.contains("ZENCRYPTEDTITLE")) {
        (true, true) => "COALESCE(ZCUSTOMLABEL, ZENCRYPTEDTITLE)",
        (true, false) => "ZCUSTOMLABEL",
        (false, true) => "ZENCRYPTEDTITLE",
        (false, false) => "NULL",
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
pub(crate) fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// Backup locations of the Voice Memos metadata DB, modern first then legacy
/// (iOS ≤ 11). Single source of truth shared by export loading and `inspect`
/// discovery so the two never disagree.
pub const DB_LOCATIONS: [(&str, &str); 2] = [
    ("AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db"),
    ("MediaDomain", "Recordings/Recordings.db"),
];

/// Subdirectory (under the export dir) that receives the recordings.
const VM_DIR: &str = "voice_memos";
/// Backup domains to probe, in order (modern group container, then legacy).
const DOMAINS: [&str; 2] = ["AppDomainGroup-group.com.apple.VoiceMemos", "MediaDomain"];
/// Recognized audio file extensions under `Recordings/` (lowercase).
const AUDIO_EXTS: [&str; 7] = ["m4a", "caf", "wav", "aifc", "aiff", "mp4", "m4r"];

/// Per-run extraction outcome, surfaced in the JSON envelope.
pub struct VmSummary {
    /// Produced format: `raw` for native copies, else `m4a`/`wav`.
    pub format: String,
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Recordings written (including raw copies kept when a transcode fails).
    pub extracted: usize,
    /// Records with no audio found in the backup.
    pub missing: usize,
}

/// Build a readable, collision-free output filename `<date>_<title>_<n>.<ext>`.
/// `date` is compacted to `YYYY-MM-DD_HHMMSS` (or `unknown`); `title` keeps
/// `[A-Za-z0-9+]` and maps the rest to `_` (or `unknown` when empty); the 1-based
/// running index `n` guarantees uniqueness (Voice Memos have no small stable id).
pub(crate) fn output_name(date: &str, title: &str, n: usize, ext: &str) -> String {
    format!("{}_{}_{}.{ext}", compact_date(date), sanitize_component(title), n)
}

/// Lowercased audio extension of `path` when it is a recognized recording type.
fn audio_ext(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    if path.contains('.') && AUDIO_EXTS.contains(&ext.as_str()) {
        Some(ext)
    } else {
        None
    }
}

/// Fetch (and optionally transcode) every Voice Memos recording into
/// `<out>/voice_memos/`, filling each record's `audio_file` in place and
/// synthesizing records for on-disk recordings absent from the metadata DB.
///
/// `format = None` copies the native file as-is (no ffmpeg). `Some(fmt)`
/// transcodes via ffmpeg; a per-file transcode failure keeps the raw native copy
/// (best-effort, mirroring voicemail). Only directory/temp creation is fatal.
pub fn extract_voice_memos(
    backup: &archive_core::Backup,
    items: &mut Vec<VoiceMemo>,
    out: &Path,
    format: Option<AudioFormat>,
) -> std::io::Result<VmSummary> {
    let to_io = |e: archive_core::BackupError| std::io::Error::other(e.to_string());
    let fmt_label = format.map(|f| f.extension().to_string()).unwrap_or_else(|| "raw".to_string());

    // Pick the first domain that actually holds recordings.
    for domain in DOMAINS {
        let listed = backup.list(domain, "Recordings/").map_err(to_io)?;
        let audio: Vec<String> = listed.into_iter().filter(|p| audio_ext(p).is_some()).collect();
        if !audio.is_empty() {
            return extract_from(backup, items, out, format, &fmt_label, domain, audio);
        }
    }

    // No recordings anywhere: every DB record is missing; write nothing.
    Ok(VmSummary {
        format: fmt_label,
        dir: VM_DIR.to_string(),
        extracted: 0,
        missing: items.len(),
    })
}

/// Core extraction loop for a known `domain` and its `audio_paths`.
fn extract_from(
    backup: &archive_core::Backup,
    items: &mut Vec<VoiceMemo>,
    out: &Path,
    format: Option<AudioFormat>,
    fmt_label: &str,
    domain: &str,
    audio_paths: Vec<String>,
) -> std::io::Result<VmSummary> {
    let vm_dir = out.join(VM_DIR);
    std::fs::create_dir_all(&vm_dir)?;
    let scratch = tempfile::TempDir::new()?;

    for (i, path) in audio_paths.iter().enumerate() {
        let n = i + 1;
        let base = basename(path);
        let native_ext = audio_ext(path).unwrap_or_else(|| "bin".to_string());
        let idx = items
            .iter()
            .position(|m| !m.source_file.is_empty() && m.source_file == base);
        let (title, date) = match idx {
            Some(j) => (items[j].title.clone(), items[j].date.clone()),
            None => (String::new(), String::new()),
        };

        // Produce the linked output path, or None on failure.
        let linked = match format {
            None => {
                let name = output_name(&date, &title, n, &native_ext);
                let dest = vm_dir.join(&name);
                match backup.fetch(domain, path, &dest) {
                    Ok(Some(_)) => Some(format!("{VM_DIR}/{name}")),
                    Ok(None) => None,
                    Err(why) => {
                        eprintln!("voice memo {base}: fetch failed: {why}");
                        None
                    }
                }
            }
            Some(fmt) => {
                let raw = scratch.path().join(format!("{n}.{native_ext}"));
                match backup.fetch(domain, path, &raw) {
                    Ok(Some(_)) => {}
                    Ok(None) => continue,
                    Err(why) => {
                        eprintln!("voice memo {base}: fetch failed: {why}");
                        continue;
                    }
                }
                let name = output_name(&date, &title, n, fmt.extension());
                let dest = vm_dir.join(&name);
                if run_transcode(&raw, &dest, fmt) {
                    Some(format!("{VM_DIR}/{name}"))
                } else {
                    // Keep the raw native copy as a fallback.
                    let raw_name = output_name(&date, &title, n, &native_ext);
                    let raw_dest = vm_dir.join(&raw_name);
                    if std::fs::copy(&raw, &raw_dest).is_ok() {
                        eprintln!("voice memo {base}: transcode failed; kept raw {native_ext}");
                        Some(format!("{VM_DIR}/{raw_name}"))
                    } else {
                        eprintln!("voice memo {base}: transcode and raw fallback both failed");
                        None
                    }
                }
            }
        };

        if let Some(rel) = linked {
            match idx {
                Some(j) => items[j].audio_file = Some(rel),
                None => items.push(VoiceMemo {
                    title: String::new(),
                    date: String::new(),
                    duration_seconds: 0,
                    source_file: base,
                    audio_file: Some(rel),
                }),
            }
        }
    }

    // Count from the final state so `extracted + missing == items.len()` holds by
    // construction — robust even if two source paths happened to share a basename.
    let extracted = items.iter().filter(|m| m.audio_file.is_some()).count();
    let missing = items.iter().filter(|m| m.audio_file.is_none()).count();
    Ok(VmSummary { format: fmt_label.to_string(), dir: VM_DIR.to_string(), extracted, missing })
}

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

    #[test]
    fn title_falls_back_to_encrypted_title_per_row() {
        use rusqlite::Connection;
        let dir = std::env::temp_dir().join(format!("be-vm-coalesce-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("CloudRecordings.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        // Both title columns exist; row 1 has only ZCUSTOMLABEL, row 2 only ZENCRYPTEDTITLE.
        conn.execute_batch(
            "CREATE TABLE ZCLOUDRECORDING (Z_PK INTEGER PRIMARY KEY, ZDATE REAL, ZDURATION REAL,
                ZCUSTOMLABEL TEXT, ZENCRYPTEDTITLE TEXT, ZPATH TEXT);
             INSERT INTO ZCLOUDRECORDING (Z_PK, ZDATE, ZDURATION, ZCUSTOMLABEL, ZENCRYPTEDTITLE, ZPATH) VALUES
                (1, 600000000.0, 1.0, 'Custom', NULL, 'a.m4a'),
                (2, 600000100.0, 1.0, NULL, 'Encrypted', 'b.m4a');",
        )
        .unwrap();
        drop(conn);

        let memos = parse(&db).unwrap();
        assert_eq!(memos[0].title, "Custom");
        assert_eq!(memos[1].title, "Encrypted"); // fell back to ZENCRYPTEDTITLE
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn basename_takes_last_component() {
        assert_eq!(basename("a/b/c.m4a"), "c.m4a");
        assert_eq!(basename("x.m4a"), "x.m4a");
        assert_eq!(basename(""), "");
    }

    #[test]
    fn output_name_is_readable_and_sanitized() {
        // 'ů' is not ASCII-alphanumeric, so sanitize_component maps it to '_'.
        assert_eq!(
            output_name("2020-01-06T10:40:00+00:00", "Schůzka", 1, "m4a"),
            "2020-01-06_104000_Sch_zka_1.m4a"
        );
        assert_eq!(output_name("", "", 2, "m4a"), "unknown_unknown_2.m4a");
    }

    #[test]
    fn audio_ext_recognizes_known_types_only() {
        assert_eq!(audio_ext("Recordings/x.m4a"), Some("m4a".to_string()));
        assert_eq!(audio_ext("Recordings/x.CAF"), Some("caf".to_string()));
        assert_eq!(audio_ext("Recordings/CloudRecordings.db"), None);
        assert_eq!(audio_ext("Recordings/noext"), None);
    }

    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP (and
    // ARCHIVE_TEST_PASSWORD if encrypted). Skipped when unset so CI stays green.
    #[test]
    fn extracts_real_voice_memos() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = archive_core::Backup::open(Path::new(&dir), pw.as_deref()).expect("open backup");

        let scratch = tempfile::TempDir::new().unwrap();
        let db = scratch.path().join("CloudRecordings.db");
        let Some(db) = backup
            .fetch(
                "AppDomainGroup-group.com.apple.VoiceMemos",
                "Recordings/CloudRecordings.db",
                &db,
            )
            .expect("fetch CloudRecordings.db")
        else {
            eprintln!("backup has no voice memos store; skipping");
            return;
        };
        let mut items = parse(&db).expect("parse voice memos");

        let out = scratch.path().join("out");
        let summary = extract_voice_memos(&backup, &mut items, &out, None).expect("extract");
        assert_eq!(summary.dir, "voice_memos");
        assert_eq!(summary.extracted + summary.missing, items.len());
        for v in items.iter().filter_map(|v| v.audio_file.as_ref()) {
            let p = out.join(v);
            assert!(p.is_file(), "linked audio should exist: {}", p.display());
            assert!(std::fs::metadata(&p).unwrap().len() > 0, "audio should be non-empty");
        }
    }
}

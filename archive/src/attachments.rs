//! Read Messages attachment metadata from `sms.db` and extract the attachment
//! files from `MediaDomain`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_any_to_iso;
use crate::sqlite_util::table_columns;

/// One Messages (iMessage/SMS) attachment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attachment {
    /// Original transfer name, else the basename of the on-device path.
    pub name: String,
    /// MIME type, e.g. `image/jpeg`; empty when unknown.
    pub mime_type: String,
    /// Creation time as ISO 8601 UTC (Cocoa s or ns); empty if unconvertible.
    pub created: String,
    /// Size in bytes per the DB (0 when unknown).
    pub total_bytes: i64,
    /// `MediaDomain`-relative source path (`Library/SMS/Attachments/…`); empty
    /// when the stored filename has no recoverable Attachments path.
    pub source_path: String,
    /// Output-relative path to the extracted file (`attachments/<name>`); `None`
    /// until extraction runs or when the file is absent from the backup.
    pub file: Option<String>,
}

impl Attachment {
    /// Whether this attachment is an image (used by the HTML gallery to inline it).
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}

/// Last path component of a (possibly `/`-containing) name.
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Map an on-device attachment `filename` to its `MediaDomain` relative path.
///
/// iOS-backup attachment paths are stored as `~/Library/Messages/Attachments/…`
/// (iMessage) or `~/Library/SMS/Attachments/…` (SMS/MMS); the `MediaDomain`
/// relative path is everything after the leading `~/` — matching
/// `imessage-database`'s `gen_ios_attachment` (`file_path[2..]`, hashed as
/// `MediaDomain-<path>`). An absolute on-device path falls back to the first
/// `Library/` segment; anything else yields an empty (non-fetchable) path.
fn to_media_path(filename: &str) -> String {
    if let Some(rest) = filename.strip_prefix("~/") {
        rest.to_string()
    } else if filename.starts_with('/') {
        filename.find("Library/").map(|i| filename[i..].to_string()).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Parse the `attachment` table, ordered by creation date. Schema-tolerant:
/// optional columns are selected only when present.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Attachment>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "attachment")?;
    let col = |n: &'static str| -> &'static str { if cols.contains(n) { n } else { "NULL" } };
    let order = if cols.contains("created_date") { "created_date" } else { "ROWID" };

    let sql = format!(
        "SELECT filename, {}, {}, {}, {} FROM attachment ORDER BY {order}",
        col("mime_type"),
        col("transfer_name"),
        col("total_bytes"),
        col("created_date"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let filename: Option<String> = row.get(0)?;
        let mime_type: Option<String> = row.get(1)?;
        let transfer_name: Option<String> = row.get(2)?;
        let total_bytes: Option<i64> = row.get(3)?;
        let created_date: Option<i64> = row.get(4)?;

        let filename = filename.unwrap_or_default();
        let transfer_name = transfer_name.unwrap_or_default();
        let name = if transfer_name.is_empty() {
            basename(&filename).to_string()
        } else {
            transfer_name
        };
        Ok(Attachment {
            name,
            mime_type: mime_type.unwrap_or_default(),
            created: created_date.and_then(cocoa_any_to_iso).unwrap_or_default(),
            total_bytes: total_bytes.unwrap_or(0),
            source_path: to_media_path(&filename),
            file: None,
        })
    })?;
    rows.collect()
}

/// Per-run extraction outcome, surfaced in the JSON envelope.
pub struct AttachmentSummary {
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Files written.
    pub extracted: usize,
    /// Rows with no file present in the backup (or no recoverable path).
    pub missing: usize,
}

/// Subdirectory (under the export dir) that receives the attachment files.
const ATT_DIR: &str = "attachments";

/// Output filename `<n>_<basename(name)>` (1-based index ensures uniqueness while
/// preserving the original name).
pub(crate) fn output_name(n: usize, name: &str) -> String {
    format!("{n}_{}", basename(name))
}

/// Fetch each attachment's file into `<out>/attachments/`, filling `file` in
/// place. Best-effort: a row absent from the backup (or with no recoverable
/// path) is counted `missing`. Only directory creation is fatal.
pub fn extract_attachments(
    backup: &archive_core::Backup,
    items: &mut [Attachment],
    out: &Path,
) -> std::io::Result<AttachmentSummary> {
    let att_dir = out.join(ATT_DIR);
    std::fs::create_dir_all(&att_dir)?;

    for (i, item) in items.iter_mut().enumerate() {
        if item.source_path.is_empty() {
            continue;
        }
        let name = output_name(i + 1, &item.name);
        let dest = att_dir.join(&name);
        // Domain is `MediaDomain` (not HomeDomain): iOS backups hash the file id
        // as `SHA1("MediaDomain-<relative_path>")` — see imessage-database's
        // `gen_ios_attachment`. `source_path` is already MediaDomain-relative.
        match backup.fetch("MediaDomain", &item.source_path, &dest) {
            Ok(Some(_)) => item.file = Some(format!("{ATT_DIR}/{name}")),
            Ok(None) => {}
            Err(why) => eprintln!("attachment {}: fetch failed: {why}", item.name),
        }
    }

    let extracted = items.iter().filter(|a| a.file.is_some()).count();
    let missing = items.iter().filter(|a| a.file.is_none()).count();
    Ok(AttachmentSummary { dir: ATT_DIR.to_string(), extracted, missing })
}

/// Build a customer-facing summary of the recovered message attachments.
pub fn summary(items: &[Attachment], files_extracted: bool) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};

    // Top-level MIME token (the part before `/`); empty when the MIME is unknown.
    fn top(a: &Attachment) -> &str {
        a.mime_type.split('/').next().unwrap_or("")
    }
    // Lowercased file extension (after the last `.`), else a stable placeholder.
    let ext = |a: &Attachment| match a.name.rsplit_once('.') {
        Some((_, e)) if !e.is_empty() => e.to_lowercase(),
        _ => "bez přípony".to_string(),
    };

    let images = items.iter().filter(|a| a.mime_type.starts_with("image/")).count();
    let videos = items.iter().filter(|a| a.mime_type.starts_with("video/")).count();
    let audio = items.iter().filter(|a| a.mime_type.starts_with("audio/")).count();
    // Empty MIME, or a top-level type that is none of image/video/audio.
    let documents = items.iter().filter(|a| !matches!(top(a), "image" | "video" | "audio")).count();
    let total_mb = (items.iter().map(|a| a.total_bytes).filter(|&b| b > 0).sum::<i64>() / 1_048_576) as usize;

    let mut s = Summary::new("attachments", "Přílohy zpráv", "příloh", items.len())
        .count("Obrázků", images)
        .count("Videí", videos)
        .count("Audio zpráv", audio)
        .count("Dokumentů a ostatní", documents)
        .count("Celková velikost (MB)", total_mb);
    // Real backup presence is known only after extraction and matches the envelope's
    // extracted/missing counts; a non-empty source_path alone does not guarantee the
    // file is in the backup, so these are omitted under --no-files.
    if files_extracted {
        let recovered = items.iter().filter(|a| a.file.is_some()).count();
        let missing = items.iter().filter(|a| a.file.is_none()).count();
        s = s.count("Souborů obnoveno", recovered).count("Chybí v záloze", missing);
    }
    s.period_from(iso_range(items.iter().map(|a| a.created.as_str())))
        .breakdown("Po letech", year_rows(items.iter().map(|a| a.created.as_str())))
        .breakdown(
            "Podle typu",
            tally(items.iter().map(|a| {
                let t = top(a);
                if t.is_empty() { "neznámé".to_string() } else { t.to_string() }
            })),
        )
        .breakdown(
            "Podle přípony",
            tally(items.iter().map(ext)).into_iter().take(12).collect(),
        )
        .note("Velikost je dolní odhad — některé přílohy nemají uloženou velikost.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_sms_attachments;

    fn att(name: &str, mime: &str, created: &str, bytes: i64, source: &str) -> Attachment {
        Attachment {
            name: name.into(),
            mime_type: mime.into(),
            created: created.into(),
            total_bytes: bytes,
            source_path: source.into(),
            file: None,
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let mut items = vec![
            att("photo.jpg", "image/jpeg", "2023-05-01T10:00:00+00:00", 2_097_152, "Library/SMS/Attachments/a/photo.jpg"),
            att("clip.mov", "video/quicktime", "2024-06-01T10:00:00+00:00", 1_048_576, "Library/SMS/Attachments/b/clip.mov"),
            att("notes.pdf", "", "", 0, ""),
        ];
        // Two files were extracted, one was absent from the backup.
        items[0].file = Some("attachments/photo.jpg".into());
        items[1].file = Some("attachments/clip.mov".into());
        let s = summary(&items, true);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "příloh");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Obrázků"), Some(1));
        assert_eq!(get("Videí"), Some(1));
        assert_eq!(get("Dokumentů a ostatní"), Some(1)); // empty-MIME pdf
        assert_eq!(get("Celková velikost (MB)"), Some(3)); // (2 MiB + 1 MiB) / 1_048_576
        assert_eq!(get("Souborů obnoveno"), Some(2)); // file.is_some()
        assert_eq!(get("Chybí v záloze"), Some(1)); // file.is_none()
        let yr = s.breakdowns.iter().find(|b| b.title == "Po letech").unwrap();
        assert_eq!(yr.rows, vec![("2023".to_string(), 1), ("2024".to_string(), 1)]);
        assert!(s.period.is_some()); // derived from the two dated attachments

        // Under --no-files, availability is unknown and those counts are omitted.
        let meta_only = summary(&items, false);
        assert!(meta_only.counts.iter().all(|(l, _)| l != "Souborů obnoveno" && l != "Chybí v záloze"));
    }

    #[test]
    fn parses_attachments_mapping_paths_and_epochs() {
        let dir = std::env::temp_dir().join(format!("be-att-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("sms.db");
        let _ = std::fs::remove_file(&db);
        make_sms_attachments(&db);

        let items = parse(&db).unwrap();
        assert_eq!(items.len(), 3);

        let img = &items[0];
        assert_eq!(img.name, "photo.jpg");
        assert_eq!(img.mime_type, "image/jpeg");
        assert_eq!(img.created, "2020-01-06T10:40:00+00:00"); // ns date
        assert_eq!(img.source_path, "Library/Messages/Attachments/ab/12/GUID1/photo.jpg");
        assert_eq!(img.file, None);

        let vid = &items[1];
        assert_eq!(vid.source_path, "Library/SMS/Attachments/cd/34/GUID2/clip.mov");
        assert_eq!(vid.created, "2020-01-06T10:41:40+00:00"); // +100s

        let weird = &items[2];
        assert_eq!(weird.source_path, ""); // no Attachments path → not fetchable
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn output_name_is_unique_and_keeps_original() {
        assert_eq!(output_name(1, "photo.jpg"), "1_photo.jpg");
        assert_eq!(output_name(2, "a/b/clip.mov"), "2_clip.mov");
        assert_ne!(output_name(1, "x.jpg"), output_name(2, "x.jpg"));
    }

    #[test]
    fn to_media_path_strips_tilde_and_handles_absolute() {
        // iMessage and SMS both use the `~/` form in iOS backups.
        assert_eq!(
            to_media_path("~/Library/Messages/Attachments/a/b/x.jpg"),
            "Library/Messages/Attachments/a/b/x.jpg"
        );
        assert_eq!(
            to_media_path("~/Library/SMS/Attachments/c/d/y.mov"),
            "Library/SMS/Attachments/c/d/y.mov"
        );
        // Absolute on-device path falls back to the first Library/ segment.
        assert_eq!(
            to_media_path("/var/mobile/Library/Messages/Attachments/e/f/z.heic"),
            "Library/Messages/Attachments/e/f/z.heic"
        );
        assert_eq!(to_media_path("/nope/z.bin"), "");
    }

    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP (and
    // ARCHIVE_TEST_PASSWORD if encrypted). Skipped when unset so CI stays green.
    #[test]
    fn extracts_real_attachments() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = archive_core::Backup::open(Path::new(&dir), pw.as_deref()).expect("open backup");

        let scratch = tempfile::TempDir::new().unwrap();
        let db = scratch.path().join("sms.db");
        let Some(db) = backup
            .fetch("HomeDomain", "Library/SMS/sms.db", &db)
            .expect("fetch sms.db")
        else {
            eprintln!("backup has no Messages store; skipping");
            return;
        };
        let mut items = parse(&db).expect("parse attachments");

        let out = scratch.path().join("out");
        let summary = extract_attachments(&backup, &mut items, &out).expect("extract");
        assert_eq!(summary.dir, "attachments");
        assert_eq!(summary.extracted + summary.missing, items.len());
        for v in items.iter().filter_map(|a| a.file.as_ref()) {
            assert!(out.join(v).is_file(), "linked file should exist: {v}");
        }
    }
}

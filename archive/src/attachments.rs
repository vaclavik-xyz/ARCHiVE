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

/// Last path component of a (possibly `/`-containing) name.
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Map an on-device attachment `filename` to its `MediaDomain` relative path by
/// taking the substring from `Library/SMS/Attachments/` onward. Empty when the
/// path does not contain that segment.
fn to_media_path(filename: &str) -> String {
    const KEY: &str = "Library/SMS/Attachments/";
    match filename.find(KEY) {
        Some(i) => filename[i..].to_string(),
        None => String::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_sms_attachments;

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
        assert_eq!(img.source_path, "Library/SMS/Attachments/ab/12/GUID1/photo.jpg");
        assert_eq!(img.file, None);

        let vid = &items[1];
        assert_eq!(vid.source_path, "Library/SMS/Attachments/cd/34/GUID2/clip.mov");
        assert_eq!(vid.created, "2020-01-06T10:41:40+00:00"); // seconds date (+100s)

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
    fn to_media_path_strips_device_prefixes() {
        assert_eq!(
            to_media_path("~/Library/SMS/Attachments/a/b/x.jpg"),
            "Library/SMS/Attachments/a/b/x.jpg"
        );
        assert_eq!(
            to_media_path("/var/mobile/Library/SMS/Attachments/c/d/y.mov"),
            "Library/SMS/Attachments/c/d/y.mov"
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

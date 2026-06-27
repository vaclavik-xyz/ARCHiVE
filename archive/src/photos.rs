//! Read Camera Roll metadata from `Photos.sqlite` (`ZASSET`) and extract the
//! photo/video files from `CameraRollDomain`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One Camera Roll asset (photo or video).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Photo {
    /// File name, e.g. `IMG_0001.HEIC`.
    pub filename: String,
    /// `image`, `video`, or `unknown`.
    pub kind: String,
    /// Capture time as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub created: String,
    /// Marked as a favorite.
    pub favorite: bool,
    /// In Recently Deleted.
    pub trashed: bool,
    /// Pixel width / height (0 when unknown).
    pub width: i64,
    pub height: i64,
    /// Latitude/longitude when a valid fix exists (else `None`).
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Video length in seconds (`None` for photos / when unset).
    pub duration_seconds: Option<i64>,
    /// Backup-relative source path (`Media/<dir>/<file>`); empty when unknown.
    pub source_path: String,
    /// Output-relative path to the extracted file (`photos/<name>`); `None`
    /// until extraction runs or when the file is absent from the backup.
    pub file: Option<String>,
}

/// Last path component of a (possibly `/`-containing) name.
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// A coordinate is valid only when finite, within range, and not Apple's `-180`
/// "no location" sentinel.
fn valid_coord(v: Option<f64>, max: f64) -> Option<f64> {
    v.filter(|&x| x.is_finite() && x.abs() <= max && x != -180.0)
}

/// Parse the Camera Roll, ordered by capture date. Tolerates missing optional
/// columns across iOS versions; `ZFILENAME`/`ZDIRECTORY` build the source path.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Photo>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZASSET")?;
    let col = |n: &'static str| -> &'static str { if cols.contains(n) { n } else { "NULL" } };
    let order = if cols.contains("ZDATECREATED") { "ZDATECREATED" } else { "Z_PK" };

    let sql = format!(
        "SELECT ZFILENAME, ZDIRECTORY, {}, {}, {}, {}, {}, {}, {}, {}, {} \
         FROM ZASSET ORDER BY {order}",
        col("ZDATECREATED"),
        col("ZKIND"),
        col("ZFAVORITE"),
        col("ZTRASHEDSTATE"),
        col("ZWIDTH"),
        col("ZHEIGHT"),
        col("ZLATITUDE"),
        col("ZLONGITUDE"),
        col("ZDURATION"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let filename: Option<String> = row.get(0)?;
        let directory: Option<String> = row.get(1)?;
        let created: Option<f64> = row.get(2)?;
        let kind: Option<i64> = row.get(3)?;
        let favorite: Option<i64> = row.get(4)?;
        let trashed: Option<i64> = row.get(5)?;
        let width: Option<i64> = row.get(6)?;
        let height: Option<i64> = row.get(7)?;
        let lat: Option<f64> = row.get(8)?;
        let lon: Option<f64> = row.get(9)?;
        let dur: Option<f64> = row.get(10)?;

        let filename = filename.unwrap_or_default();
        let directory = directory.unwrap_or_default();
        let source_path = if filename.is_empty() {
            String::new()
        } else if directory.is_empty() {
            format!("Media/{filename}")
        } else {
            format!("Media/{directory}/{filename}")
        };
        Ok(Photo {
            filename: basename(&filename).to_string(),
            kind: match kind {
                Some(0) => "image",
                Some(1) => "video",
                _ => "unknown",
            }
            .to_string(),
            created: created.and_then(cocoa_to_iso).unwrap_or_default(),
            favorite: favorite == Some(1),
            trashed: trashed.unwrap_or(0) != 0,
            width: width.unwrap_or(0),
            height: height.unwrap_or(0),
            latitude: valid_coord(lat, 90.0),
            longitude: valid_coord(lon, 180.0),
            duration_seconds: if kind == Some(1) { dur.map(|d| d.round() as i64) } else { None },
            source_path,
            file: None,
        })
    })?;
    rows.collect()
}

/// Per-run extraction outcome, surfaced in the JSON envelope.
pub struct PhotoSummary {
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Files written.
    pub extracted: usize,
    /// Assets with no file present in the backup (e.g. iCloud-only).
    pub missing: usize,
}

/// Subdirectory (under the export dir) that receives the media files.
const PHOTO_DIR: &str = "photos";

/// Output filename `<n>_<filename>` (1-based index ensures uniqueness across
/// directories while preserving the original name).
pub(crate) fn output_name(n: usize, filename: &str) -> String {
    format!("{n}_{}", basename(filename))
}

/// Fetch each asset's file into `<out>/photos/`, filling `file` in place.
/// Best-effort: an asset absent from the backup is counted `missing`. Only
/// directory creation is fatal.
pub fn extract_photos(
    backup: &archive_core::Backup,
    items: &mut [Photo],
    out: &Path,
) -> std::io::Result<PhotoSummary> {
    let photo_dir = out.join(PHOTO_DIR);
    std::fs::create_dir_all(&photo_dir)?;

    for (i, item) in items.iter_mut().enumerate() {
        if item.source_path.is_empty() {
            continue;
        }
        let name = output_name(i + 1, &item.filename);
        let dest = photo_dir.join(&name);
        match backup.fetch("CameraRollDomain", &item.source_path, &dest) {
            Ok(Some(_)) => item.file = Some(format!("{PHOTO_DIR}/{name}")),
            Ok(None) => {}
            Err(why) => eprintln!("photo {}: fetch failed: {why}", item.filename),
        }
    }

    let extracted = items.iter().filter(|p| p.file.is_some()).count();
    let missing = items.iter().filter(|p| p.file.is_none()).count();
    Ok(PhotoSummary { dir: PHOTO_DIR.to_string(), extracted, missing })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_photos;

    #[test]
    fn parses_assets_with_kinds_flags_and_gps() {
        let dir = std::env::temp_dir().join(format!("be-photos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Photos.sqlite");
        let _ = std::fs::remove_file(&db);
        make_photos(&db);

        let assets = parse(&db).unwrap();
        assert_eq!(assets.len(), 3);

        let img = &assets[0];
        assert_eq!(img.filename, "IMG_0001.HEIC");
        assert_eq!(img.kind, "image");
        assert_eq!(img.created, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000
        assert!(img.favorite);
        assert!(!img.trashed);
        assert_eq!(img.width, 4032);
        assert_eq!(img.latitude, Some(50.087));
        assert_eq!(img.longitude, Some(14.42));
        assert_eq!(img.duration_seconds, None);
        assert_eq!(img.source_path, "Media/DCIM/100APPLE/IMG_0001.HEIC");
        assert_eq!(img.file, None);

        let vid = &assets[1];
        assert_eq!(vid.kind, "video");
        assert_eq!(vid.duration_seconds, Some(13)); // 12.5 rounded
        assert_eq!(vid.latitude, None); // -180 sentinel → None
        assert_eq!(vid.longitude, None);

        let trashed = &assets[2];
        assert!(trashed.trashed);
        assert_eq!(trashed.latitude, None); // NULL → None
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn output_name_is_unique_and_keeps_original() {
        assert_eq!(output_name(1, "IMG_0001.HEIC"), "1_IMG_0001.HEIC");
        assert_eq!(output_name(2, "a/b/IMG_0002.MOV"), "2_IMG_0002.MOV");
        assert_ne!(output_name(1, "x.jpg"), output_name(2, "x.jpg"));
    }

    #[test]
    fn valid_coord_filters_sentinel_and_range() {
        assert_eq!(valid_coord(Some(50.0), 90.0), Some(50.0));
        assert_eq!(valid_coord(Some(-180.0), 180.0), None); // sentinel
        assert_eq!(valid_coord(Some(91.0), 90.0), None); // out of range
        assert_eq!(valid_coord(Some(f64::NAN), 90.0), None);
        assert_eq!(valid_coord(None, 90.0), None);
    }

    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP (and
    // ARCHIVE_TEST_PASSWORD if encrypted). Skipped when unset so CI stays green.
    #[test]
    fn extracts_real_photos() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = archive_core::Backup::open(Path::new(&dir), pw.as_deref()).expect("open backup");

        let scratch = tempfile::TempDir::new().unwrap();
        let db = scratch.path().join("Photos.sqlite");
        let Some(db) = backup
            .fetch("CameraRollDomain", "Media/PhotoData/Photos.sqlite", &db)
            .expect("fetch Photos.sqlite")
        else {
            eprintln!("backup has no photos store; skipping");
            return;
        };
        let mut items = parse(&db).expect("parse photos");

        let out = scratch.path().join("out");
        let summary = extract_photos(&backup, &mut items, &out).expect("extract");
        assert_eq!(summary.dir, "photos");
        assert_eq!(summary.extracted + summary.missing, items.len());
        for v in items.iter().filter_map(|p| p.file.as_ref()) {
            let p = out.join(v);
            assert!(p.is_file(), "linked file should exist: {}", p.display());
        }
    }
}

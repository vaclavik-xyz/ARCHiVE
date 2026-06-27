//! Read Camera Roll metadata from `Photos.sqlite` (`ZASSET`) and extract the
//! photo/video files from `CameraRollDomain`.

use std::collections::HashMap;
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
    /// Last-modified time as ISO 8601 UTC; empty if unset.
    pub modified: String,
    /// Added-to-library time as ISO 8601 UTC; empty if unset.
    pub added: String,
    /// Marked as a favorite.
    pub favorite: bool,
    /// In the Hidden album (still recovered; just flagged).
    pub hidden: bool,
    /// In Recently Deleted.
    pub trashed: bool,
    /// Has edits/adjustments (`ZHASADJUSTMENTS`).
    pub edited: bool,
    /// Best-effort Live Photo flag (image with `ZKINDSUBTYPE == 2`).
    pub live_photo: bool,
    /// Raw `ZKINDSUBTYPE` (version-dependent; preserved for fidelity).
    pub kind_subtype: Option<i64>,
    /// Pixel width / height (0 when unknown).
    pub width: i64,
    pub height: i64,
    /// Latitude/longitude when a valid fix exists (else `None`).
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Video length in seconds (`None` for photos / when unset).
    pub duration_seconds: Option<i64>,
    /// Burst group identifier (`ZAVALANCHEUUID`); `None` when not a burst.
    pub burst_id: Option<String>,
    /// Original filename before any iCloud rename; empty when unknown.
    pub original_filename: String,
    /// User caption/title; empty when none.
    pub title: String,
    /// Album titles this asset belongs to (sorted, deduped).
    pub albums: Vec<String>,
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

/// Whether `s` is `Z_<digits><suffix>` (e.g. a `Z_28ASSETS`/`Z_28ALBUMS` name).
fn matches_z(s: &str, suffix: &str) -> bool {
    match s.strip_prefix("Z_").and_then(|x| x.strip_suffix(suffix)) {
        Some(mid) => !mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Whether a table exists in the database.
fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Discover the album↔asset many-to-many join (a `Z_<n>ASSETS` table with an
/// `…ALBUMS` and an `…ASSETS` column — the number is schema-version-dependent) and
/// build `asset_pk → album titles`. Empty when no such join / `ZGENERICALBUM`.
fn album_membership(conn: &Connection) -> rusqlite::Result<HashMap<i64, Vec<String>>> {
    if !table_exists(conn, "ZGENERICALBUM")? {
        return Ok(HashMap::new());
    }
    let tables: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut chosen: Option<(String, String, String)> = None;
    for name in tables.iter().filter(|n| matches_z(n, "ASSETS")) {
        let cols = table_columns(conn, name)?;
        let album_col = cols.iter().find(|c| matches_z(c, "ALBUMS")).cloned();
        let asset_col = cols.iter().find(|c| matches_z(c, "ASSETS")).cloned();
        if let (Some(ac), Some(sc)) = (album_col, asset_col) {
            chosen = Some((name.clone(), ac, sc));
            break;
        }
    }
    let Some((table, album_col, asset_col)) = chosen else {
        return Ok(HashMap::new());
    };

    // Table/column names come from the schema (not user input) → safe to interpolate.
    let sql = format!(
        "SELECT j.{asset_col}, a.ZTITLE FROM {table} j \
         JOIN ZGENERICALBUM a ON a.Z_PK = j.{album_col} \
         WHERE a.ZTITLE IS NOT NULL AND a.ZTITLE <> ''"
    );
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (pk, title) = row?;
        map.entry(pk).or_default().push(title);
    }
    for titles in map.values_mut() {
        titles.sort();
        titles.dedup();
    }
    Ok(map)
}

/// Parse the Camera Roll, ordered by capture date, enriched with album membership,
/// hidden/edited/Live flags, extra dates, and original name/title. Tolerates
/// missing optional columns and tables across iOS versions.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<Photo>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZASSET")?;
    let albums = album_membership(&conn)?;
    // Qualified ZASSET column, or NULL when absent (joined as alias `a`).
    let col = |n: &str| -> String { if cols.contains(n) { format!("a.{n}") } else { "NULL".into() } };
    let order = if cols.contains("ZDATECREATED") { "a.ZDATECREATED" } else { "a.Z_PK" };

    // Additional-attributes join (original filename + title), when present.
    let has_aa = cols.contains("ZADDITIONALATTRIBUTES")
        && table_exists(&conn, "ZADDITIONALASSETATTRIBUTES")?;
    let aa_cols = if has_aa {
        table_columns(&conn, "ZADDITIONALASSETATTRIBUTES")?
    } else {
        Default::default()
    };
    let orig_sel = if has_aa && aa_cols.contains("ZORIGINALFILENAME") { "aa.ZORIGINALFILENAME" } else { "NULL" };
    let title_sel = if has_aa && aa_cols.contains("ZTITLE") { "aa.ZTITLE" } else { "NULL" };
    let aa_join = if has_aa {
        "LEFT JOIN ZADDITIONALASSETATTRIBUTES aa ON aa.Z_PK = a.ZADDITIONALATTRIBUTES"
    } else {
        ""
    };

    let sql = format!(
        "SELECT a.Z_PK, a.ZFILENAME, a.ZDIRECTORY, {created}, {modif}, {added}, {kind}, {subtype}, \
         {fav}, {hidden}, {trash}, {edited}, {w}, {h}, {lat}, {lon}, {dur}, {avalanche}, {orig_sel}, {title_sel} \
         FROM ZASSET a {aa_join} ORDER BY {order}",
        created = col("ZDATECREATED"),
        modif = col("ZMODIFICATIONDATE"),
        added = col("ZADDEDDATE"),
        kind = col("ZKIND"),
        subtype = col("ZKINDSUBTYPE"),
        fav = col("ZFAVORITE"),
        hidden = col("ZHIDDEN"),
        trash = col("ZTRASHEDSTATE"),
        edited = col("ZHASADJUSTMENTS"),
        w = col("ZWIDTH"),
        h = col("ZHEIGHT"),
        lat = col("ZLATITUDE"),
        lon = col("ZLONGITUDE"),
        dur = col("ZDURATION"),
        avalanche = col("ZAVALANCHEUUID"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let pk: i64 = row.get(0)?;
        let filename: Option<String> = row.get(1)?;
        let directory: Option<String> = row.get(2)?;
        let created: Option<f64> = row.get(3)?;
        let modified: Option<f64> = row.get(4)?;
        let added: Option<f64> = row.get(5)?;
        let kind: Option<i64> = row.get(6)?;
        let subtype: Option<i64> = row.get(7)?;
        let favorite: Option<i64> = row.get(8)?;
        let hidden: Option<i64> = row.get(9)?;
        let trashed: Option<i64> = row.get(10)?;
        let edited: Option<i64> = row.get(11)?;
        let width: Option<i64> = row.get(12)?;
        let height: Option<i64> = row.get(13)?;
        let lat: Option<f64> = row.get(14)?;
        let lon: Option<f64> = row.get(15)?;
        let dur: Option<f64> = row.get(16)?;
        let avalanche: Option<String> = row.get(17)?;
        let original_filename: Option<String> = row.get(18)?;
        let title: Option<String> = row.get(19)?;

        let filename = filename.unwrap_or_default();
        let directory = directory.unwrap_or_default();
        let source_path = if filename.is_empty() {
            String::new()
        } else if directory.is_empty() {
            format!("Media/{filename}")
        } else {
            format!("Media/{directory}/{filename}")
        };
        let kind = match kind {
            Some(0) => "image",
            Some(1) => "video",
            _ => "unknown",
        };
        Ok(Photo {
            filename: basename(&filename).to_string(),
            kind: kind.to_string(),
            created: created.and_then(cocoa_to_iso).unwrap_or_default(),
            modified: modified.and_then(cocoa_to_iso).unwrap_or_default(),
            added: added.and_then(cocoa_to_iso).unwrap_or_default(),
            favorite: favorite == Some(1),
            hidden: hidden.unwrap_or(0) != 0,
            trashed: trashed.unwrap_or(0) != 0,
            edited: edited.unwrap_or(0) != 0,
            live_photo: kind == "image" && subtype == Some(2),
            kind_subtype: subtype,
            width: width.unwrap_or(0),
            height: height.unwrap_or(0),
            latitude: valid_coord(lat, 90.0),
            longitude: valid_coord(lon, 180.0),
            duration_seconds: if kind == "video" { dur.map(|d| d.round() as i64) } else { None },
            burst_id: avalanche.filter(|s| !s.is_empty()),
            original_filename: original_filename.unwrap_or_default(),
            title: title.unwrap_or_default(),
            albums: albums.get(&pk).cloned().unwrap_or_default(),
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
        assert_eq!(img.modified, "2020-01-06T10:40:50+00:00");
        assert_eq!(img.added, "2020-01-06T10:40:10+00:00");
        assert!(img.favorite);
        assert!(img.hidden); // ZHIDDEN = 1
        assert!(!img.trashed);
        assert!(img.edited); // ZHASADJUSTMENTS = 1
        assert!(img.live_photo); // image + ZKINDSUBTYPE = 2
        assert_eq!(img.kind_subtype, Some(2));
        assert_eq!(img.width, 4032);
        assert_eq!(img.latitude, Some(50.087));
        assert_eq!(img.longitude, Some(14.42));
        assert_eq!(img.duration_seconds, None);
        assert_eq!(img.burst_id, None);
        assert_eq!(img.original_filename, "IMG_E0001.HEIC");
        assert_eq!(img.title, "Západ slunce");
        assert_eq!(img.albums, vec!["Dovolená".to_string(), "Rodina".to_string()]); // sorted; NULL-title album excluded
        assert_eq!(img.source_path, "Media/DCIM/100APPLE/IMG_0001.HEIC");
        assert_eq!(img.file, None);

        let vid = &assets[1];
        assert_eq!(vid.kind, "video");
        assert!(!vid.live_photo); // a video is never a Live Photo
        assert_eq!(vid.duration_seconds, Some(13)); // 12.5 rounded
        assert_eq!(vid.latitude, None); // -180 sentinel → None
        assert_eq!(vid.longitude, None);
        assert_eq!(vid.burst_id.as_deref(), Some("BURST1"));
        assert!(vid.albums.is_empty());

        let trashed = &assets[2];
        assert!(trashed.trashed);
        assert_eq!(trashed.latitude, None); // NULL → None
        assert_eq!(trashed.burst_id.as_deref(), Some("BURST1")); // same burst as the video
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_minimal_zasset_without_enriched_columns() {
        let dir = std::env::temp_dir().join(format!("be-photos-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Photos.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        // No enriched columns, no album/attribute tables → all new fields default.
        conn.execute_batch(
            "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZFILENAME TEXT, ZDIRECTORY TEXT, ZKIND INTEGER);
             INSERT INTO ZASSET VALUES (1, 'IMG_9.JPG', 'DCIM/100APPLE', 0);",
        )
        .unwrap();
        drop(conn);

        let assets = parse(&db).unwrap();
        assert_eq!(assets.len(), 1);
        let a = &assets[0];
        assert_eq!(a.filename, "IMG_9.JPG");
        assert!(!a.hidden && !a.edited && !a.live_photo);
        assert_eq!(a.kind_subtype, None);
        assert_eq!(a.modified, "");
        assert_eq!(a.original_filename, "");
        assert_eq!(a.title, "");
        assert!(a.burst_id.is_none());
        assert!(a.albums.is_empty());
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

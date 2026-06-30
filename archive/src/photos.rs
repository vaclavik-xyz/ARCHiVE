//! Read Camera Roll metadata from `Photos.sqlite` (`ZASSET` on iOS 13+,
//! `ZGENERICASSET` on iOS ≤12) and extract the photo/video files from
//! `CameraRollDomain`.

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
    /// When the asset was moved to Recently Deleted as ISO 8601 UTC
    /// (`ZTRASHEDDATE`); empty when not trashed / unset.
    pub trashed_date: String,
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
    /// True when `file` is a reduced-quality thumbnail extracted as a fallback
    /// because the full-resolution original is not in the backup (e.g. iCloud
    /// Shared Album items or iCloud-optimized originals, which keep only a
    /// thumbnail on-device). Always false for full-resolution originals.
    pub file_is_thumbnail: bool,
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

/// The Camera Roll asset table name: `ZASSET` on iOS 13+, `ZGENERICASSET` on
/// iOS ≤12 (Apple renamed it). Columns are otherwise compatible, so resolving
/// the table name is all that's needed to read either schema. Prefers `ZASSET`
/// when present and falls back to the historical name (so a backup missing both
/// still fails with the familiar `no such table: ZASSET`).
fn asset_table_name(conn: &Connection) -> rusqlite::Result<String> {
    for name in ["ZASSET", "ZGENERICASSET"] {
        if table_exists(conn, name)? {
            return Ok(name.to_string());
        }
    }
    Ok("ZASSET".to_string())
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
    let asset_table = asset_table_name(&conn)?;
    let cols = table_columns(&conn, &asset_table)?;
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
         {fav}, {hidden}, {trash}, {edited}, {w}, {h}, {lat}, {lon}, {dur}, {avalanche}, {orig_sel}, {title_sel}, {tdate} \
         FROM {asset_table} a {aa_join} ORDER BY {order}",
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
        tdate = col("ZTRASHEDDATE"),
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
        let trashed_date: Option<f64> = row.get(20)?;

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
            trashed_date: trashed_date.and_then(cocoa_to_iso).unwrap_or_default(),
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
            file_is_thumbnail: false,
        })
    })?;
    rows.collect()
}

/// Per-run extraction outcome, surfaced in the JSON envelope.
pub struct PhotoSummary {
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Full-resolution originals written.
    pub extracted: usize,
    /// Reduced-quality thumbnails written as a fallback because the original was
    /// not in the backup (written under `<dir>/thumbnails/`).
    pub thumbnails: usize,
    /// Assets with no file at all in the backup (no original and no thumbnail).
    pub missing: usize,
}

/// Image file extensions a thumbnail can have (lowercase, no dot).
const THUMB_IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "heic"];

/// The backup-relative directory holding an asset's on-device thumbnails (the
/// iOS ≤12 `Thumbnails/V2` store), derived from its `source_path`
/// (`Media/<dir>/<file>` → `Media/PhotoData/Thumbnails/V2/<dir>/<file>/`). iOS
/// stores each asset's thumbnails in a directory named after the asset, holding
/// one or more size-coded JPEGs. `None` for an empty or non-`Media/` path.
fn thumbnail_prefix(source_path: &str) -> Option<String> {
    let rel = source_path.strip_prefix("Media/")?;
    if rel.is_empty() {
        return None;
    }
    Some(format!("Media/PhotoData/Thumbnails/V2/{rel}/"))
}

/// Choose the best thumbnail from a directory listing of backup-relative paths:
/// the highest size-coded image. Size codes are fixed-width zero-padded numbers
/// (e.g. `5003`, `5005`), so the lexically greatest image basename is the
/// largest rendition. `None` when no image-typed candidate is present.
fn pick_thumbnail(candidates: &[String]) -> Option<&str> {
    candidates
        .iter()
        .filter(|p| {
            let ext = basename(p).rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            THUMB_IMAGE_EXTS.contains(&ext.as_str())
        })
        .max_by(|a, b| basename(a).cmp(basename(b)))
        .map(|s| s.as_str())
}

/// Output filename for a thumbnail fallback: the asset's index and stem with the
/// thumbnail's real extension (`770_IMG_0015.jpg`), so a video poster is not
/// mislabeled with the video's `.MOV` extension.
fn thumbnail_output_name(n: usize, asset_filename: &str, chosen: &str) -> String {
    let ext = basename(chosen).rsplit('.').next().filter(|e| !e.is_empty()).unwrap_or("jpg").to_ascii_lowercase();
    let base = basename(asset_filename);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    format!("{n}_{stem}.{ext}")
}

/// Tally extraction outcomes: full originals, thumbnail fallbacks, and assets
/// with no file at all.
fn summarize(items: &[Photo], dir: &str) -> PhotoSummary {
    let thumbnails = items.iter().filter(|p| p.file_is_thumbnail).count();
    let extracted = items.iter().filter(|p| p.file.is_some() && !p.file_is_thumbnail).count();
    let missing = items.iter().filter(|p| p.file.is_none()).count();
    PhotoSummary { dir: dir.to_string(), extracted, thumbnails, missing }
}

/// Subdirectory (under the export dir) that receives the media files.
const PHOTO_DIR: &str = "photos";

/// Output filename `<n>_<filename>` (1-based index ensures uniqueness across
/// directories while preserving the original name).
pub(crate) fn output_name(n: usize, filename: &str) -> String {
    format!("{n}_{}", basename(filename))
}

/// Fetch each asset's file into `<out>/photos/`, filling `file` in place.
pub fn extract_photos(
    backup: &archive_core::Backup,
    items: &mut [Photo],
    out: &Path,
) -> std::io::Result<PhotoSummary> {
    extract_into(backup, items, out, PHOTO_DIR)
}

/// Fetch each asset's file into `<out>/<subdir>/`, filling `file` in place.
/// Best-effort: an asset absent from the backup is counted `missing`. Only
/// directory creation is fatal. Shared by `photos` and `photos-recently-deleted`.
pub fn extract_into(
    backup: &archive_core::Backup,
    items: &mut [Photo],
    out: &Path,
    subdir: &str,
) -> std::io::Result<PhotoSummary> {
    let media_dir = out.join(subdir);
    std::fs::create_dir_all(&media_dir)?;
    let thumb_dir = media_dir.join("thumbnails");

    for (i, item) in items.iter_mut().enumerate() {
        if item.source_path.is_empty() {
            continue;
        }
        let name = output_name(i + 1, &item.filename);
        let dest = media_dir.join(&name);
        match backup.fetch("CameraRollDomain", &item.source_path, &dest) {
            Ok(Some(_)) => item.file = Some(format!("{subdir}/{name}")),
            // Original not in the backup: fall back to the on-device thumbnail,
            // if one is present (iCloud Shared Album / iCloud-optimized assets).
            Ok(None) => {
                if let Some(out_name) = extract_thumbnail(backup, &item.source_path, &item.filename, i + 1, &thumb_dir) {
                    item.file = Some(format!("{subdir}/thumbnails/{out_name}"));
                    item.file_is_thumbnail = true;
                }
            }
            Err(why) => eprintln!("photo {}: fetch failed: {why}", item.filename),
        }
    }

    Ok(summarize(items, subdir))
}

/// Backup-relative prefix of the iOS ≤12 thumbnail store.
const THUMBNAIL_STORE: &str = "Media/PhotoData/Thumbnails/V2/";

/// Whether a path is an image by extension (for thumbnail candidates).
fn is_thumb_image(path: &str) -> bool {
    let ext = basename(path).rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    THUMB_IMAGE_EXTS.contains(&ext.as_str())
}

/// Classify each asset's availability in the backup WITHOUT copying any files:
/// `(originals, thumbnails, missing)` — how many full-resolution originals are
/// present, how many would fall back to a thumbnail, and how many have neither.
/// The cheap counterpart of [`extract_into`], for the `--summary` report. Scans
/// the manifest once into in-memory sets, then probes each asset in O(1) (a
/// per-asset `Backup::has`/`list` would rescan the whole manifest each call).
pub fn availability(backup: &archive_core::Backup, items: &[Photo]) -> (usize, usize, usize) {
    use std::collections::HashSet;
    let present: HashSet<String> =
        backup.list("CameraRollDomain", "").unwrap_or_default().into_iter().collect();
    // Thumbnail directories (kept with a trailing '/') that hold ≥1 image file.
    let thumb_dirs: HashSet<String> = present
        .iter()
        .filter(|p| p.starts_with(THUMBNAIL_STORE) && is_thumb_image(p))
        .filter_map(|p| p.rsplit_once('/').map(|(dir, _)| format!("{dir}/")))
        .collect();

    let (mut originals, mut thumbnails, mut missing) = (0, 0, 0);
    for item in items {
        if item.source_path.is_empty() {
            missing += 1;
        } else if present.contains(&item.source_path) {
            originals += 1;
        } else if thumbnail_prefix(&item.source_path).is_some_and(|p| thumb_dirs.contains(&p)) {
            thumbnails += 1;
        } else {
            missing += 1;
        }
    }
    (originals, thumbnails, missing)
}

/// Fetch an asset's best thumbnail into `thumb_dir` when the full-resolution
/// original is missing from the backup. Returns the written output filename on
/// success. Best-effort: a missing thumbnail directory or any list/fetch failure
/// yields `None` (the asset stays counted as missing).
fn extract_thumbnail(
    backup: &archive_core::Backup,
    source_path: &str,
    asset_filename: &str,
    n: usize,
    thumb_dir: &Path,
) -> Option<String> {
    let prefix = thumbnail_prefix(source_path)?;
    let candidates = backup.list("CameraRollDomain", &prefix).ok()?;
    let chosen = pick_thumbnail(&candidates)?;
    let out_name = thumbnail_output_name(n, asset_filename, chosen);
    std::fs::create_dir_all(thumb_dir).ok()?;
    let dest = thumb_dir.join(&out_name);
    match backup.fetch("CameraRollDomain", chosen, &dest) {
        Ok(Some(_)) => Some(out_name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_photos;

    /// Minimal default `Photo` with the given filename, for tally/selection tests.
    fn make_one(filename: &str) -> Photo {
        Photo {
            filename: filename.into(),
            kind: "image".into(),
            created: String::new(),
            modified: String::new(),
            added: String::new(),
            favorite: false,
            hidden: false,
            trashed: false,
            trashed_date: String::new(),
            edited: false,
            live_photo: false,
            kind_subtype: None,
            width: 0,
            height: 0,
            latitude: None,
            longitude: None,
            duration_seconds: None,
            burst_id: None,
            original_filename: String::new(),
            title: String::new(),
            albums: Vec::new(),
            source_path: String::new(),
            file: None,
            file_is_thumbnail: false,
        }
    }

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
        assert_eq!(trashed.trashed_date, "2020-01-06T10:45:00+00:00"); // Cocoa 600_000_300
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
        assert!(!a.trashed);
        assert_eq!(a.trashed_date, "");
        assert_eq!(a.kind_subtype, None);
        assert_eq!(a.modified, "");
        assert_eq!(a.original_filename, "");
        assert_eq!(a.title, "");
        assert!(a.burst_id.is_none());
        assert!(a.albums.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_ios12_zgenericasset_table() {
        // iOS ≤12 names the Camera Roll asset table ZGENERICASSET; iOS 13
        // renamed it to ZASSET. The columns are otherwise compatible, so the
        // extractor must resolve whichever table the backup actually has.
        let dir = std::env::temp_dir().join(format!("be-photos-gen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Photos.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZGENERICASSET (Z_PK INTEGER PRIMARY KEY, ZFILENAME TEXT, ZDIRECTORY TEXT, ZKIND INTEGER, ZDATECREATED REAL);
             INSERT INTO ZGENERICASSET VALUES (1, 'IMG_5301.JPG', 'DCIM/105APPLE', 0, 600000000.0);
             INSERT INTO ZGENERICASSET VALUES (2, 'IMG_5350.MOV', 'DCIM/105APPLE', 1, 600000100.0);",
        )
        .unwrap();
        drop(conn);

        let assets = parse(&db).unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].filename, "IMG_5301.JPG");
        assert_eq!(assets[0].kind, "image");
        assert_eq!(assets[0].source_path, "Media/DCIM/105APPLE/IMG_5301.JPG");
        assert_eq!(assets[1].kind, "video");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn output_name_is_unique_and_keeps_original() {
        assert_eq!(output_name(1, "IMG_0001.HEIC"), "1_IMG_0001.HEIC");
        assert_eq!(output_name(2, "a/b/IMG_0002.MOV"), "2_IMG_0002.MOV");
        assert_ne!(output_name(1, "x.jpg"), output_name(2, "x.jpg"));
    }

    #[test]
    fn thumbnail_prefix_derives_v2_directory() {
        assert_eq!(
            thumbnail_prefix("Media/PhotoData/PhotoCloudSharingData/16/UUID/100CLOUD/IMG_0017.JPG").as_deref(),
            Some("Media/PhotoData/Thumbnails/V2/PhotoData/PhotoCloudSharingData/16/UUID/100CLOUD/IMG_0017.JPG/"),
        );
        assert_eq!(
            thumbnail_prefix("Media/DCIM/100APPLE/IMG_0001.HEIC").as_deref(),
            Some("Media/PhotoData/Thumbnails/V2/DCIM/100APPLE/IMG_0001.HEIC/"),
        );
        assert_eq!(thumbnail_prefix(""), None);
        assert_eq!(thumbnail_prefix("Media/"), None);
    }

    #[test]
    fn pick_thumbnail_takes_largest_image_and_ignores_non_images() {
        let dir = "Media/PhotoData/Thumbnails/V2/DCIM/100APPLE/IMG_0001.HEIC";
        let candidates = vec![
            format!("{dir}/5003.JPG"),
            format!("{dir}/5005.JPG"),
            format!("{dir}/5000.AAE"), // sidecar, not an image
        ];
        assert_eq!(pick_thumbnail(&candidates), Some(format!("{dir}/5005.JPG").as_str()));
        // No image-typed entry → None.
        assert_eq!(pick_thumbnail(&[format!("{dir}/info.plist")]), None);
        assert_eq!(pick_thumbnail(&[]), None);
    }

    #[test]
    fn thumbnail_output_name_uses_thumbnail_extension() {
        // Image asset: stem + thumbnail's jpg extension.
        assert_eq!(thumbnail_output_name(770, "IMG_0017.JPG", "x/y/5005.JPG"), "770_IMG_0017.jpg");
        // Video asset: a JPEG poster must not keep the .MOV extension.
        assert_eq!(thumbnail_output_name(42, "IMG_0015.MOV", "x/y/5005.JPG"), "42_IMG_0015.jpg");
    }

    #[test]
    fn summarize_counts_originals_thumbnails_and_missing() {
        let mut orig = make_one("a.jpg");
        orig.file = Some("photos/1_a.jpg".into());
        let mut thumb = make_one("b.jpg");
        thumb.file = Some("photos/thumbnails/2_b.jpg".into());
        thumb.file_is_thumbnail = true;
        let missing = make_one("c.jpg"); // file None
        let s = summarize(&[orig, thumb, missing], "photos");
        assert_eq!(s.extracted, 1);
        assert_eq!(s.thumbnails, 1);
        assert_eq!(s.missing, 1);
        assert_eq!(s.dir, "photos");
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
        assert_eq!(summary.extracted + summary.thumbnails + summary.missing, items.len());
        for v in items.iter().filter_map(|p| p.file.as_ref()) {
            let p = out.join(v);
            assert!(p.is_file(), "linked file should exist: {}", p.display());
        }
    }
}

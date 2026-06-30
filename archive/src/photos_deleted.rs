//! Recently Deleted (trashed) Camera Roll assets — a recovery-focused view over
//! `Photos.sqlite`. iOS keeps deleted photos/videos in the "Recently Deleted"
//! album for ~30 days before purging them, so a backup taken inside that window
//! still contains the original files. This module isolates those assets and
//! estimates when each would have been permanently removed.

use chrono::DateTime;
use serde::Serialize;

use crate::datetime::unix_to_iso;
use crate::photos::Photo;

/// Days iOS keeps a trashed asset in Recently Deleted before purging it.
const RETENTION_DAYS: i64 = 30;

/// Subdirectory (under the export dir) that receives recovered files.
pub const DELETED_DIR: &str = "recently-deleted";

/// A trashed Camera Roll asset plus its estimated permanent-deletion date.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeletedAsset {
    /// The underlying asset (filename, kind, dates, GPS, extracted `file`, …).
    #[serde(flatten)]
    pub photo: Photo,
    /// Estimated permanent-deletion time (`trashed_date` + 30 days) as ISO 8601
    /// UTC; empty when the trashed date is unknown.
    pub purge_after: String,
}

/// Estimated purge date: the trashed ISO timestamp advanced by the 30-day
/// retention window. Empty in / empty out (and on any unparseable input).
pub fn purge_after(trashed_date: &str) -> String {
    if trashed_date.is_empty() {
        return String::new();
    }
    match DateTime::parse_from_rfc3339(trashed_date) {
        Ok(dt) => unix_to_iso(dt.timestamp() + RETENTION_DAYS * 86_400).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Keep only trashed assets (those in Recently Deleted), preserving order.
pub fn filter_trashed(items: Vec<Photo>) -> Vec<Photo> {
    items.into_iter().filter(|p| p.trashed).collect()
}

/// Wrap trashed photos with their estimated purge date.
pub fn into_deleted(items: Vec<Photo>) -> Vec<DeletedAsset> {
    items
        .into_iter()
        .map(|p| {
            let purge_after = purge_after(&p.trashed_date);
            DeletedAsset { photo: p, purge_after }
        })
        .collect()
}

/// Build a customer-facing summary of the recovered recently-deleted assets.
pub fn summary(items: &[DeletedAsset]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};

    let img = items.iter().filter(|d| d.photo.kind == "image").count();
    let vid = items.iter().filter(|d| d.photo.kind == "video").count();
    let other = items.len() - img - vid;
    let full_quality = items
        .iter()
        .filter(|d| d.photo.file.is_some() && !d.photo.file_is_thumbnail)
        .count();
    let thumbnail = items.iter().filter(|d| d.photo.file_is_thumbnail).count();
    let missing = items.iter().filter(|d| d.photo.file.is_none()).count();
    let with_gps = items
        .iter()
        .filter(|d| d.photo.latitude.is_some() && d.photo.longitude.is_some())
        .count();
    let favorites = items.iter().filter(|d| d.photo.favorite).count();
    let in_album = items.iter().filter(|d| !d.photo.albums.is_empty()).count();

    // Manual rows (occurrence counts over three fixed buckets); drop empties so
    // an all-photos backup shows no spurious "Videa"/"Ostatní" lines.
    let mut by_type: Vec<(String, usize)> = Vec::new();
    for (label, n) in [("Fotky", img), ("Videa", vid), ("Ostatní", other)] {
        if n > 0 {
            by_type.push((label.to_string(), n));
        }
    }
    let albums: Vec<(String, usize)> =
        tally(items.iter().flat_map(|d| d.photo.albums.iter().cloned()))
            .into_iter()
            .take(15)
            .collect();

    Summary::new("photos-recently-deleted", "Nedávno smazané fotky", "obnovitelných položek", items.len())
        .count("Fotek", img)
        .count("Videí", vid)
        .count("V plné kvalitě", full_quality)
        .count("Jen náhled", thumbnail)
        .count("Chybí v záloze", missing)
        .count("S GPS", with_gps)
        .count("Oblíbených", favorites)
        .count("Bylo v albu", in_album)
        .period_from(iso_range(items.iter().map(|d| d.photo.trashed_date.as_str())))
        .breakdown("Podle typu", by_type)
        .breakdown("Rok smazání", year_rows(items.iter().map(|d| d.photo.trashed_date.as_str())))
        .breakdown("Rok pořízení", year_rows(items.iter().map(|d| d.photo.created.as_str())))
        .breakdown("Alba", albums)
        .note("iOS koš maže po ~30 dnech — obnovitelné jen v tomto okně.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(filename: &str, trashed: bool, trashed_date: &str) -> Photo {
        Photo {
            filename: filename.to_string(),
            kind: "image".to_string(),
            created: String::new(),
            modified: String::new(),
            added: String::new(),
            favorite: false,
            hidden: false,
            trashed,
            trashed_date: trashed_date.to_string(),
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
    fn purge_after_adds_thirty_days() {
        assert_eq!(
            purge_after("2020-01-06T10:45:00+00:00"),
            "2020-02-05T10:45:00+00:00",
        );
    }

    #[test]
    fn purge_after_empty_or_invalid_is_empty() {
        assert_eq!(purge_after(""), "");
        assert_eq!(purge_after("not a date"), "");
    }

    #[test]
    fn filter_keeps_only_trashed() {
        let items = vec![
            photo("a.jpg", false, ""),
            photo("b.jpg", true, "2020-01-06T10:45:00+00:00"),
            photo("c.jpg", false, ""),
        ];
        let trashed = filter_trashed(items);
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].filename, "b.jpg");
    }

    #[test]
    fn into_deleted_computes_purge_date() {
        let deleted = into_deleted(vec![photo("b.jpg", true, "2020-01-06T10:45:00+00:00")]);
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].purge_after, "2020-02-05T10:45:00+00:00");
        assert_eq!(deleted[0].photo.filename, "b.jpg");
    }

    #[test]
    fn into_deleted_without_trashed_date_has_empty_purge() {
        let deleted = into_deleted(vec![photo("b.jpg", true, "")]);
        assert_eq!(deleted[0].purge_after, "");
    }

    #[allow(clippy::too_many_arguments)]
    fn asset(
        kind: &str,
        created: &str,
        trashed_date: &str,
        file: Option<&str>,
        thumb: bool,
        gps: bool,
        favorite: bool,
        albums: &[&str],
    ) -> DeletedAsset {
        let mut p = photo("x.jpg", true, trashed_date);
        p.kind = kind.to_string();
        p.created = created.to_string();
        p.file = file.map(String::from);
        p.file_is_thumbnail = thumb;
        if gps {
            p.latitude = Some(50.08);
            p.longitude = Some(14.42);
        }
        p.favorite = favorite;
        p.albums = albums.iter().map(|a| a.to_string()).collect();
        let purge_after = purge_after(&p.trashed_date);
        DeletedAsset { photo: p, purge_after }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let items = vec![
            asset("image", "2022-03-01T10:00:00+00:00", "2024-01-06T10:45:00+00:00", Some("photos/a.jpg"), false, true, true, &["Dovolená"]),
            asset("image", "2023-07-01T10:00:00+00:00", "2024-02-06T10:45:00+00:00", None, false, false, false, &["Dovolená"]),
            asset("video", "2021-05-01T10:00:00+00:00", "2025-03-06T10:45:00+00:00", Some("photos/b.mov"), true, false, false, &[]),
        ];
        let s = summary(&items);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "obnovitelných položek");

        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Fotek"), Some(2));
        assert_eq!(get("Videí"), Some(1));
        assert_eq!(get("V plné kvalitě"), Some(1)); // a.jpg: present and not a thumbnail
        assert_eq!(get("Jen náhled"), Some(1)); // b.mov: thumbnail fallback
        assert_eq!(get("Chybí v záloze"), Some(1)); // second image: no file
        assert_eq!(get("Bylo v albu"), Some(2));

        let by_type = s.breakdowns.iter().find(|b| b.title == "Podle typu").unwrap();
        assert_eq!(by_type.rows, vec![("Fotky".to_string(), 2), ("Videa".to_string(), 1)]);
        let albums = s.breakdowns.iter().find(|b| b.title == "Alba").unwrap();
        assert_eq!(albums.rows[0], ("Dovolená".to_string(), 2));

        assert!(s.period.is_some()); // derived from the trashed dates
    }

    #[test]
    fn json_flattens_photo_fields() {
        let deleted = into_deleted(vec![photo("b.jpg", true, "2020-01-06T10:45:00+00:00")]);
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&deleted).unwrap()).unwrap();
        // `photo` is flattened: filename sits at the top level, not nested.
        assert_eq!(v[0]["filename"], "b.jpg");
        assert_eq!(v[0]["purge_after"], "2020-02-05T10:45:00+00:00");
        assert!(v[0].get("photo").is_none());
    }
}

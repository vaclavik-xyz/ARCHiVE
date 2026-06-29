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

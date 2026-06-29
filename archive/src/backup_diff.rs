//! Diff two backups at the file level: which manifest files were **added**,
//! **removed**, or **modified** (logical size changed) between an older backup A
//! (`--backup`) and a newer backup B (`--against`). Keyed by `(domain,
//! relative_path)`; the size comes from each manifest, so the comparison works for
//! encrypted backups too (it never compares on-disk ciphertext lengths). This is a
//! structural diff — it flags that a file's content changed (its size differs), not
//! what changed inside it. Read-only on both backups.

use std::collections::HashMap;

use archive_core::FileEntry;
use serde::Serialize;

/// One file-level difference between the two backups.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileChange {
    /// `added` (only in B) | `removed` (only in A) | `modified` (size differs).
    pub change: &'static str,
    pub domain: String,
    pub path: String,
    /// Size in backup A, or `None` when the file was added.
    pub size_a: Option<u64>,
    /// Size in backup B, or `None` when the file was removed.
    pub size_b: Option<u64>,
}

/// Roll-up counts for the diff.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffSummary {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub unchanged: usize,
}

/// Compare backup A's file entries against backup B's, keyed by `(domain,
/// relative_path)`. Returns the roll-up summary and the sorted list of changes
/// (added/removed/modified only — unchanged files are counted, not listed).
pub fn diff(a: &[FileEntry], b: &[FileEntry]) -> (DiffSummary, Vec<FileChange>) {
    let map_a: HashMap<(&str, &str), u64> =
        a.iter().map(|e| ((e.domain.as_str(), e.relative_path.as_str()), e.size)).collect();
    let map_b: HashMap<(&str, &str), u64> =
        b.iter().map(|e| ((e.domain.as_str(), e.relative_path.as_str()), e.size)).collect();

    let mut changes: Vec<FileChange> = Vec::new();
    let mut unchanged = 0usize;
    for (&(domain, path), &sa) in &map_a {
        match map_b.get(&(domain, path)) {
            None => changes.push(FileChange {
                change: "removed",
                domain: domain.into(),
                path: path.into(),
                size_a: Some(sa),
                size_b: None,
            }),
            Some(&sb) if sb != sa => changes.push(FileChange {
                change: "modified",
                domain: domain.into(),
                path: path.into(),
                size_a: Some(sa),
                size_b: Some(sb),
            }),
            Some(_) => unchanged += 1,
        }
    }
    for (&(domain, path), &sb) in &map_b {
        if !map_a.contains_key(&(domain, path)) {
            changes.push(FileChange {
                change: "added",
                domain: domain.into(),
                path: path.into(),
                size_a: None,
                size_b: Some(sb),
            });
        }
    }
    // Stable output: group by change kind, then domain, then path.
    changes.sort_by(|x, y| {
        (x.change, &x.domain, &x.path).cmp(&(y.change, &y.domain, &y.path))
    });

    let count = |kind: &str| changes.iter().filter(|c| c.change == kind).count();
    let summary = DiffSummary { added: count("added"), removed: count("removed"), modified: count("modified"), unchanged };
    (summary, changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fe(domain: &str, path: &str, size: u64) -> FileEntry {
        FileEntry { domain: domain.into(), relative_path: path.into(), size }
    }

    #[test]
    fn detects_added_removed_modified_and_unchanged() {
        let a = vec![
            fe("HomeDomain", "Library/SMS/sms.db", 100),  // modified (size changes)
            fe("HomeDomain", "Library/keep.db", 50),      // unchanged
            fe("HomeDomain", "Library/gone.db", 10),      // removed
        ];
        let b = vec![
            fe("HomeDomain", "Library/SMS/sms.db", 140),  // grew
            fe("HomeDomain", "Library/keep.db", 50),      // same
            fe("CameraRollDomain", "Media/new.jpg", 999), // added
        ];
        let (summary, changes) = diff(&a, &b);
        assert_eq!(summary, DiffSummary { added: 1, removed: 1, modified: 1, unchanged: 1 });

        // Sorted: added, modified, removed.
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].change, "added");
        assert_eq!(changes[0].path, "Media/new.jpg");
        assert_eq!(changes[0].size_a, None);
        assert_eq!(changes[0].size_b, Some(999));
        assert_eq!(changes[1].change, "modified");
        assert_eq!(changes[1].size_a, Some(100));
        assert_eq!(changes[1].size_b, Some(140));
        assert_eq!(changes[2].change, "removed");
        assert_eq!(changes[2].size_b, None);
    }

    #[test]
    fn identical_backups_have_no_changes() {
        let a = vec![fe("HomeDomain", "a.db", 1), fe("HomeDomain", "b.db", 2)];
        let (summary, changes) = diff(&a, &a.clone());
        assert_eq!(summary, DiffSummary { added: 0, removed: 0, modified: 0, unchanged: 2 });
        assert!(changes.is_empty());
    }

    #[test]
    fn same_path_different_domain_is_not_confused() {
        // The same relative path under two domains are distinct files.
        let a = vec![fe("HomeDomain", "x.db", 1)];
        let b = vec![fe("CameraRollDomain", "x.db", 1)];
        let (summary, _) = diff(&a, &b);
        assert_eq!(summary, DiffSummary { added: 1, removed: 1, modified: 0, unchanged: 0 });
    }
}

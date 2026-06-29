//! Extract the document/media files of a named third-party app from its backup
//! container(s). Where a parser cannot help (the message database is excluded or
//! encrypted), the **files** an app stored — photos, videos, voice messages,
//! documents — are usually still present and recoverable. This walks the app's
//! `AppDomain-…` and any `AppDomainGroup-…` containers and copies matching files
//! out, preserving their relative layout.
//!
//! By default only media files are copied (the recovery-relevant content);
//! `--all` copies every file. This module owns the cheap, testable helpers
//! (classification, path safety); the fetch loop lives in the command layer.

use serde::Serialize;

/// One extracted file, recorded in the run manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtractedFile {
    /// Backup domain the file came from.
    pub domain: String,
    /// Relative path within the domain.
    pub path: String,
    /// File size in bytes.
    pub bytes: u64,
    /// `image` / `video` / `audio` / `other`.
    pub category: String,
    /// Output-relative path of the copied file.
    pub file: String,
}

/// Classify a file by extension into a coarse media category.
pub fn category(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "bmp" | "tiff" | "tif" => "image",
        "mp4" | "mov" | "m4v" | "3gp" | "avi" | "mkv" | "webm" => "video",
        "m4a" | "amr" | "aac" | "mp3" | "wav" | "caf" | "opus" | "ogg" | "ptt" => "audio",
        _ => "other",
    }
}

/// Whether a file is a media file (image / video / audio).
pub fn is_media(path: &str) -> bool {
    category(path) != "other"
}

/// Make a backup-relative path safe to join under an output directory: reject
/// absolute paths and any `..` traversal, normalise separators. Returns `None`
/// for an unsafe or empty path.
pub fn safe_relpath(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    // Treat backslash as a separator too, so a Windows-style `..\..\x` cannot
    // slip through as one opaque segment when this runs on (or processes a
    // manifest crafted for) Windows.
    let normalized = path.replace('\\', "/");
    // Reject rooted (`/…`) and drive-absolute (`C:\…` / `C:/…`) paths outright,
    // per the sanitizer contract — backup manifest paths are domain-relative.
    if normalized.starts_with('/') {
        return None;
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut parts = Vec::new();
    for seg in normalized.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return None,
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Sanitise a backup domain into a single output-path segment (no slashes).
pub fn domain_dir(domain: &str) -> String {
    domain.replace(['/', '\\'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_by_extension() {
        assert_eq!(category("a/b/IMG_1.HEIC"), "image");
        assert_eq!(category("v.MP4"), "video");
        assert_eq!(category("Documents/VoiceMessages/x.ptt"), "audio");
        assert_eq!(category("db.sqlite"), "other");
        assert_eq!(category("noext"), "other");
        assert!(is_media("p.jpg"));
        assert!(!is_media("p.plist"));
    }

    #[test]
    fn safe_relpath_blocks_traversal_and_absolutes() {
        assert_eq!(safe_relpath("Documents/a/b.jpg"), Some("Documents/a/b.jpg".to_string()));
        assert_eq!(safe_relpath("/Documents/a.jpg"), None);
        assert_eq!(safe_relpath("a/./b.jpg"), Some("a/b.jpg".to_string()));
        assert_eq!(safe_relpath("../etc/passwd"), None);
        assert_eq!(safe_relpath("a/../../b"), None);
        assert_eq!(safe_relpath(""), None);
        assert_eq!(safe_relpath("/"), None);
        // Backslash separators and drive prefixes must not escape either.
        assert_eq!(safe_relpath("..\\etc\\passwd"), None);
        assert_eq!(safe_relpath("a\\..\\b"), None);
        assert_eq!(safe_relpath("C:\\Windows\\x"), None);
        assert_eq!(safe_relpath("Documents\\a\\b.jpg"), Some("Documents/a/b.jpg".to_string()));
    }

    #[test]
    fn domain_dir_has_no_separators() {
        assert_eq!(domain_dir("AppDomain-com.viber"), "AppDomain-com.viber");
        assert!(!domain_dir("AppDomainGroup-group.net.whatsapp.WhatsApp.shared").contains('/'));
    }
}

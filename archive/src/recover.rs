//! The `recover` capstone: the section model and the customer-facing
//! `index.html` landing-page renderer. Per-type orchestration lives in
//! `run_recover` (main.rs), which reuses every existing extractor.

use askama::Template;
use serde::Serialize;

/// Media-extraction outcome for a section that wrote files.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecoverMedia {
    /// Output-relative media directory.
    pub dir: String,
    /// Full-resolution originals written.
    pub extracted: usize,
    /// Reduced-quality thumbnail fallbacks written (under `<dir>/thumbnails/`);
    /// 0 for media types that have no thumbnail fallback. Counting these keeps
    /// `extracted + thumbnails + missing == count`.
    pub thumbnails: usize,
    /// Items with no file at all in the backup.
    pub missing: usize,
}

/// One recovered data type in the package. Serializes to the documented envelope
/// shape (`type`/`count`/`file`/`files?`) — `label` is HTML-only and the media
/// object is named `files` and omitted when absent, matching the other commands.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecoverSection {
    /// Machine type, e.g. `contacts`.
    #[serde(rename = "type")]
    pub data_type: String,
    /// Human label for the index (HTML only, not part of the JSON API).
    #[serde(skip)]
    pub label: String,
    /// HTML file written for this type, e.g. `contacts.html`.
    pub file: String,
    /// Item count.
    pub count: usize,
    /// Media folder details, when this type extracted files.
    #[serde(rename = "files", skip_serializing_if = "Option::is_none")]
    pub media: Option<RecoverMedia>,
}

#[derive(Template)]
#[template(path = "recover-index.html")]
struct IndexTemplate<'a> {
    name: &'a str,
    model: &'a str,
    ios: &'a str,
    serial: &'a str,
    udid: &'a str,
    generated: &'a str,
    sections: &'a [RecoverSection],
}

/// Render the customer-facing `index.html` landing page: a device sheet plus a
/// table linking every recovered section. All dynamic values are askama-escaped.
pub fn render_index(
    device: &archive_core::DeviceInfo,
    generated: &str,
    sections: &[RecoverSection],
) -> String {
    IndexTemplate {
        name: &device.device_name,
        model: &device.model,
        ios: &device.product_version,
        serial: &device.serial,
        udid: &device.udid,
        generated,
        sections,
    }
    .render()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> archive_core::DeviceInfo {
        archive_core::DeviceInfo {
            device_name: "Janin iPhone".into(),
            product_version: "17.5".into(),
            model: "iPhone14,2".into(),
            serial: "F2LabcXYZ".into(),
            udid: "00008110-000".into(),
        }
    }

    fn sections() -> Vec<RecoverSection> {
        vec![
            RecoverSection {
                data_type: "contacts".into(),
                label: "Kontakty".into(),
                file: "contacts.html".into(),
                count: 1234,
                media: None,
            },
            RecoverSection {
                data_type: "photos".into(),
                label: "Fotky".into(),
                file: "photos.html".into(),
                count: 1240,
                media: Some(RecoverMedia { dir: "photos".into(), extracted: 1236, thumbnails: 2, missing: 2 }),
            },
        ]
    }

    #[test]
    fn index_shows_device_sheet_and_sections() {
        let html = render_index(&device(), "2026-06-27T12:00:00+00:00", &sections());
        // Device sheet.
        assert!(html.contains("Janin iPhone"));
        assert!(html.contains("iPhone14,2"));
        assert!(html.contains("17.5"));
        assert!(html.contains("F2LabcXYZ"));
        // Sections: labels, counts, links, media folder.
        assert!(html.contains("Kontakty"));
        assert!(html.contains("href=\"contacts.html\""));
        assert!(html.contains("1234"));
        assert!(html.contains("href=\"photos.html\""));
        assert!(html.contains("href=\"photos\"") || html.contains("href=\"photos/\""));
        assert!(html.contains("1236"));
        assert!(html.contains("náhled")); // thumbnail count shown when > 0
        assert!(html.contains("2026-06-27T12:00:00+00:00"));
    }

    #[test]
    fn index_escapes_dynamic_values() {
        let mut d = device();
        d.device_name = "<script>alert(1)</script>".into();
        let html = render_index(&d, "2026-06-27T12:00:00+00:00", &sections());
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn section_serializes_to_documented_api_shape() {
        let v = serde_json::to_value(sections()).unwrap();
        // Non-media section: `type`/`count`/`file`, no `files`, no `label`/`data_type`.
        assert_eq!(v[0]["type"], "contacts");
        assert_eq!(v[0]["count"], 1234);
        assert_eq!(v[0]["file"], "contacts.html");
        assert!(v[0].get("files").is_none());
        assert!(v[0].get("label").is_none());
        assert!(v[0].get("data_type").is_none());
        // Media section: `files` object present.
        assert_eq!(v[1]["type"], "photos");
        assert_eq!(v[1]["files"]["extracted"], 1236);
        assert_eq!(v[1]["files"]["thumbnails"], 2);
        assert_eq!(v[1]["files"]["missing"], 2);
    }

    #[test]
    fn index_with_no_sections_still_renders_device() {
        let html = render_index(&device(), "2026-06-27T12:00:00+00:00", &[]);
        assert!(html.contains("Janin iPhone"));
        assert!(html.contains("<html"));
    }
}

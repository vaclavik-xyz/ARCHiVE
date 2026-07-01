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

/// Total media files recovered across every section (originals + thumbnails).
fn media_files(sections: &[RecoverSection]) -> usize {
    sections.iter().filter_map(|s| s.media.as_ref()).map(|m| m.extracted + m.thumbnails).sum()
}

/// The root unified summary as plain markdown (`summary.md`): a customer one-pager
/// covering every recovered type, built on the generic [`crate::summary`] model so
/// it shares the renderer and escaping with the per-type reports.
pub fn summary_md(device: &archive_core::DeviceInfo, generated: &str, sections: &[RecoverSection]) -> String {
    let total: usize = sections.iter().map(|s| s.count).sum();
    let rows: Vec<(String, usize)> = sections.iter().map(|s| (s.label.clone(), s.count)).collect();
    let s = crate::summary::Summary::new("recover", "Záloha", "položek", total)
        .count("Datových typů", sections.len())
        .count("Obnovených souborů (média)", media_files(sections))
        .breakdown("Obnoveno podle typu", rows)
        .note("Podrobnosti k jednotlivým typům najdete v index.html a ve stránkách <typ>.html.");
    crate::summary::summary_md(device, generated, &s)
}

/// One row of the root summary one-pager: a section's label, item count, and a
/// human files string (empty for non-media types).
struct SummaryRow {
    label: String,
    count: usize,
    files: String,
}

#[derive(Template)]
#[template(path = "recover-summary.html")]
struct SummaryTemplate<'a> {
    name: &'a str,
    model: String,
    ios: &'a str,
    generated: String,
    total: usize,
    media_files: usize,
    rows: Vec<SummaryRow>,
}

/// The root unified summary as HTML, for rendering `summary.pdf` (the polished
/// customer one-pager). Mirrors [`summary_md`] but escaped for HTML/PDF.
pub fn render_summary_html(device: &archive_core::DeviceInfo, generated: &str, sections: &[RecoverSection]) -> String {
    let rows = sections
        .iter()
        .map(|s| {
            let files = match &s.media {
                Some(m) => {
                    let mut f = format!("{} souborů", m.extracted);
                    if m.thumbnails > 0 {
                        f.push_str(&format!(" + {} náhledů", m.thumbnails));
                    }
                    if m.missing > 0 {
                        f.push_str(&format!(", {} chybí", m.missing));
                    }
                    f
                }
                None => String::new(),
            };
            SummaryRow { label: s.label.clone(), count: s.count, files }
        })
        .collect();
    SummaryTemplate {
        name: &device.device_name,
        model: crate::device_model::display_model(&device.model),
        ios: &device.product_version,
        generated: crate::format::cz_date(generated),
        total: sections.iter().map(|s| s.count).sum(),
        media_files: media_files(sections),
        rows,
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

    #[test]
    fn summary_md_lists_sections_and_grand_total() {
        let md = summary_md(&device(), "2026-06-27T12:00:00+00:00", &sections());
        assert!(md.contains("# Záloha — souhrn"));
        assert!(md.contains("Janin iPhone"));
        // grand total = 1234 + 1240
        assert!(md.contains("**Zachráněno:** 2474 položek"));
        assert!(md.contains("- Kontakty — 1234"));
        assert!(md.contains("- Fotky — 1240"));
        // media files = 1236 + 2 thumbnails
        assert!(md.contains("Obnovených souborů (média): 1238"));
    }

    #[test]
    fn summary_html_shows_sections_and_files_and_escapes() {
        let mut d = device();
        d.device_name = "<script>".into();
        let html = render_summary_html(&d, "2026-06-27T12:00:00+00:00", &sections());
        assert!(html.contains("Souhrn zálohy"));
        assert!(html.contains("Kontakty"));
        assert!(html.contains("1236 souborů + 2 náhledů, 2 chybí"));
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>"));
    }
}

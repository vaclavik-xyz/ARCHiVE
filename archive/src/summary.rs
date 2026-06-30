//! Generic, customer-facing recovery summaries.
//!
//! Every collection-bearing data type builds a [`Summary`] from the records it
//! already loaded (no extra IO, no media copy) and writes a plain-markdown
//! `<type>-summary.md` next to its main export, so a reader sees at a glance
//! what the backup holds. The `recover` capstone aggregates the same data into
//! one root overview. Markdown is deliberately dependency-free (no headless
//! Chrome), agent- and human-readable, and renders anywhere.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use crate::format::cz_date;

/// A named group of `(name, count)` rows within a summary (e.g. "Po letech").
pub struct Breakdown {
    pub title: String,
    pub rows: Vec<(String, usize)>,
}

/// A generic recovery summary for one data type, rendered to markdown.
pub struct Summary {
    /// Machine type, e.g. `calls`.
    pub data_type: String,
    /// Czech section title, e.g. "Hovory".
    pub title: String,
    /// Noun for the grand total, e.g. "hovorů".
    pub total_label: String,
    pub total: usize,
    /// Capture range as Czech "D. M. YYYY" `(from, to)`, when the type is temporal.
    pub period: Option<(String, String)>,
    /// Headline scalar counts.
    pub counts: Vec<(String, usize)>,
    /// Breakdown groups (by year, by contact, …).
    pub breakdowns: Vec<Breakdown>,
    /// Honesty caveats shown at the bottom.
    pub notes: Vec<String>,
}

impl Summary {
    /// Start a summary with its identity and grand total; everything else is
    /// added with the chainable builders below.
    pub fn new(data_type: &str, title: &str, total_label: &str, total: usize) -> Self {
        Summary {
            data_type: data_type.to_string(),
            title: title.to_string(),
            total_label: total_label.to_string(),
            total,
            period: None,
            counts: Vec::new(),
            breakdowns: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Add a headline count row.
    pub fn count(mut self, label: &str, n: usize) -> Self {
        self.counts.push((label.to_string(), n));
        self
    }

    /// Add a breakdown group. Empty `rows` are dropped (no heading is rendered).
    pub fn breakdown(mut self, title: &str, rows: Vec<(String, usize)>) -> Self {
        if !rows.is_empty() {
            self.breakdowns.push(Breakdown { title: title.to_string(), rows });
        }
        self
    }

    /// Set the period from an ISO `(from, to)` range (see [`iso_range`]); each
    /// bound is reformatted via [`cz_date`]. Skipped unless both reformat cleanly.
    pub fn period_from(mut self, range: Option<(String, String)>) -> Self {
        if let Some((from, to)) = range {
            let (from, to) = (cz_date(&from), cz_date(&to));
            if !from.is_empty() && !to.is_empty() {
                self.period = Some((from, to));
            }
        }
        self
    }

    /// Append an honesty note.
    pub fn note(mut self, text: &str) -> Self {
        self.notes.push(text.to_string());
        self
    }
}

/// Count occurrences of each key; rows sorted most-frequent first, ties by name.
pub fn tally<I: IntoIterator<Item = String>>(keys: I) -> Vec<(String, usize)> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for k in keys {
        *counts.entry(k).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Item counts bucketed by calendar year (the `YYYY` prefix of each ISO date),
/// oldest year first. Undated/garbage entries are skipped.
pub fn year_rows<'a, I: IntoIterator<Item = &'a str>>(dates: I) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut by_year: BTreeMap<i32, usize> = BTreeMap::new();
    for d in dates {
        if let Ok(y) = d.get(0..4).unwrap_or("").parse::<i32>() {
            *by_year.entry(y).or_default() += 1;
        }
    }
    by_year.into_iter().map(|(y, n)| (y.to_string(), n)).collect()
}

/// The chronological `(min, max)` of a set of ISO dates, skipping empties.
/// ISO-8601 sorts lexicographically, so string min/max is the date range.
pub fn iso_range<'a, I: IntoIterator<Item = &'a str>>(dates: I) -> Option<(String, String)> {
    let mut min: Option<&str> = None;
    let mut max: Option<&str> = None;
    for d in dates {
        if d.is_empty() {
            continue;
        }
        if min.is_none_or(|m| d < m) {
            min = Some(d);
        }
        if max.is_none_or(|m| d > m) {
            max = Some(d);
        }
    }
    match (min, max) {
        (Some(a), Some(b)) => Some((a.to_string(), b.to_string())),
        _ => None,
    }
}

/// Collapse whitespace and escape markdown metacharacters so an arbitrary
/// user-supplied name cannot corrupt list/heading structure.
fn md_inline(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(collapsed.len() + 4);
    for (i, c) in collapsed.chars().enumerate() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' => {
                out.push('\\');
                out.push(c);
            }
            '#' | '-' | '+' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Render a [`Summary`] as markdown. `generated_iso` is shown as a Czech date.
pub fn summary_md(device: &archive_core::DeviceInfo, generated_iso: &str, s: &Summary) -> String {
    let model = crate::device_model::display_model(&device.model);
    let mut out = String::new();
    let _ = writeln!(out, "# {} — souhrn", md_inline(&s.title));
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "**Zařízení:** {} — {}, iOS {}",
        md_inline(&device.device_name),
        md_inline(&model),
        md_inline(&device.product_version)
    );
    let _ = writeln!(out, "**Vytvořeno:** {}", cz_date(generated_iso));
    let _ = writeln!(out);
    let _ = writeln!(out, "**Zachráněno:** {} {}", s.total, s.total_label);
    if let Some((from, to)) = &s.period {
        let _ = writeln!(out, "**Období:** {from} – {to}");
    }
    if !s.counts.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Přehled");
        for (label, n) in &s.counts {
            let _ = writeln!(out, "- {}: {n}", md_inline(label));
        }
    }
    for bd in &s.breakdowns {
        let _ = writeln!(out);
        let _ = writeln!(out, "## {}", md_inline(&bd.title));
        for (name, n) in &bd.rows {
            let _ = writeln!(out, "- {} — {n}", md_inline(name));
        }
    }
    for note in &s.notes {
        let _ = writeln!(out);
        let _ = writeln!(out, "> {}", md_inline(note));
    }
    out
}

/// Write `<data_type>-summary.md` into `out`, recording its path in `outputs`.
pub fn write_summary_md(
    out: &Path,
    generated_iso: &str,
    device: &archive_core::DeviceInfo,
    s: &Summary,
    outputs: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let path = out.join(format!("{}-summary.md", s.data_type));
    std::fs::write(&path, summary_md(device, generated_iso, s))?;
    outputs.push(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> archive_core::DeviceInfo {
        archive_core::DeviceInfo {
            device_name: "Janin iPhone".into(),
            product_version: "17.5".into(),
            model: "iPhone14,2".into(),
            serial: "x".into(),
            udid: "u".into(),
        }
    }

    #[test]
    fn tally_sorts_count_desc_then_name() {
        let r = tally(["b", "a", "b", "a", "a"].into_iter().map(String::from));
        assert_eq!(r, vec![("a".to_string(), 3), ("b".to_string(), 2)]);
    }

    #[test]
    fn year_rows_oldest_first_skips_undated() {
        let r = year_rows(["2024-01-01T00:00:00Z", "2022-05-01", "", "bad"]);
        assert_eq!(r, vec![("2022".to_string(), 1), ("2024".to_string(), 1)]);
    }

    #[test]
    fn iso_range_min_max_skip_empty() {
        assert_eq!(
            iso_range(["2023-01-01", "", "2021-06-01", "2025-12-31"]),
            Some(("2021-06-01".to_string(), "2025-12-31".to_string()))
        );
        assert_eq!(iso_range(Vec::<&str>::new()), None);
    }

    #[test]
    fn summary_md_renders_sections_and_counts() {
        let s = Summary::new("calls", "Hovory", "hovorů", 10)
            .count("Příchozí", 6)
            .count("Odchozí", 4)
            .period_from(Some(("2022-01-02".into(), "2024-03-04".into())))
            .breakdown("Po letech", vec![("2022".into(), 5), ("2024".into(), 5)])
            .note("Historie hovorů je na zařízení omezená.");
        let md = summary_md(&dev(), "2026-07-01T00:00:00Z", &s);
        assert!(md.contains("# Hovory — souhrn"), "title heading: {md}");
        assert!(md.contains("Janin iPhone"));
        assert!(md.contains("**Zachráněno:** 10 hovorů"));
        assert!(md.contains("2. 1. 2022 – 4. 3. 2024"));
        assert!(md.contains("- Příchozí: 6"));
        assert!(md.contains("## Po letech"));
        assert!(md.contains("- 2024 — 5"));
        assert!(md.contains("> Historie hovorů"));
    }

    #[test]
    fn summary_md_sanitizes_names() {
        let s = Summary::new("x", "T", "x", 1).breakdown("B", vec![("a | b\nc".into(), 1)]);
        let md = summary_md(&dev(), "2026-07-01T00:00:00Z", &s);
        assert!(md.contains("a \\| b c — 1"), "sanitized: {md}");
        assert!(!md.contains("a | b"));
    }

    #[test]
    fn breakdown_skips_empty_rows() {
        let s = Summary::new("x", "T", "x", 0).breakdown("Empty", vec![]);
        assert!(s.breakdowns.is_empty());
    }
}

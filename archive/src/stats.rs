//! Aggregate statistics over the unified timeline: per-category event counts and
//! date ranges, plus a grand total and the backup's overall activity span. Like
//! [`crate::timeline`], this is a *view* over the extractors, not a new data store.
//!
//! All timestamps are ISO 8601 (RFC 3339) UTC strings, so they sort
//! lexicographically in chronological order — no parsing needed for min/max.

use serde::Serialize;

use crate::timeline::Event;

/// Statistics for one activity category (a timeline `kind`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CategoryStat {
    /// Timeline kind, e.g. `call`, `photo`, `whatsapp`.
    pub category: String,
    /// Number of events in this category (dated or not).
    pub count: usize,
    /// Earliest dated event, ISO 8601 UTC; empty when none are dated.
    pub earliest: String,
    /// Latest dated event, ISO 8601 UTC; empty when none are dated.
    pub latest: String,
}

/// The whole dashboard.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    /// Per-category rows, sorted by descending count then category name.
    pub categories: Vec<CategoryStat>,
    /// Total events across all categories.
    pub total_events: usize,
    /// Earliest dated event overall, ISO 8601 UTC; empty when none.
    pub earliest: String,
    /// Latest dated event overall, ISO 8601 UTC; empty when none.
    pub latest: String,
}

fn keep_min(slot: &mut String, v: &str) {
    if slot.is_empty() || v < slot.as_str() {
        *slot = v.to_string();
    }
}

fn keep_max(slot: &mut String, v: &str) {
    if v > slot.as_str() {
        *slot = v.to_string();
    }
}

/// Summarize timeline events into per-category counts and date ranges. Undated
/// events (empty timestamp) are counted but excluded from the ranges.
pub fn summarize(events: &[Event]) -> Stats {
    use std::collections::BTreeMap;

    // (count, earliest, latest) per kind; BTreeMap gives a deterministic base order.
    let mut groups: BTreeMap<&str, (usize, String, String)> = BTreeMap::new();
    let mut earliest = String::new();
    let mut latest = String::new();

    for e in events {
        let entry = groups.entry(e.kind.as_str()).or_insert((0, String::new(), String::new()));
        entry.0 += 1;
        if !e.timestamp.is_empty() {
            keep_min(&mut entry.1, &e.timestamp);
            keep_max(&mut entry.2, &e.timestamp);
            keep_min(&mut earliest, &e.timestamp);
            keep_max(&mut latest, &e.timestamp);
        }
    }

    let mut categories: Vec<CategoryStat> = groups
        .into_iter()
        .map(|(k, (count, lo, hi))| CategoryStat {
            category: k.to_string(),
            count,
            earliest: lo,
            latest: hi,
        })
        .collect();
    categories.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.category.cmp(&b.category)));

    Stats { categories, total_events: events.len(), earliest, latest }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: &str, kind: &str) -> Event {
        // Event::new is private; build via the public from_* path is overkill here,
        // so construct through serde-equivalent fields using the timeline helper.
        Event { timestamp: ts.to_string(), kind: kind.to_string(), summary: String::new() }
    }

    #[test]
    fn counts_and_ranges_per_category() {
        let events = vec![
            ev("2020-01-01T00:00:00+00:00", "call"),
            ev("2020-03-01T00:00:00+00:00", "call"),
            ev("2020-02-01T00:00:00+00:00", "call"),
            ev("2021-06-01T00:00:00+00:00", "photo"),
        ];
        let s = summarize(&events);
        assert_eq!(s.total_events, 4);
        assert_eq!(s.earliest, "2020-01-01T00:00:00+00:00");
        assert_eq!(s.latest, "2021-06-01T00:00:00+00:00");

        // Sorted by count desc → call (3) before photo (1).
        assert_eq!(s.categories[0].category, "call");
        assert_eq!(s.categories[0].count, 3);
        assert_eq!(s.categories[0].earliest, "2020-01-01T00:00:00+00:00");
        assert_eq!(s.categories[0].latest, "2020-03-01T00:00:00+00:00");
        assert_eq!(s.categories[1].category, "photo");
        assert_eq!(s.categories[1].count, 1);
        assert_eq!(s.categories[1].earliest, s.categories[1].latest);
    }

    #[test]
    fn undated_events_counted_but_excluded_from_range() {
        let events = vec![ev("", "note"), ev("2020-01-01T00:00:00+00:00", "note")];
        let s = summarize(&events);
        assert_eq!(s.categories.len(), 1);
        assert_eq!(s.categories[0].count, 2);
        assert_eq!(s.categories[0].earliest, "2020-01-01T00:00:00+00:00");
        assert_eq!(s.categories[0].latest, "2020-01-01T00:00:00+00:00");
    }

    #[test]
    fn empty_input_is_all_empty() {
        let s = summarize(&[]);
        assert_eq!(s.total_events, 0);
        assert!(s.categories.is_empty());
        assert_eq!(s.earliest, "");
        assert_eq!(s.latest, "");
    }
}

//! Case-file search: match one query term across every textual record the
//! in-process extractors produce (the unified timeline) plus the address book, so
//! an examiner can pull "everything that mentions X" — a phone number, a name, a
//! keyword — in a single read-only pass. Matching is a case-insensitive substring;
//! results carry the source category and the record's timestamp when it has one.
//! Best-effort: it searches the one-line summaries the timeline builds, which hold
//! the salient text (message bodies, call numbers, titles, URLs, resolved names).

use serde::Serialize;

use crate::contacts::Contact;
use crate::timeline::Event;

/// One record matching the query.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    /// Source category, e.g. `call`, `whatsapp`, `note`, `safari`, `contacts`.
    pub store: String,
    /// ISO 8601 UTC timestamp when the source record has one, else `None`.
    pub timestamp: Option<String>,
    /// The matched record's one-line description.
    pub snippet: String,
}

fn contains_ci(haystack: &str, needle_lc: &str) -> bool {
    haystack.to_lowercase().contains(needle_lc)
}

/// Search timeline `events` and the address book `contacts` for `query`
/// (case-insensitive substring). An empty/whitespace query matches nothing.
/// Timeline hits come first in their existing order, then contact hits.
pub fn search(events: &[Event], contacts: &[Contact], query: &str) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<SearchHit> = Vec::new();
    for e in events {
        if contains_ci(&e.summary, &q) || contains_ci(&e.kind, &q) {
            hits.push(SearchHit {
                store: e.kind.clone(),
                timestamp: (!e.timestamp.is_empty()).then(|| e.timestamp.clone()),
                snippet: e.summary.clone(),
            });
        }
    }
    for c in contacts {
        let name = format!("{} {}", c.first, c.last);
        let mut fields: Vec<&str> = vec![name.as_str(), c.organization.as_str(), c.note.as_str()];
        fields.extend(c.phones.iter().map(|p| p.value.as_str()));
        fields.extend(c.emails.iter().map(|e| e.value.as_str()));
        if !fields.iter().any(|f| contains_ci(f, &q)) {
            continue;
        }
        // Compact identity snippet: "First Last · Org — phone, email".
        let mut parts: Vec<String> = Vec::new();
        let nm = name.trim().to_string();
        if !nm.is_empty() {
            parts.push(nm);
        }
        if !c.organization.is_empty() {
            parts.push(c.organization.clone());
        }
        let handles: Vec<String> = c.phones.iter().chain(c.emails.iter()).map(|h| h.value.clone()).collect();
        let mut snippet = parts.join(" · ");
        if !handles.is_empty() {
            snippet = if snippet.is_empty() { handles.join(", ") } else { format!("{snippet} — {}", handles.join(", ")) };
        }
        if snippet.is_empty() {
            snippet = c.note.clone();
        }
        hits.push(SearchHit { store: "contacts".into(), timestamp: None, snippet });
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{Contact, Labeled};

    fn ev(ts: &str, kind: &str, summary: &str) -> Event {
        Event { timestamp: ts.into(), kind: kind.into(), summary: summary.into() }
    }

    fn contact(first: &str, last: &str, org: &str, phone: &str) -> Contact {
        Contact {
            first: first.into(),
            last: last.into(),
            organization: org.into(),
            phones: if phone.is_empty() { vec![] } else { vec![Labeled { label: "mobile".into(), value: phone.into() }] },
            emails: vec![],
            note: String::new(),
            addresses: vec![],
        }
    }

    #[test]
    fn matches_event_summary_case_insensitively() {
        let events = vec![
            ev("2020-01-06T00:00:00+00:00", "call", "+420776452878 (42s)"),
            ev("2020-02-01T00:00:00+00:00", "note", "Nákupní seznam: mléko"),
            ev("2020-03-01T00:00:00+00:00", "safari", "https://example.com/page"),
        ];
        let hits = search(&events, &[], "MLÉKO");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].store, "note");
        assert_eq!(hits[0].timestamp.as_deref(), Some("2020-02-01T00:00:00+00:00"));
    }

    #[test]
    fn matches_phone_number_across_events_and_contacts() {
        let events = vec![ev("2020-01-06T00:00:00+00:00", "call", "+420776452878 (42s)")];
        let contacts = vec![contact("Jan", "Novák", "Acme", "+420776452878")];
        let hits = search(&events, &contacts, "776452878");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].store, "call");
        assert_eq!(hits[1].store, "contacts");
        assert_eq!(hits[1].timestamp, None);
        assert!(hits[1].snippet.contains("Jan Novák"));
    }

    #[test]
    fn empty_query_matches_nothing() {
        let events = vec![ev("2020-01-06T00:00:00+00:00", "call", "anything")];
        assert!(search(&events, &[], "   ").is_empty());
    }

    #[test]
    fn non_matching_query_returns_no_hits() {
        let events = vec![ev("2020-01-06T00:00:00+00:00", "call", "+420776452878")];
        let contacts = vec![contact("Jan", "Novák", "Acme", "+420111000999")];
        assert!(search(&events, &contacts, "zzzznotfound").is_empty());
    }
}

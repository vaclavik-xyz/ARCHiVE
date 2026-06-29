//! Case-file search: match one query term across every textual record the
//! in-process extractors produce (the unified timeline) plus the address book, so
//! an examiner can pull "everything that mentions X" — a phone number, a name, a
//! keyword — in a single read-only pass. Matching is a case-insensitive substring;
//! results carry the source category and the record's timestamp when it has one.
//!
//! Each searchable record keeps the human-readable `snippet` separate from the
//! `searchable` text it is matched against. For handle-bearing stores (calls,
//! voicemail, WhatsApp) the timeline summary shows the *resolved contact name*
//! while `searchable` also carries the *raw number / JID* — so a phone-number
//! query still finds a call even after the address book renamed it to a person.

use serde::Serialize;

use crate::contacts::Contact;
use crate::timeline::Event;

/// One searchable record: a human-readable `snippet` to display plus the
/// `searchable` text actually matched against (a superset of the snippet for
/// handle-bearing stores, which also fold in the raw number/JID).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRecord {
    pub store: String,
    pub timestamp: Option<String>,
    pub snippet: String,
    pub searchable: String,
}

impl SearchRecord {
    fn ts(timestamp: &str) -> Option<String> {
        (!timestamp.is_empty()).then(|| timestamp.to_string())
    }

    /// A record whose searchable text is exactly its display snippet.
    pub fn from_event(e: &Event) -> Self {
        SearchRecord {
            store: e.kind.clone(),
            timestamp: Self::ts(&e.timestamp),
            snippet: e.summary.clone(),
            searchable: e.summary.clone(),
        }
    }

    /// A record whose searchable text folds in an extra raw handle (number/JID)
    /// alongside the (possibly contact-name-resolved) display snippet.
    pub fn with_extra(e: &Event, extra: &str) -> Self {
        let searchable =
            if extra.is_empty() { e.summary.clone() } else { format!("{} {extra}", e.summary) };
        SearchRecord {
            store: e.kind.clone(),
            timestamp: Self::ts(&e.timestamp),
            snippet: e.summary.clone(),
            searchable,
        }
    }
}

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

/// Apply `--redact` to the search *output*: returns the display query and hits the
/// caller should render and report. When `redact` is true, the echoed query and
/// every hit snippet are masked by [`crate::redact::redact_pii`] (so a redacted
/// report leaks neither the searched identifier nor the matched ones); when false
/// they pass through unchanged. Matching has already run on the raw query upstream.
pub fn apply_redaction(query: &str, mut hits: Vec<SearchHit>, redact: bool) -> (String, Vec<SearchHit>) {
    if !redact {
        return (query.to_string(), hits);
    }
    for h in &mut hits {
        h.snippet = crate::redact::redact_pii(&h.snippet);
    }
    (crate::redact::redact_pii(query), hits)
}

/// Search `records` and the address book `contacts` for `query` (case-insensitive
/// substring of each record's `searchable` text). An empty/whitespace query
/// matches nothing. Record hits come first in their existing order, then contacts.
pub fn search(records: &[SearchRecord], contacts: &[Contact], query: &str) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<SearchHit> = Vec::new();
    for r in records {
        if contains_ci(&r.searchable, &q) {
            hits.push(SearchHit { store: r.store.clone(), timestamp: r.timestamp.clone(), snippet: r.snippet.clone() });
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

    fn rec(ts: &str, kind: &str, summary: &str) -> SearchRecord {
        SearchRecord::from_event(&ev(ts, kind, summary))
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
    fn matches_record_snippet_case_insensitively() {
        let records = vec![
            rec("2020-01-06T00:00:00+00:00", "call", "outgoing Jan Novák (42s)"),
            rec("2020-02-01T00:00:00+00:00", "note", "Nákupní seznam: mléko"),
            rec("2020-03-01T00:00:00+00:00", "safari", "https://example.com/page"),
        ];
        let hits = search(&records, &[], "MLÉKO");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].store, "note");
        assert_eq!(hits[0].timestamp.as_deref(), Some("2020-02-01T00:00:00+00:00"));
    }

    #[test]
    fn raw_handle_matches_even_after_enrichment() {
        // A call whose display snippet was renamed to the contact still matches the
        // raw number via `with_extra`, and the contact matches too.
        let enriched = ev("2020-01-06T00:00:00+00:00", "call", "outgoing Jan Novák (42s)");
        let records = vec![SearchRecord::with_extra(&enriched, "+420776452878")];
        let contacts = vec![contact("Jan", "Novák", "Acme", "+420776452878")];
        let hits = search(&records, &contacts, "776452878");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].store, "call");
        assert!(hits[0].snippet.contains("Jan Novák")); // display keeps the resolved name
        assert_eq!(hits[1].store, "contacts");
        assert_eq!(hits[1].timestamp, None);
    }

    #[test]
    fn kind_name_is_not_a_match() {
        // Querying a category name must not return every record of that category.
        let records = vec![rec("2020-01-06T00:00:00+00:00", "call", "outgoing Jan (42s)")];
        assert!(search(&records, &[], "call").is_empty());
    }

    #[test]
    fn apply_redaction_masks_query_and_snippets_when_on() {
        let hits = vec![SearchHit { store: "call".into(), timestamp: None, snippet: "outgoing +420776452878 (42s)".into() }];
        // redact on → query and snippets masked, no raw identifier survives.
        let (q, redacted) = apply_redaction("+420776452878", hits.clone(), true);
        assert_eq!(q, "+••••••••••78");
        assert!(!redacted[0].snippet.contains("420776452878"));
        assert!(redacted[0].snippet.contains("••"));
        // redact off → passes through verbatim.
        let (q2, plain) = apply_redaction("+420776452878", hits, false);
        assert_eq!(q2, "+420776452878");
        assert!(plain[0].snippet.contains("420776452878"));
    }

    #[test]
    fn empty_query_matches_nothing() {
        let records = vec![rec("2020-01-06T00:00:00+00:00", "call", "anything")];
        assert!(search(&records, &[], "   ").is_empty());
    }

    #[test]
    fn non_matching_query_returns_no_hits() {
        let records = vec![rec("2020-01-06T00:00:00+00:00", "call", "outgoing Jan (1s)")];
        let contacts = vec![contact("Jan", "Novák", "Acme", "+420111000999")];
        assert!(search(&records, &contacts, "zzzznotfound").is_empty());
    }
}

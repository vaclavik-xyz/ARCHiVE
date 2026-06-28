//! Attribute generic carved SQLite records (from `archive_core::carve`) to the
//! stores we care about and extract a salient summary for each. The carver is
//! schema-less; these *signatures* recognise a deleted row by the shape of its
//! decoded values (an anchor column plus heuristics) and reduce it to a uniform
//! [`DeletedRecord`] — much like the timeline view.
//!
//! Everything here is **best-effort**: carved bytes can decode as a plausible
//! record by chance, column positions drift across iOS versions, and unresolved
//! foreign keys mean some fields are unknowable. Recovered rows are inherently
//! lower-confidence than live ones; callers must present them as such.

use std::collections::HashSet;

use archive_core::carve::{CarveSource, CarvedRecord, CarvedValue};
use serde::Serialize;

use crate::datetime;

/// Keys identifying rows that are still LIVE in the current database, used to
/// reject carved candidates that are not actually deleted. This matters most for
/// WAL frames, which are full page images containing live rows: without this a
/// live message/call/contact present in `-wal` would be reported as deleted. A
/// carved record whose cell rowid (or, for messages, GUID) is still present in
/// the live table is excluded; genuinely deleted rows are gone from the live
/// table (or carry no cell framing) and are kept. Built by the controller from a
/// live read of the database; empty means "exclude nothing".
#[derive(Debug, Default, Clone)]
pub struct LiveKeys {
    /// Cell rowids present in the live table (sms.db ROWID, CallHistory Z_PK, …).
    pub rowids: HashSet<i64>,
    /// Live message GUIDs (extra safety against rowid reuse), messages only.
    pub guids: HashSet<String>,
}

/// One recovered deleted row, normalised across stores (mirrors the timeline
/// `Event` shape so a single formatter/template renders all of them).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeletedRecord {
    /// Which store the row was attributed to: `messages` | `calls` | `contacts`.
    pub store: String,
    /// Which free region it was carved from: `freelist` | `freeblock` | `unallocated` | `wal`.
    pub source: String,
    /// The cell rowid when recovered from full cell framing, else `None`.
    pub rowid: Option<i64>,
    /// The row's salient timestamp as ISO 8601 UTC, when one could be identified.
    pub date: Option<String>,
    /// A human-readable one-line description (message text, call number, names).
    pub summary: String,
}

fn source_str(s: CarveSource) -> &'static str {
    match s {
        CarveSource::Freelist => "freelist",
        CarveSource::Freeblock => "freeblock",
        CarveSource::Unallocated => "unallocated",
        CarveSource::Wal => "wal",
    }
}

fn as_text(v: &CarvedValue) -> Option<&str> {
    match v {
        CarvedValue::Text(t) => Some(t),
        _ => None,
    }
}
fn as_int(v: &CarvedValue) -> Option<i64> {
    match v {
        CarvedValue::Int(i) => Some(*i),
        _ => None,
    }
}
fn as_real(v: &CarvedValue) -> Option<f64> {
    match v {
        CarvedValue::Real(r) => Some(*r),
        _ => None,
    }
}

/// Whitespace-collapse and cap a string for one-line display.
fn trunc(s: &str, n: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= n {
        collapsed
    } else {
        let head: String = collapsed.chars().take(n).collect();
        format!("{head}…")
    }
}

/// A Messages `guid` is a 36-char uppercase-or-lower hex UUID (`8-4-4-4-12`).
/// This is the discriminating anchor for an sms.db `message` row.
fn is_guid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }
        })
}

/// Loosely "addressy": a short string carrying a phone number / email / handle.
fn looks_addressy(s: &str) -> bool {
    let len = s.chars().count();
    (3..=64).contains(&len)
        && (s.contains('@') || s.bytes().filter(|b| b.is_ascii_digit()).count() >= 3)
}

/// sms.db `message`: anchored by a 36-char guid. The body is the longest
/// non-guid text; the date is the largest plausible Cocoa value (the table
/// stores several close timestamps — date/date_read/date_delivered — in
/// nanoseconds; `cocoa_any_to_iso` handles the ns-vs-s magnitude).
fn messages_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            let guid = r.values.iter().filter_map(as_text).find(|t| is_guid(t))?;
            // Skip rows still live (e.g. a live cell captured in a WAL frame).
            if r.rowid.is_some_and(|id| live.rowids.contains(&id)) || live.guids.contains(guid) {
                return None;
            }
            let date = r
                .values
                .iter()
                .filter_map(as_int)
                .filter(|&i| i >= 100_000_000)
                .max()
                .and_then(datetime::cocoa_any_to_iso);
            let text = r
                .values
                .iter()
                .filter_map(as_text)
                .filter(|t| !is_guid(t) && !t.is_empty())
                .max_by_key(|t| t.chars().count());
            let summary = match text {
                Some(t) => trunc(t, 200),
                None => format!("(deleted message {guid})"),
            };
            Some(DeletedRecord {
                store: "messages".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                date,
                summary,
            })
        })
        .collect()
}

/// Whether a carved calls/contacts candidate should be rejected. Unlike messages
/// (which carry a GUID anchor usable for live-exclusion), these stores have no
/// strong content key, so beyond dropping rows whose rowid is still live we also
/// drop rowid-less candidates carved from WAL frames: a WAL frame is a full page
/// image, and a genuinely deleted row survives there with cell framing (a rowid),
/// whereas a rowid-less raw-scan hit is almost always a duplicate of a live row.
fn excluded_call_or_contact(r: &CarvedRecord, live: &LiveKeys) -> bool {
    if r.rowid.is_some_and(|id| live.rowids.contains(&id)) {
        return true;
    }
    r.source == CarveSource::Wal && r.rowid.is_none()
}

/// CallHistory `ZCALLRECORD` (Core Data): anchored by a REAL in a plausible
/// Cocoa-seconds date range. Duration is a separate small REAL; the address is a
/// phone-ish text/blob value.
fn calls_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_call_or_contact(r, live) {
                return None;
            }
            // ~2008-12 .. ~2035 in Cocoa (2001-epoch) seconds.
            let date_real = r
                .values
                .iter()
                .filter_map(as_real)
                .find(|&x| (250_000_000.0..1_100_000_000.0).contains(&x))?;
            let date = datetime::cocoa_to_iso(date_real);
            let duration = r
                .values
                .iter()
                .filter_map(as_real)
                .find(|&x| (0.0..86_400.0).contains(&x));
            let address = r.values.iter().find_map(|v| match v {
                CarvedValue::Text(t) if looks_addressy(t) => Some(t.clone()),
                CarvedValue::Blob(b) => std::str::from_utf8(b)
                    .ok()
                    .filter(|s| looks_addressy(s))
                    .map(str::to_string),
                _ => None,
            });
            let mut summary = address.unwrap_or_else(|| "(unknown number)".into());
            if let Some(d) = duration {
                summary = format!("{summary} ({}s)", d.round() as i64);
            }
            Some(DeletedRecord {
                store: "calls".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                date,
                summary,
            })
        })
        .collect()
}

/// AddressBook `ABPerson`: the softest signature (no strong anchor). Keeps rows
/// carrying at least one alphabetic text value (a name/organization) and joins
/// the non-empty texts. Noisier than the others — labelled best-effort.
fn contacts_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_call_or_contact(r, live) {
                return None;
            }
            let texts: Vec<&str> = r.values.iter().filter_map(as_text).filter(|t| !t.is_empty()).collect();
            if texts.is_empty() || !texts.iter().any(|t| t.chars().any(char::is_alphabetic)) {
                return None;
            }
            Some(DeletedRecord {
                store: "contacts".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                date: None,
                summary: trunc(&texts.join(" · "), 200),
            })
        })
        .collect()
}

/// Apply the signature for `store` to carved records, excluding rows still live
/// in the database (`live`) — which filters out the live rows that WAL frame
/// images inevitably contain — then drop near-duplicates (the same row often
/// survives in more than one free region).
pub fn recover(store: &str, records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    let mut out = match store {
        "messages" => messages_from(records, live),
        "calls" => calls_from(records, live),
        "contacts" => contacts_from(records, live),
        _ => Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    out.retain(|r| seen.insert((r.rowid, r.summary.clone(), r.date.clone())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(source: CarveSource, values: Vec<CarvedValue>) -> CarvedRecord {
        rec_n(Some(7), source, values)
    }

    fn rec_n(rowid: Option<i64>, source: CarveSource, values: Vec<CarvedValue>) -> CarvedRecord {
        CarvedRecord { rowid, source, values, truncated: false }
    }

    #[test]
    fn messages_signature_extracts_guid_text_and_date() {
        // guid anchor + a Cocoa-nanosecond date + a body, plus a decoy short text.
        let records = vec![rec(
            CarveSource::Wal,
            vec![
                CarvedValue::Text("9B7E5F2A-1C3D-4E5F-8A9B-0C1D2E3F4A5B".into()),
                CarvedValue::Int(600_000_000_000_000_000), // Cocoa ns
                CarvedValue::Int(3),                       // handle_id (ignored)
                CarvedValue::Text("ahoj jak se mas".into()),
                CarvedValue::Text("x".into()),
            ],
        )];
        let out = recover("messages", &records, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].store, "messages");
        assert_eq!(out[0].source, "wal");
        assert_eq!(out[0].summary, "ahoj jak se mas");
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));
    }

    #[test]
    fn messages_signature_ignores_records_without_guid() {
        let records = vec![rec(CarveSource::Freelist, vec![CarvedValue::Text("no guid here".into())])];
        assert!(recover("messages", &records, &LiveKeys::default()).is_empty());
    }

    #[test]
    fn calls_signature_extracts_date_number_and_duration() {
        let records = vec![rec(
            CarveSource::Freelist,
            vec![
                CarvedValue::Int(1),               // Z_PK
                CarvedValue::Real(600_000_000.0),  // ZDATE (Cocoa seconds, 2020)
                CarvedValue::Real(42.0),           // ZDURATION
                CarvedValue::Text("+420776452878".into()),
            ],
        )];
        let out = recover("calls", &records, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].store, "calls");
        assert!(out[0].summary.contains("+420776452878"));
        assert!(out[0].summary.contains("42s"));
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));
    }

    #[test]
    fn contacts_signature_joins_names_and_skips_noise() {
        let names = vec![rec(
            CarveSource::Unallocated,
            vec![
                CarvedValue::Text("Jan".into()),
                CarvedValue::Text("Novák".into()),
                CarvedValue::Null,
                CarvedValue::Text("".into()),
            ],
        )];
        let out = recover("contacts", &names, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "Jan · Novák");
        assert_eq!(out[0].date, None);

        // A record with only digits (no alphabetic text) is dropped as noise.
        let noise = vec![rec(CarveSource::Unallocated, vec![CarvedValue::Text("12345".into())])];
        assert!(recover("contacts", &noise, &LiveKeys::default()).is_empty());
    }

    #[test]
    fn recover_dedupes_identical_rows_from_multiple_sources() {
        let g = "9B7E5F2A-1C3D-4E5F-8A9B-0C1D2E3F4A5B";
        let mk = |src| rec(src, vec![CarvedValue::Text(g.into()), CarvedValue::Text("dup body".into())]);
        let records = vec![mk(CarveSource::Freeblock), mk(CarveSource::Wal)];
        // Same rowid + summary + date → collapsed to one.
        assert_eq!(recover("messages", &records, &LiveKeys::default()).len(), 1);
    }

    #[test]
    fn live_rows_are_excluded() {
        // A message whose rowid is still live (as in a WAL frame's live cell) and
        // a separate message whose GUID is still live are both dropped; a third,
        // genuinely-deleted message survives.
        let g_live = "AAAAAAAA-1111-2222-3333-444444444444";
        let g_gone = "BBBBBBBB-5555-6666-7777-888888888888";
        let mut records = vec![
            CarvedRecord { rowid: Some(10), source: CarveSource::Wal, values: vec![CarvedValue::Text(g_live.into()), CarvedValue::Text("live by rowid".into())], truncated: false },
            CarvedRecord { rowid: Some(99), source: CarveSource::Wal, values: vec![CarvedValue::Text(g_gone.into()), CarvedValue::Text("deleted".into())], truncated: false },
        ];
        // A third row: live by GUID (different rowid).
        records.push(CarvedRecord { rowid: Some(50), source: CarveSource::Wal, values: vec![CarvedValue::Text("CCCCCCCC-9999-0000-1111-222222222222".into()), CarvedValue::Text("live by guid".into())], truncated: false });

        let mut live = LiveKeys::default();
        live.rowids.insert(10);
        live.guids.insert("CCCCCCCC-9999-0000-1111-222222222222".into());

        let out = recover("messages", &records, &live);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "deleted");
    }

    #[test]
    fn calls_and_contacts_drop_rowidless_wal_candidates() {
        // A rowid-less WAL raw-scan hit (a live-row duplicate from a full page
        // image) is dropped for calls and contacts...
        let call = vec![rec_n(
            None,
            CarveSource::Wal,
            vec![CarvedValue::Real(600_000_000.0), CarvedValue::Real(30.0), CarvedValue::Text("+420111222333".into())],
        )];
        assert!(recover("calls", &call, &LiveKeys::default()).is_empty());
        let contact = vec![rec_n(
            None,
            CarveSource::Wal,
            vec![CarvedValue::Text("Eva".into()), CarvedValue::Text("Dvořák".into())],
        )];
        assert!(recover("contacts", &contact, &LiveKeys::default()).is_empty());
        // ...but a framed (rowid present) WAL call that is not live is kept.
        let framed = vec![rec_n(
            Some(123),
            CarveSource::Wal,
            vec![CarvedValue::Real(600_000_000.0), CarvedValue::Real(30.0), CarvedValue::Text("+420111222333".into())],
        )];
        assert_eq!(recover("calls", &framed, &LiveKeys::default()).len(), 1);
    }

    #[test]
    fn unknown_store_yields_nothing() {
        let records = vec![rec(CarveSource::Wal, vec![CarvedValue::Text("x".into())])];
        assert!(recover("photos", &records, &LiveKeys::default()).is_empty());
    }
}

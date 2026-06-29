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
    /// Live Safari history URLs (`history_items.url`). Safari recovers URL-only
    /// rows that carry no usable rowid key, so live URLs are excluded by content.
    pub urls: HashSet<String>,
}

/// One recovered deleted row, normalised across stores (mirrors the timeline
/// `Event` shape so a single formatter/template renders all of them).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeletedRecord {
    /// Which store the row was attributed to: `messages` | `calls` | `contacts`
    /// | `notes` | `calendar` | `safari` | `photos`.
    pub store: String,
    /// Which free region it was carved from: `freelist` | `freeblock` | `unallocated` | `wal`.
    pub source: String,
    /// The cell rowid when recovered from full cell framing, else `None`.
    pub rowid: Option<i64>,
    /// The row's salient timestamp as ISO 8601 UTC, when one could be identified.
    pub date: Option<String>,
    /// A human-readable one-line description (message text, call number, names).
    pub summary: String,
    /// The carved cell ran past the bytes available (record-size cap or an
    /// overwritten tail), so the recovered values are **partial** — trailing
    /// columns may be missing or cut short. Surfaced so callers can flag the row.
    pub truncated: bool,
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

/// A REAL that looks like a Cocoa (2001-epoch) seconds timestamp in a plausible
/// range (~2008 .. ~2035), the date anchor for calls/calendar/safari rows.
fn cocoa_seconds(v: &CarvedValue) -> Option<f64> {
    match v {
        CarvedValue::Real(r) if (250_000_000.0..1_100_000_000.0).contains(r) => Some(*r),
        _ => None,
    }
}

/// The largest plausible Cocoa-seconds date among a record's values, as ISO 8601.
fn max_cocoa_date(r: &CarvedRecord) -> Option<String> {
    r.values
        .iter()
        .filter_map(cocoa_seconds)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .and_then(datetime::cocoa_to_iso)
}

/// The smallest plausible Cocoa-seconds date among a record's values, as ISO
/// 8601. Used for calendar events: schema-less carving cannot single out the
/// start column from end/created, so we report the *earliest* associated date —
/// the closest honest proxy for when the event happened (its start, or the
/// creation date when that is earlier).
fn min_cocoa_date(r: &CarvedRecord) -> Option<String> {
    r.values
        .iter()
        .filter_map(cocoa_seconds)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .and_then(datetime::cocoa_to_iso)
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

/// Whether a text is an AddressBook label literal (`_$!<Home>!$_`). These appear
/// in the separate `ABMultiValueLabel` table — a deleted phone/email row stores
/// only an integer FK to it, not this string — so when one is carved we treat it
/// as noise to keep out of the recovered name, never as contact content.
fn is_ab_label(s: &str) -> bool {
    s.len() > 8 && s.starts_with("_$!<") && s.ends_with(">!$_")
}

/// A text that looks like an email address (an `@` with a dotted domain after it).
fn looks_email(s: &str) -> bool {
    let len = s.chars().count();
    if !(5..=254).contains(&len) || s.chars().any(char::is_whitespace) {
        return false;
    }
    matches!((s.find('@'), s.rfind('.')), (Some(at), Some(dot)) if at > 0 && dot > at + 1 && dot + 1 < s.len())
}

/// A text that looks like a phone number: only digits and phone punctuation, with
/// enough digits (≥7) to be a real number rather than a stray short integer (so a
/// bare `12345` stays noise, while `+420776452878` and `776452878` are phones).
fn looks_phone(s: &str) -> bool {
    let len = s.chars().count();
    if !(5..=40).contains(&len) || s.contains('@') {
        return false;
    }
    let digits = s.bytes().filter(u8::is_ascii_digit).count();
    digits >= 7 && s.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c))
}

/// Whether a decoded text reads as genuine human content rather than carved
/// binary. Rejects strings dominated (≥20%) by control / non-printable characters
/// **or** Unicode replacement characters — the carver decodes with
/// `String::from_utf8_lossy`, so invalid bytes surface as `U+FFFD`, not raw
/// control bytes. Gates the *soft* text anchors (names, titles, note snippets,
/// message bodies) so a random byte run can't mint a record.
fn looks_texty(s: &str) -> bool {
    let total = s.chars().count();
    if total == 0 {
        return false;
    }
    let bad = s
        .chars()
        .filter(|&c| c == '\u{FFFD}' || (c.is_control() && !matches!(c, '\t' | '\n' | '\r')))
        .count();
    bad * 5 < total
}

/// A text that looks like a browser URL — the (soft) anchor for Safari history.
fn looks_url(s: &str) -> bool {
    (s.contains("://") || s.starts_with("www."))
        && !s.chars().any(char::is_whitespace)
        && (6..=2048).contains(&s.chars().count())
}

/// Media-file extensions a Camera Roll asset's `ZFILENAME` ends with.
const PHOTO_EXTS: &[&str] = &[
    ".heic", ".heif", ".jpg", ".jpeg", ".png", ".gif", ".mov", ".mp4", ".m4v", ".dng", ".tiff", ".tif", ".aae", ".webp",
];

/// A text that looks like a Camera Roll filename — the anchor for a deleted
/// `ZASSET` row (a media extension, no spaces, plausible length).
fn looks_photo_filename(s: &str) -> bool {
    let len = s.chars().count();
    if !(5..=128).contains(&len) || s.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    PHOTO_EXTS.iter().any(|e| lower.ends_with(e))
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
                .filter(|t| !is_guid(t) && !t.is_empty() && looks_texty(t))
                .max_by_key(|t| t.chars().count());
            let summary = match text {
                Some(t) => trunc(t, 200),
                None => format!("(deleted message {guid})"),
            };
            Some(DeletedRecord {
                store: "messages".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                truncated: r.truncated,
                date,
                summary,
            })
        })
        .collect()
}

/// Whether a carved candidate from an *anchorless* store should be rejected.
/// Unlike messages (which carry a GUID anchor usable for live-exclusion), these
/// stores — calls, contacts, notes, calendar, safari — have no strong content
/// key, so we cannot reliably separate deleted rows from the live rows that WAL
/// frames inevitably contain: a WAL frame is a full page image, and the carver's
/// raw slide can even misframe live bytes into a rowid-bearing candidate, so
/// neither "rowid is live" nor "has a rowid" is a sufficient test. We therefore
/// drop **every** WAL-sourced candidate for these stores — genuinely deleted rows
/// still surface from the main file's free regions (freelist/freeblock/
/// unallocated), which are not contaminated by live cells. We also drop rows
/// whose rowid is still live.
fn excluded_anchorless(r: &CarvedRecord, live: &LiveKeys) -> bool {
    r.source == CarveSource::Wal || r.rowid.is_some_and(|id| live.rowids.contains(&id))
}

/// CallHistory `ZCALLRECORD` (Core Data): anchored by a REAL in a plausible
/// Cocoa-seconds date range. Duration is a separate small REAL; the address is a
/// phone-ish text/blob value.
fn calls_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_anchorless(r, live) {
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
            // Corroboration: a lone in-range REAL is too weak an anchor on its own
            // (any date-shaped float would mint a phantom call). Require a second
            // call signal — a number, or a plausible duration (0s for a missed call).
            if address.is_none() && duration.is_none() {
                return None;
            }
            let mut summary = address.unwrap_or_else(|| "(unknown number)".into());
            if let Some(d) = duration {
                summary = format!("{summary} ({}s)", d.round() as i64);
            }
            Some(DeletedRecord {
                store: "calls".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                truncated: r.truncated,
                date,
                summary,
            })
        })
        .collect()
}

/// AddressBook (`ABPerson` / `ABMultiValue`): the softest signature (no strong
/// anchor). Rather than blindly joining every text, classify each value into a
/// name/org field, an email, or a phone, then reassemble a structured contact:
/// `Name · Org — phone, email`. A lone deleted phone/email value (an
/// `ABMultiValue` row, no name) is now recovered too, instead of dropped as "no
/// alphabetic text". Labels live in a separate `ABMultiValueLabel` table (an
/// integer FK in the value row), so a carved label literal is treated as noise,
/// not annotated onto a handle. Still best-effort/noisier than the anchored stores.
fn contacts_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_anchorless(r, live) {
                return None;
            }
            let (mut names, mut emails, mut phones): (Vec<&str>, Vec<&str>, Vec<&str>) =
                (Vec::new(), Vec::new(), Vec::new());
            for t in r.values.iter().filter_map(as_text).filter(|t| !t.is_empty()) {
                if is_ab_label(t) {
                    continue;
                } else if looks_email(t) {
                    if !emails.contains(&t) {
                        emails.push(t);
                    }
                } else if looks_phone(t) {
                    if !phones.contains(&t) {
                        phones.push(t);
                    }
                } else if t.chars().count() <= 64
                    && t.chars().any(char::is_alphabetic)
                    && looks_texty(t)
                    && !names.contains(&t)
                {
                    names.push(t);
                }
            }
            if names.is_empty() && emails.is_empty() && phones.is_empty() {
                return None;
            }
            // Reassemble: name/org fields, then the contact handles (phones, then
            // emails). We can't pair labels to values across the carved soup, so
            // handles are listed plain.
            let mut handles: Vec<String> = Vec::new();
            handles.extend(phones.iter().map(|p| (*p).to_string()));
            handles.extend(emails.iter().map(|e| (*e).to_string()));
            let name_part = names.join(" · ");
            let summary = match (name_part.is_empty(), handles.is_empty()) {
                (false, false) => format!("{name_part} — {}", handles.join(", ")),
                (false, true) => name_part,
                (true, false) => handles.join(", "),
                (true, true) => return None,
            };
            Some(DeletedRecord {
                store: "contacts".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                truncated: r.truncated,
                date: None,
                summary: trunc(&summary, 200),
            })
        })
        .collect()
}

/// NoteStore.sqlite (`ZICCLOUDSYNCINGOBJECT`): no content anchor — the note body
/// is gzipped protobuf in `ZICNOTEDATA`, but the title (`ZTITLE1`) and snippet
/// (`ZSNIPPET`) are plain-text columns. Joins the non-empty alphabetic texts and
/// attaches the largest Cocoa date (creation/modification). Best-effort/noisier.
fn notes_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_anchorless(r, live) {
                return None;
            }
            let texts: Vec<&str> = r
                .values
                .iter()
                .filter_map(as_text)
                .filter(|t| t.chars().count() >= 2 && t.chars().any(char::is_alphabetic) && looks_texty(t))
                .collect();
            if texts.is_empty() {
                return None;
            }
            Some(DeletedRecord {
                store: "notes".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                truncated: r.truncated,
                date: max_cocoa_date(r),
                summary: trunc(&texts.join(" · "), 200),
            })
        })
        .collect()
}

/// Calendar.sqlitedb (`CalendarItem`): the event title (`summary`) is plain text,
/// `start_date` a Cocoa-seconds REAL, `location` more text. Anchors on the longest
/// alphabetic title; attaches the location and a date. The row holds several
/// indistinguishable dates (start/end/created); we report the **earliest** as the
/// most honest proxy for when the event was (see `min_cocoa_date`).
fn calendar_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_anchorless(r, live) {
                return None;
            }
            let title = r
                .values
                .iter()
                .filter_map(as_text)
                .filter(|t| t.chars().any(char::is_alphabetic) && looks_texty(t))
                .max_by_key(|t| t.chars().count())?;
            let location = r
                .values
                .iter()
                .filter_map(as_text)
                .find(|t| *t != title && t.chars().any(char::is_alphabetic) && looks_texty(t));
            let date = min_cocoa_date(r);
            // Corroboration: a lone alphabetic word is too weak to be an event on
            // its own. Require a second signal — a Cocoa date or a location text.
            if date.is_none() && location.is_none() {
                return None;
            }
            let mut summary = trunc(title, 160);
            if let Some(loc) = location {
                summary = format!("{summary} · {}", trunc(loc, 60));
            }
            Some(DeletedRecord {
                store: "calendar".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                truncated: r.truncated,
                date,
                summary,
            })
        })
        .collect()
}

/// Safari `History.db`: deleted browsing history. `history_items` rows carry the
/// URL; `history_visits` rows carry a visit title and a Cocoa-seconds
/// `visit_time`. Anchors on a URL-looking text, or a (title + visit date) pair.
fn safari_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_anchorless(r, live) {
                return None;
            }
            let url = r.values.iter().filter_map(as_text).find(|t| looks_url(t));
            // A URL still present in the live `history_items` table is not deleted.
            if url.is_some_and(|u| live.urls.contains(u)) {
                return None;
            }
            let date = max_cocoa_date(r);
            let title = r
                .values
                .iter()
                .filter_map(as_text)
                .filter(|t| !looks_url(t) && t.chars().any(char::is_alphabetic))
                .max_by_key(|t| t.chars().count());
            // Require a URL, or a titled visit with a date, to avoid plain-text noise.
            let summary = match (url, title) {
                (Some(u), Some(t)) => format!("{} — {}", trunc(t, 120), trunc(u, 200)),
                (Some(u), None) => trunc(u, 200),
                (None, Some(t)) if date.is_some() => trunc(t, 200),
                _ => return None,
            };
            Some(DeletedRecord {
                store: "safari".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                truncated: r.truncated,
                date,
                summary,
            })
        })
        .collect()
}

/// Photos.sqlite (`ZASSET`): a deleted Camera Roll asset — recovered even after it
/// was purged from "Recently Deleted". Anchored by a media `ZFILENAME`; the date
/// is the earliest plausible Cocoa value (`ZDATECREATED`, when the photo was
/// taken, precedes the added/modified dates). The pixels are usually gone, but the
/// filename + capture date prove the asset existed.
fn photos_from(records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    records
        .iter()
        .filter_map(|r| {
            if excluded_anchorless(r, live) {
                return None;
            }
            let filename = r.values.iter().filter_map(as_text).find(|t| looks_photo_filename(t))?;
            Some(DeletedRecord {
                store: "photos".into(),
                source: source_str(r.source).into(),
                rowid: r.rowid,
                date: min_cocoa_date(r),
                truncated: r.truncated,
                summary: trunc(filename, 120),
            })
        })
        .collect()
}

/// Apply the signature for `store` to carved records, excluding rows still live
/// in the database (`live`) — which filters out the live rows that WAL frame
/// images inevitably contain — then drop near-duplicates (the same row often
/// survives in more than one free region).
pub fn recover(store: &str, records: &[CarvedRecord], live: &LiveKeys) -> Vec<DeletedRecord> {
    let out = match store {
        "messages" => messages_from(records, live),
        "calls" => calls_from(records, live),
        "contacts" => contacts_from(records, live),
        "notes" => notes_from(records, live),
        "calendar" => calendar_from(records, live),
        "safari" => safari_from(records, live),
        "photos" => photos_from(records, live),
        _ => Vec::new(),
    };
    // De-duplicate rows that survived in several free regions, preferring a
    // complete copy: a later non-truncated duplicate upgrades an earlier
    // truncated one, so a row is never reported partial when a full copy was also
    // recovered. First-seen order is preserved (the caller sorts by date).
    let mut index: std::collections::HashMap<(Option<i64>, String, Option<String>), usize> =
        std::collections::HashMap::new();
    let mut deduped: Vec<DeletedRecord> = Vec::new();
    for r in out {
        let key = (r.rowid, r.summary.clone(), r.date.clone());
        match index.get(&key) {
            Some(&i) => {
                if deduped[i].truncated && !r.truncated {
                    deduped[i] = r;
                }
            }
            None => {
                index.insert(key, deduped.len());
                deduped.push(r);
            }
        }
    }
    // Truncation can also shorten the recovered fields themselves (the body was
    // cut), so the partial and complete copies of one row carry *different*
    // summaries and escape the exact-key dedup above. Drop a still-truncated row
    // when a complete copy of the *same* row (same rowid, and whose summary begins
    // with the truncated one — i.e. the truncated value is a genuine prefix)
    // exists. The prefix guard avoids collapsing two distinct rows that merely
    // share a reused rowid.
    let complete: Vec<(i64, String, Option<String>)> = deduped
        .iter()
        .filter(|r| !r.truncated)
        .filter_map(|r| r.rowid.map(|id| (id, r.summary.clone(), r.date.clone())))
        .collect();
    deduped.retain(|r| {
        if !r.truncated {
            return true;
        }
        match r.rowid {
            // Drop only when a complete copy of the same row clearly subsumes
            // this one: same rowid, summary prefix, AND a compatible date — the
            // partial either kept the row's date (equal) or lost it to truncation
            // (None). A different date means a distinct row that reused the rowid.
            Some(id) => !complete.iter().any(|(cid, csum, cdate)| {
                *cid == id
                    && csum.starts_with(&r.summary)
                    && (r.date.is_none() || r.date.as_deref() == cdate.as_deref())
            }),
            None => true,
        }
    });
    deduped
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
    fn contacts_signature_reassembles_fields() {
        // ABPerson-ish row: name fields + a phone + an email reassembled in order.
        let person = vec![rec(
            CarveSource::Unallocated,
            vec![
                CarvedValue::Text("Jan".into()),
                CarvedValue::Text("Novák".into()),
                CarvedValue::Text("+420776452878".into()),
                CarvedValue::Text("jan@example.com".into()),
            ],
        )];
        let out = recover("contacts", &person, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "Jan · Novák — +420776452878, jan@example.com");

        // ABMultiValue row in its real shape: record_id + label id (both integer
        // FKs) + the phone value, no name — recovered now (the old "needs an
        // alphabetic text" rule dropped it). The label literal lives in a separate
        // ABMultiValueLabel table, so there is no (label) annotation.
        let phone_only = vec![rec(
            CarveSource::Unallocated,
            vec![
                CarvedValue::Int(42), // record_id FK, ignored
                CarvedValue::Int(3),  // label id FK → ABMultiValueLabel, ignored
                CarvedValue::Text("+420776452878".into()),
            ],
        )];
        let out = recover("contacts", &phone_only, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "+420776452878");

        // A carved AddressBook label literal with nothing else is noise → dropped.
        let label_only = vec![rec(CarveSource::Unallocated, vec![CarvedValue::Text("_$!<Work>!$_".into())])];
        assert!(recover("contacts", &label_only, &LiveKeys::default()).is_empty());
    }

    #[test]
    fn anchor_upgrade_requires_corroboration() {
        // calls: a lone in-range date REAL with neither a number nor a duration is
        // an uncorroborated coincidence → dropped.
        let lone_date = vec![rec(CarveSource::Freelist, vec![CarvedValue::Real(600_000_000.0)])];
        assert!(recover("calls", &lone_date, &LiveKeys::default()).is_empty());
        // ...but a date + duration (a missed call whose number didn't survive) is
        // corroborated and recovered.
        let missed = vec![rec(CarveSource::Freelist, vec![CarvedValue::Real(600_000_000.0), CarvedValue::Real(0.0)])];
        let out = recover("calls", &missed, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert!(out[0].summary.contains("unknown number"));

        // calendar: a lone alphabetic title with no date and no location is noise.
        let bare = vec![rec(CarveSource::Freelist, vec![CarvedValue::Text("Schůzka".into())])];
        assert!(recover("calendar", &bare, &LiveKeys::default()).is_empty());
        // ...but a title + location (no date) is corroborated.
        let titled = vec![rec(CarveSource::Freelist, vec![CarvedValue::Text("Oběd".into()), CarvedValue::Text("Praha".into())])];
        assert_eq!(recover("calendar", &titled, &LiveKeys::default()).len(), 1);
    }

    #[test]
    fn anchor_upgrade_rejects_binary_text() {
        // A carved "title" dominated by control bytes is binary noise, not a real
        // event — dropped even though it decoded as valid UTF-8 and carries a date.
        let junk = "\u{1}\u{2}\u{3}\u{4}ab".to_string(); // 4 control + 2 letters
        let records = vec![rec(CarveSource::Freelist, vec![CarvedValue::Text(junk), CarvedValue::Real(600_000_000.0)])];
        assert!(recover("calendar", &records, &LiveKeys::default()).is_empty());

        // The carver decodes with from_utf8_lossy, so invalid bytes become U+FFFD
        // replacement characters — these must count as binary too, not human text.
        let lossy = "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}ab".to_string(); // 4 replacement + 2 letters
        let records = vec![rec(CarveSource::Freelist, vec![CarvedValue::Text(lossy), CarvedValue::Real(600_000_000.0)])];
        assert!(recover("calendar", &records, &LiveKeys::default()).is_empty());
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
    fn calls_and_contacts_drop_all_wal_candidates() {
        // calls/contacts have no content anchor, so EVERY WAL candidate is
        // dropped — rowid-less raw-scan and rowid-bearing (possibly misframed)
        // alike — since WAL frames mix live and deleted cells indistinguishably.
        let nums = || vec![CarvedValue::Real(600_000_000.0), CarvedValue::Real(30.0), CarvedValue::Text("+420111222333".into())];
        let wal = vec![rec_n(None, CarveSource::Wal, nums()), rec_n(Some(123), CarveSource::Wal, nums())];
        assert!(recover("calls", &wal, &LiveKeys::default()).is_empty());
        let wal_contact = vec![rec_n(None, CarveSource::Wal, vec![CarvedValue::Text("Eva".into()), CarvedValue::Text("Dvořák".into())])];
        assert!(recover("contacts", &wal_contact, &LiveKeys::default()).is_empty());
        // ...but a deleted call from the main file's free regions IS recovered.
        let freelist = vec![rec_n(Some(50), CarveSource::Freelist, nums())];
        assert_eq!(recover("calls", &freelist, &LiveKeys::default()).len(), 1);
    }

    #[test]
    fn notes_signature_joins_text_and_dates() {
        // A deleted note: title + snippet text plus a Cocoa creation date; the
        // gzipped body blob carries no readable text and is ignored.
        let records = vec![rec_n(
            Some(12),
            CarveSource::Freeblock,
            vec![
                CarvedValue::Text("Nákupní seznam".into()),
                CarvedValue::Text("mléko, chléb, vejce".into()),
                CarvedValue::Real(600_000_000.0), // Cocoa seconds, 2020
                CarvedValue::Blob(vec![0x1f, 0x8b, 0x08]), // gzip magic, not text
            ],
        )];
        let out = recover("notes", &records, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].store, "notes");
        assert_eq!(out[0].summary, "Nákupní seznam · mléko, chléb, vejce");
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));
    }

    #[test]
    fn calendar_signature_extracts_title_location_and_date() {
        let records = vec![rec_n(
            Some(3),
            CarveSource::Unallocated,
            vec![
                CarvedValue::Text("Schůzka s klientem".into()),
                CarvedValue::Real(600_000_000.0), // ZSTARTDATE
                CarvedValue::Text("Kavárna Praha".into()),
            ],
        )];
        let out = recover("calendar", &records, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].store, "calendar");
        assert_eq!(out[0].summary, "Schůzka s klientem · Kavárna Praha");
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));
    }

    #[test]
    fn safari_signature_recovers_url_title_and_visit_date() {
        // A history_items row (url) and a history_visits row (title + visit_time).
        let item = rec_n(Some(8), CarveSource::Freelist, vec![CarvedValue::Text("https://example.com/page".into()), CarvedValue::Int(4)]);
        let out = recover("safari", &[item], &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].store, "safari");
        assert_eq!(out[0].summary, "https://example.com/page");

        let visit = rec_n(
            Some(9),
            CarveSource::Freelist,
            vec![CarvedValue::Text("Example Domain".into()), CarvedValue::Real(600_000_000.0)],
        );
        let out = recover("safari", &[visit], &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "Example Domain");
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));
    }

    #[test]
    fn safari_drops_plain_text_without_url_or_date() {
        // A titled visit with no date and no URL is noise → dropped.
        let noise = vec![rec_n(Some(1), CarveSource::Unallocated, vec![CarvedValue::Text("just some words".into())])];
        assert!(recover("safari", &noise, &LiveKeys::default()).is_empty());
    }

    #[test]
    fn soft_stores_drop_all_wal_candidates() {
        // notes/calendar/safari share the anchorless WAL-drop policy.
        let note = vec![rec_n(None, CarveSource::Wal, vec![CarvedValue::Text("Poznámka".into())])];
        assert!(recover("notes", &note, &LiveKeys::default()).is_empty());
        let cal = vec![rec_n(Some(2), CarveSource::Wal, vec![CarvedValue::Text("Událost".into()), CarvedValue::Real(600_000_000.0)])];
        assert!(recover("calendar", &cal, &LiveKeys::default()).is_empty());
        let saf = vec![rec_n(None, CarveSource::Wal, vec![CarvedValue::Text("https://x.test/a".into())])];
        assert!(recover("safari", &saf, &LiveKeys::default()).is_empty());
    }

    #[test]
    fn photos_signature_recovers_filename_and_capture_date() {
        // A deleted ZASSET row: media filename + created/added dates; the
        // earliest (created) date is reported.
        let created = 600_000_000.0; // 2020-01-06
        let added = 631_000_000.0;
        let records = vec![rec_n(
            Some(42),
            CarveSource::Freelist,
            vec![
                CarvedValue::Text("DCIM/100APPLE".into()),
                CarvedValue::Text("IMG_4242.HEIC".into()),
                CarvedValue::Real(added),
                CarvedValue::Real(created),
            ],
        )];
        let out = recover("photos", &records, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].store, "photos");
        assert_eq!(out[0].summary, "IMG_4242.HEIC");
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));

        // A row with no media filename is not a photo asset → dropped.
        let noise = vec![rec_n(Some(1), CarveSource::Freelist, vec![CarvedValue::Text("not a file".into())])];
        assert!(recover("photos", &noise, &LiveKeys::default()).is_empty());
    }

    #[test]
    fn calendar_reports_earliest_date_not_end() {
        // Schema-less carving can't tell start from end/created, so the earliest
        // plausible date is reported (never the later end). Here created < start
        // < end, and the earliest (created) is what we surface.
        let created = 600_000_000.0; // 2020-01-06
        let start = 615_000_000.0; // ~5.5 months later
        let end = 631_000_000.0; // later still
        let records = vec![rec_n(
            Some(4),
            CarveSource::Freelist,
            vec![CarvedValue::Text("Dovolená".into()), CarvedValue::Real(end), CarvedValue::Real(start), CarvedValue::Real(created)],
        )];
        let out = recover("calendar", &records, &LiveKeys::default());
        assert_eq!(out.len(), 1);
        // The earliest timestamp wins; crucially it is not the end date.
        assert!(out[0].date.as_deref().unwrap().starts_with("2020-01-06"));
    }

    #[test]
    fn safari_excludes_live_history_item_url() {
        let live_url = "https://live.example.com/";
        let mut live = LiveKeys::default();
        live.urls.insert(live_url.into());
        // The same URL carved from a free region is still live → dropped.
        let live_row = vec![rec_n(Some(2), CarveSource::Freelist, vec![CarvedValue::Text(live_url.into())])];
        assert!(recover("safari", &live_row, &live).is_empty());
        // A different (genuinely deleted) URL is kept.
        let gone = vec![rec_n(Some(3), CarveSource::Freelist, vec![CarvedValue::Text("https://gone.example.com/x".into())])];
        assert_eq!(recover("safari", &gone, &live).len(), 1);
    }

    #[test]
    fn dedup_prefers_complete_copy_over_truncated() {
        let g = "9B7E5F2A-1C3D-4E5F-8A9B-0C1D2E3F4A5B";
        let vals = || vec![CarvedValue::Text(g.into()), CarvedValue::Text("same body".into())];
        // The truncated copy is seen first; the later complete copy upgrades it.
        let truncated = CarvedRecord { rowid: Some(7), source: CarveSource::Freelist, values: vals(), truncated: true };
        let full = CarvedRecord { rowid: Some(7), source: CarveSource::Freeblock, values: vals(), truncated: false };
        let out = recover("messages", &[truncated, full], &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert!(!out[0].truncated);
    }

    #[test]
    fn dedup_drops_truncated_prefix_of_complete_same_row() {
        // Truncation shortened the body, so the two copies have different
        // summaries; the partial body is a prefix of the complete one, same
        // rowid → the truncated copy is dropped in favour of the complete one.
        let g = "9B7E5F2A-1C3D-4E5F-8A9B-0C1D2E3F4A5B";
        let partial = CarvedRecord { rowid: Some(7), source: CarveSource::Freelist, values: vec![CarvedValue::Text(g.into()), CarvedValue::Text("hello wor".into())], truncated: true };
        let full = CarvedRecord { rowid: Some(7), source: CarveSource::Freeblock, values: vec![CarvedValue::Text(g.into()), CarvedValue::Text("hello world".into())], truncated: false };
        let out = recover("messages", &[partial, full], &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary, "hello world");
        assert!(!out[0].truncated);

        // But a truncated row whose rowid has no complete counterpart is kept.
        let lonely = CarvedRecord { rowid: Some(9), source: CarveSource::Freelist, values: vec![CarvedValue::Text(g.into()), CarvedValue::Text("orphan".into())], truncated: true };
        let out = recover("messages", &[lonely], &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert!(out[0].truncated);
    }

    #[test]
    fn dedup_keeps_distinct_same_rowid_rows_with_different_dates() {
        // A reused rowid holds two real messages whose summaries prefix-match
        // ("OK" vs "OK thanks") but whose dates differ → both must be kept.
        let g1 = "11111111-1111-1111-1111-111111111111";
        let g2 = "22222222-2222-2222-2222-222222222222";
        let short = CarvedRecord { rowid: Some(5), source: CarveSource::Freelist, values: vec![CarvedValue::Text(g1.into()), CarvedValue::Int(600_000_000_000_000_000), CarvedValue::Text("OK".into())], truncated: true };
        let long = CarvedRecord { rowid: Some(5), source: CarveSource::Freeblock, values: vec![CarvedValue::Text(g2.into()), CarvedValue::Int(660_000_000_000_000_000), CarvedValue::Text("OK thanks".into())], truncated: false };
        let out = recover("messages", &[short, long], &LiveKeys::default());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn truncated_flag_propagates_to_recovered_record() {
        let g = "9B7E5F2A-1C3D-4E5F-8A9B-0C1D2E3F4A5B";
        let partial = CarvedRecord {
            rowid: Some(7),
            source: CarveSource::Freelist,
            values: vec![CarvedValue::Text(g.into()), CarvedValue::Text("partial body".into())],
            truncated: true,
        };
        let out = recover("messages", &[partial], &LiveKeys::default());
        assert_eq!(out.len(), 1);
        assert!(out[0].truncated);
        // A fully-recovered record reports false.
        let full = vec![rec_n(Some(8), CarveSource::Freelist, vec![CarvedValue::Text(g.into()), CarvedValue::Text("full".into())])];
        assert!(!recover("messages", &full, &LiveKeys::default())[0].truncated);
    }

    #[test]
    fn unknown_store_yields_nothing() {
        let records = vec![rec(CarveSource::Wal, vec![CarvedValue::Text("x".into())])];
        assert!(recover("photos", &records, &LiveKeys::default()).is_empty());
    }
}

//! A unified chronological timeline that merges events from the in-process
//! extractors (calls, messages media, photos, browsing, calendar, …) into one
//! sorted stream. This is a *view* over the other extractors, not a new data
//! store: each `from_*` builder maps a slice of one record type to `Event`s, and
//! [`finalize`] drops undated events and sorts the rest.
//!
//! All record date fields are ISO 8601 (RFC 3339) UTC strings produced by
//! [`crate::datetime`], so they share one fixed format and sort lexicographically
//! in chronological order — no timestamp parsing is needed here.

use serde::Serialize;

/// One point on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    /// ISO 8601 UTC timestamp (the event's primary time).
    pub timestamp: String,
    /// Short machine-stable category, e.g. `call`, `photo`, `safari`.
    pub kind: String,
    /// Human-readable one-line description.
    pub summary: String,
}

impl Event {
    fn new(timestamp: impl Into<String>, kind: &str, summary: String) -> Self {
        Event { timestamp: timestamp.into(), kind: kind.to_string(), summary }
    }
}

/// Truncate `s` to at most `n` characters (not bytes), appending `…` when cut.
/// Whitespace is collapsed so multi-line bodies stay on one timeline row.
fn trunc(s: &str, n: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= n {
        collapsed
    } else {
        let head: String = collapsed.chars().take(n).collect();
        format!("{head}…")
    }
}

/// A blank field rendered as a stable placeholder.
fn or_unknown(s: &str) -> &str {
    if s.is_empty() { "(unknown)" } else { s }
}

pub fn from_calls(calls: &[crate::calls::Call]) -> Vec<Event> {
    calls
        .iter()
        .map(|c| {
            Event::new(
                c.date.clone(),
                "call",
                format!("{} {} ({}s)", c.direction, or_unknown(&c.number), c.duration_seconds),
            )
        })
        .collect()
}

pub fn from_accounts(items: &[crate::accounts::Account]) -> Vec<Event> {
    items
        .iter()
        .map(|a| {
            let who = if a.username.is_empty() { or_unknown(&a.description) } else { &a.username };
            Event::new(
                a.date.clone(),
                "account",
                format!("added {} account ({})", a.account_type, who),
            )
        })
        .collect()
}

pub fn from_voicemail(items: &[crate::voicemail::Voicemail]) -> Vec<Event> {
    items
        .iter()
        .map(|v| {
            Event::new(
                v.date.clone(),
                "voicemail",
                format!("from {} ({}s)", or_unknown(&v.sender), v.duration_seconds),
            )
        })
        .collect()
}

pub fn from_voice_memos(items: &[crate::voice_memos::VoiceMemo]) -> Vec<Event> {
    items
        .iter()
        .map(|m| {
            let title = if m.title.is_empty() { "(untitled)" } else { &m.title };
            Event::new(m.date.clone(), "voice-memo", format!("{title} ({}s)", m.duration_seconds))
        })
        .collect()
}

pub fn from_safari_history(items: &[crate::safari::HistoryVisit]) -> Vec<Event> {
    items
        .iter()
        .map(|h| {
            let label = if h.title.is_empty() { h.url.clone() } else { h.title.clone() };
            Event::new(h.date.clone(), "safari", trunc(&label, 120))
        })
        .collect()
}

pub fn from_calendar(items: &[crate::calendar::CalendarEvent]) -> Vec<Event> {
    items
        .iter()
        .map(|e| {
            let summary = if e.calendar.is_empty() {
                e.summary.clone()
            } else {
                format!("{} [{}]", e.summary, e.calendar)
            };
            Event::new(e.start.clone(), "calendar", summary)
        })
        .collect()
}

pub fn from_notes(items: &[crate::notes::Note]) -> Vec<Event> {
    items
        .iter()
        .map(|n| {
            let title = if n.title.is_empty() { "(untitled)" } else { &n.title };
            let summary =
                if n.folder.is_empty() { title.to_string() } else { format!("{title} [{}]", n.folder) };
            Event::new(n.created.clone(), "note", summary)
        })
        .collect()
}

pub fn from_photos(items: &[crate::photos::Photo]) -> Vec<Event> {
    items
        .iter()
        .map(|p| {
            let name = if p.filename.is_empty() { or_unknown(&p.original_filename) } else { &p.filename };
            Event::new(p.created.clone(), "photo", name.to_string())
        })
        .collect()
}

pub fn from_attachments(items: &[crate::attachments::Attachment]) -> Vec<Event> {
    items
        .iter()
        .map(|a| {
            Event::new(
                a.created.clone(),
                "attachment",
                format!("{} ({})", or_unknown(&a.name), or_unknown(&a.mime_type)),
            )
        })
        .collect()
}

pub fn from_whatsapp(items: &[crate::whatsapp::WaMessage]) -> Vec<Event> {
    items
        .iter()
        .map(|m| {
            let arrow = if m.from_me { "→" } else { "←" };
            Event::new(
                m.date.clone(),
                "whatsapp",
                format!("{} {} {}", or_unknown(&m.chat), arrow, trunc(&m.text, 80)),
            )
        })
        .collect()
}

pub fn from_reminders(items: &[crate::reminders::Reminder]) -> Vec<Event> {
    items
        .iter()
        .filter_map(|r| {
            let created = r.created.clone()?;
            let title = if r.title.is_empty() { "(untitled)" } else { &r.title };
            Some(Event::new(created, "reminder", format!("{}: {title}", or_unknown(&r.list))))
        })
        .collect()
}

pub fn from_workouts(items: &[crate::health::Workout]) -> Vec<Event> {
    items
        .iter()
        .filter(|w| !w.start.is_empty())
        .map(|w| {
            let what = w
                .activity_type
                .clone()
                .or_else(|| w.activity_type_id.map(|id| format!("#{id}")))
                .unwrap_or_else(|| "workout".to_string());
            let dur = w.duration_seconds.map(|d| format!(" ({d}s)")).unwrap_or_default();
            Event::new(w.start.clone(), "workout", format!("{what}{dur}"))
        })
        .collect()
}

pub fn from_mail(items: &[crate::mail::MailMessage]) -> Vec<Event> {
    items
        .iter()
        .filter_map(|m| {
            let date = m.date.clone()?;
            Some(Event::new(
                date,
                "mail",
                format!("{} — {}", or_unknown(&m.subject), or_unknown(&m.from)),
            ))
        })
        .collect()
}

/// Drop undated events and sort the rest chronologically (ISO 8601 UTC strings
/// sort lexicographically in time order). The sort is stable, so events sharing
/// a timestamp keep their insertion order.
pub fn finalize(mut events: Vec<Event>) -> Vec<Event> {
    events.retain(|e| !e.timestamp.is_empty());
    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trunc_collapses_and_caps() {
        assert_eq!(trunc("a  b\nc", 10), "a b c");
        assert_eq!(trunc("hello world", 5), "hello…");
        // Counts characters, not bytes (no panic on multibyte).
        assert_eq!(trunc("ďďďďď", 3), "ďďď…");
    }

    #[test]
    fn finalize_drops_undated_and_sorts() {
        let events = vec![
            Event::new("2022-05-01T00:00:00+00:00", "b", "later".into()),
            Event::new("", "x", "undated".into()),
            Event::new("2020-01-01T00:00:00+00:00", "a", "earlier".into()),
        ];
        let out = finalize(events);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].summary, "earlier");
        assert_eq!(out[1].summary, "later");
    }

    #[test]
    fn finalize_is_stable_for_equal_timestamps() {
        let ts = "2021-01-01T00:00:00+00:00";
        let events = vec![
            Event::new(ts, "call", "first".into()),
            Event::new(ts, "photo", "second".into()),
        ];
        let out = finalize(events);
        assert_eq!(out[0].summary, "first");
        assert_eq!(out[1].summary, "second");
    }

    #[test]
    fn builders_map_dates_and_skip_optionals() {
        let calls = vec![crate::calls::Call {
            number: "+420123".into(),
            date: "2021-01-01T08:00:00+00:00".into(),
            duration_seconds: 42,
            direction: "incoming".into(),
            answered: true,
            service: "iMessage".into(),
            video: None,
            call_type: None,
            location: None,
            country: None,
        }];
        let ev = from_calls(&calls);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, "call");
        assert!(ev[0].summary.contains("+420123") && ev[0].summary.contains("42s"));

        // Reminders/mail with no date are skipped by their builders.
        let reminders = vec![crate::reminders::Reminder {
            list: "L".into(),
            title: "T".into(),
            notes: String::new(),
            due: None,
            completed: false,
            completed_date: None,
            priority: 0,
            created: None,
            flagged: false,
        }];
        assert!(from_reminders(&reminders).is_empty());

        let workouts = vec![crate::health::Workout {
            activity_type_id: Some(37),
            activity_type: Some("running".into()),
            start: "2021-06-01T06:00:00+00:00".into(),
            end: "2021-06-01T06:30:00+00:00".into(),
            duration_seconds: Some(1800),
            total_distance: None,
            total_energy_burned: None,
        }];
        let wev = from_workouts(&workouts);
        assert_eq!(wev.len(), 1);
        assert_eq!(wev[0].kind, "workout");
        assert!(wev[0].summary.contains("running"));
    }
}

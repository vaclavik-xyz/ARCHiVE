//! Read calendar events from an iOS `Calendar.sqlitedb` (Core Data SQLite).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// One calendar event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalendarEvent {
    /// Event title/summary; empty when unset.
    pub summary: String,
    /// Start time as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub start: String,
    /// End time as ISO 8601 UTC (Cocoa epoch); empty if unset/unconvertible.
    pub end: String,
    /// Whether this is an all-day event.
    pub all_day: bool,
    /// Owning calendar's title; empty when unresolved.
    pub calendar: String,
}

/// Parse every calendar event, joining the owning calendar's title and ordering
/// by start time. Optional columns (`end_date`, `all_day`) are tolerated when
/// absent across iOS versions.
///
/// Dates use the raw Cocoa→UTC conversion; all-day/floating events are stored
/// timezone-less by iOS and are not normalized here (consistent with the rest of
/// the tool).
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<CalendarEvent>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "CalendarItem")?;
    let end_sel = if cols.contains("end_date") { "ci.end_date" } else { "NULL" };
    let allday_sel = if cols.contains("all_day") { "ci.all_day" } else { "NULL" };

    let sql = format!(
        "SELECT ci.summary, ci.start_date, {end_sel}, {allday_sel}, c.title \
         FROM CalendarItem ci LEFT JOIN Calendar c ON c.ROWID = ci.calendar_id \
         ORDER BY ci.start_date"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let summary: Option<String> = row.get(0)?;
        let start: Option<f64> = row.get(1)?;
        let end: Option<f64> = row.get(2)?;
        let all_day: Option<i64> = row.get(3)?;
        let calendar: Option<String> = row.get(4)?;
        Ok(CalendarEvent {
            summary: summary.unwrap_or_default(),
            start: start.and_then(cocoa_to_iso).unwrap_or_default(),
            end: end.and_then(cocoa_to_iso).unwrap_or_default(),
            all_day: all_day == Some(1),
            calendar: calendar.unwrap_or_default(),
        })
    })?;
    rows.collect()
}

/// Build a customer-facing summary of the recovered calendar events.
pub fn summary(items: &[CalendarEvent]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};
    use std::collections::HashSet;

    let all_day = items.iter().filter(|e| e.all_day).count();
    let timed = items.iter().filter(|e| !e.all_day).count();
    let with_end = items.iter().filter(|e| !e.end.is_empty()).count();
    let calendars = items
        .iter()
        .filter(|e| !e.calendar.is_empty())
        .map(|e| e.calendar.clone())
        .collect::<HashSet<_>>()
        .len();
    let with_title = items.iter().filter(|e| !e.summary.is_empty()).count();
    let calendar_key = |e: &CalendarEvent| -> String {
        if e.calendar.is_empty() {
            "(neznámý)".to_string()
        } else {
            e.calendar.clone()
        }
    };

    Summary::new("calendar", "Kalendář", "událostí", items.len())
        .count("Celodenních", all_day)
        .count("Časovaných", timed)
        .count("S koncem", with_end)
        .count("Kalendářů", calendars)
        .count("S názvem", with_title)
        .period_from(iso_range(items.iter().map(|e| e.start.as_str())))
        .breakdown("Po letech", year_rows(items.iter().map(|e| e.start.as_str())))
        .breakdown("Podle kalendáře", tally(items.iter().map(calendar_key)))
        .note("Jen kalendáře uložené v zařízení; účty iCloud/Google nemusí být součástí zálohy.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_calendar;

    fn event(title: &str, start: &str, end: &str, all_day: bool, calendar: &str) -> CalendarEvent {
        CalendarEvent {
            summary: title.into(),
            start: start.into(),
            end: end.into(),
            all_day,
            calendar: calendar.into(),
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let events = vec![
            event("Standup", "2023-05-01T10:00:00+00:00", "2023-05-01T11:00:00+00:00", false, "Work"),
            event("Holiday", "2024-12-24T00:00:00+00:00", "", true, "Home"),
            event("", "", "", false, ""),
        ];
        let s = summary(&events);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "událostí");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("Celodenních"), Some(1));
        assert_eq!(get("Časovaných"), Some(2));
        assert_eq!(get("S koncem"), Some(1));
        assert_eq!(get("Kalendářů"), Some(2));
        assert_eq!(get("S názvem"), Some(2));
        let yr = s.breakdowns.iter().find(|b| b.title == "Po letech").unwrap();
        assert_eq!(yr.rows, vec![("2023".to_string(), 1), ("2024".to_string(), 1)]);
        let by_cal = s.breakdowns.iter().find(|b| b.title == "Podle kalendáře").unwrap();
        assert!(by_cal.rows.iter().any(|(name, _)| name == "(neznámý)"));
        assert!(s.period.is_some()); // derived from the two dated events
    }

    #[test]
    fn parses_events_joined_and_ordered() {
        let dir = std::env::temp_dir().join(format!("be-cal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Calendar.sqlitedb");
        let _ = std::fs::remove_file(&db);
        make_calendar(&db);

        let events = parse(&db).unwrap();
        assert_eq!(events.len(), 2);
        // Ordered by start_date ascending.
        assert_eq!(events[0].summary, "Standup");
        assert_eq!(events[0].start, "2020-01-06T10:40:00+00:00"); // Cocoa 600_000_000
        assert!(!events[0].all_day);
        assert_eq!(events[0].calendar, "Work");
        assert_eq!(events[1].summary, "Holiday");
        assert!(events[1].all_day);
        assert_eq!(events[1].calendar, "Home");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_when_optional_columns_absent() {
        let dir = std::env::temp_dir().join(format!("be-cal-min-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Calendar.sqlitedb");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE Calendar (ROWID INTEGER PRIMARY KEY, title TEXT);
             CREATE TABLE CalendarItem (ROWID INTEGER PRIMARY KEY, summary TEXT, start_date REAL, calendar_id INTEGER);
             INSERT INTO Calendar VALUES (1, 'Work');
             INSERT INTO CalendarItem VALUES (1, 'Solo', 600000000.0, 1);",
        )
        .unwrap();
        drop(conn);

        let events = parse(&db).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Solo");
        assert_eq!(events[0].end, ""); // no end_date column
        assert!(!events[0].all_day); // no all_day column → false
        assert_eq!(events[0].calendar, "Work");
        std::fs::remove_dir_all(&dir).ok();
    }
}

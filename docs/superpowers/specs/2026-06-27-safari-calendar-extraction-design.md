# Safari + Calendar Extraction — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) — three plain-SQLite extractors, no `archive-core` change
**Pattern:** mirrors the existing `calls` extractor (Core-Data SQLite, Cocoa epoch,
schema-tolerant column selection). No new dependencies.

## Goal

Add three commands that recover browsing and scheduling data from an iOS backup:

- `safari-history` — visited URLs with title, timestamp, and visit count.
- `safari-bookmarks` — saved bookmarks (title, URL, containing folder).
- `calendar` — calendar events (summary, start/end, all-day, calendar name).

All three are plain SQLite reads with Cocoa-epoch dates — structurally identical
to `calls`/`voicemail`, so they reuse the established parser → formatter → CLI →
inspect pattern verbatim. **Reminders are out of scope** (their on-backup storage
varies and is entangled with CalDAV/account stores; deferred to keep this correct).

## Sources (grounded)

### Safari history — `AppDomain-com.apple.mobilesafari` / `Library/Safari/History.db`

- `history_items(id INTEGER PK, url TEXT, visit_count INTEGER, …)`
- `history_visits(id INTEGER PK, history_item INTEGER→history_items.id,
  visit_time REAL, title TEXT, …)` — `visit_time` is the Cocoa/2001 epoch.
- One record per **visit**:
  `SELECT hi.url, hv.title, hv.visit_time, hi.visit_count
   FROM history_visits hv JOIN history_items hi ON hi.id = hv.history_item
   ORDER BY hv.visit_time`.

### Safari bookmarks — `AppDomain-com.apple.mobilesafari` / `Library/Safari/Bookmarks.db`

- `bookmarks(id INTEGER PK, title TEXT, url TEXT, parent INTEGER, type INTEGER,
  …)` — `type` 0 = leaf bookmark (has `url`), 1 = folder (no `url`).
- One record per leaf bookmark; the containing folder name is resolved by looking
  up `parent`'s title:
  `SELECT b.title, b.url, p.title AS folder
   FROM bookmarks b LEFT JOIN bookmarks p ON p.id = b.parent
   WHERE b.url IS NOT NULL AND b.url <> '' ORDER BY b.id`.

### Calendar — `HomeDomain` / `Library/Calendar/Calendar.sqlitedb`

- `CalendarItem(ROWID, summary TEXT, start_date REAL, end_date REAL,
  all_day INTEGER, calendar_id INTEGER→Calendar.ROWID, …)` — dates Cocoa/2001.
- `Calendar(ROWID, title TEXT, …)`.
- One record per event:
  `SELECT ci.summary, ci.start_date, ci.end_date, ci.all_day, c.title AS calendar
   FROM CalendarItem ci LEFT JOIN Calendar c ON c.ROWID = ci.calendar_id
   ORDER BY ci.start_date`.
- **Timezone caveat (documented):** `Calendar.sqlitedb` stores event dates in the
  Cocoa epoch but all-day/floating events use a timezone-less convention; we emit
  the raw Cocoa→UTC conversion (consistent with the rest of the tool) and note the
  caveat. No per-event timezone normalization (YAGNI).

All three parsers select optional columns via the existing `table_columns`
helper (`col(name)` → name-or-`NULL`), tolerating schema drift across iOS versions.

## CLI surface

```
archive --backup <dir> -o <out> safari-history   -f <csv|json|html>
archive --backup <dir> -o <out> safari-bookmarks -f <csv|json|html>
archive --backup <dir> -o <out> calendar         -f <csv|json|html>
```

- `-f` accepts `csv | json | html` (`vcf` rejected, exit 1), mirroring `calls`.
- `--out` required. Writes `<out>/safari-history.<ext>`,
  `<out>/safari-bookmarks.<ext>`, `<out>/calendar.<ext>`.
- Store absent → `count: 0`, `outputs: []`, `note`, exit 0 (clean success).

## Data models

```rust
// safari.rs
pub struct HistoryVisit { pub url: String, pub title: String,
                          pub date: String, pub visit_count: i64 }
pub struct Bookmark     { pub title: String, pub url: String, pub folder: String }
// calendar.rs
pub struct CalendarEvent { pub summary: String, pub start: String, pub end: String,
                           pub all_day: bool, pub calendar: String }
```

All derive `Debug, Clone, PartialEq, Serialize`. Dates are ISO 8601 UTC (empty
when unconvertible), exactly like `Call::date`.

## Formatters

For each type: `*_csv` (csv crate, explicit header), `*_json`
(`serde_json::to_string_pretty`), `*_html` (askama template). New templates:
`safari-history.html`, `safari-bookmarks.html`, `calendar.html`, each mirroring
`calls.html` (table; askama auto-escapes all cell data — URLs/titles are
user-controlled and rendered as text, never as raw HTML/links, so injection-safe).
CSV columns:

- history: `url, title, date, visit_count`
- bookmarks: `title, url, folder`
- calendar: `summary, start, end, all_day, calendar`

## inspect

Add to `KNOWN_STORES` (all `supported: true`):

- `("safari-history", true, "AppDomain-com.apple.mobilesafari", "Library/Safari/History.db")`
- `("safari-bookmarks", true, "AppDomain-com.apple.mobilesafari", "Library/Safari/Bookmarks.db")`
- `("calendar", true, "HomeDomain", "Library/Calendar/Calendar.sqlitedb")`

`count` filled best-effort via the matching `load_*` (parsed row count), `null`
on read failure — same contract as the others.

## Error handling

| Situation | Behavior |
|---|---|
| Store DB absent | `count: 0`, `outputs: []`, `note`, exit 0 |
| Present but unreadable/parse error | exit 1, `{ok:false, kind:"other"}` |
| `-f vcf` / unknown `-f` | usage error, exit 1 (before opening backup) |
| Encrypted backup, no/wrong password | exit 2, `kind:"auth"` |

## Testing

Per extractor, mirroring `calls`:

- Fixture builders in `test_fixtures.rs` (`make_safari_history`,
  `make_safari_bookmarks`, `make_calendar`) creating minimal real DBs with the
  join structure and Cocoa dates.
- Parser test: correct join, Cocoa→ISO conversion, ordering, schema-tolerance
  (optional columns absent).
- Formatter tests: csv header+row, json roundtrip, html lists item + escapes a
  crafted `<script>` URL/title to numeric entities.
- CLI tests: each command parses; `_format` rejects vcf; inspect lists the new
  stores as supported.

## Out of scope (YAGNI)

- Reminders (storage varies / CalDAV-entangled) — deferred, documented.
- Safari bookmark folder hierarchy beyond the immediate parent name.
- Recurring-event expansion, attendees, alarms (calendar) — raw event rows only.
- Notes (separate sub-project — binary blob decode).

## Global constraints (carried from the project)

- Agent-first contract: one JSON object on stdout; progress on stderr; exit
  `0`/`1`/`2`; clap errors the only non-JSON exit-2 case.
- Password: `--password` or `ARCHIVE_PASSWORD`; never prompts.
- No new dependency (plain rusqlite, already present).
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

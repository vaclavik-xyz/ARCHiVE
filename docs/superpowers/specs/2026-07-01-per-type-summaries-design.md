# Per-type recovery summaries — design

Date: 2026-07-01
Status: approved (design dialogue, user approved 2026-07-01)

## Motivation

The `photos --summary` report (a customer-facing "what did we recover" overview)
proved valuable on the macrecepce export workflow. The same overview makes sense
for every data type that yields a sizeable collection. We want:

1. **Per-type lightweight report** — a `<type>-summary.md` written automatically
   alongside each export, in plain **markdown** (zero dependencies, no headless
   Chrome, agent- and human-readable, renders on the Intel reception Mac).
2. **One root unified report** — `recover` additionally writes a single
   `summary.pdf` (+ `summary.md`) covering everything recovered: the "one paper"
   a customer reads to see what the backup holds.

Both are cheap: they aggregate records already loaded in memory; no extra media
copy.

## Approved decisions

- **Generic `Summary` model**, not 22× bespoke à la photos. One shared struct +
  one markdown renderer; each type writes only a small builder. Photos keeps its
  existing richer `--summary -f html|pdf` report and additionally gains a generic
  builder so it flows through the same per-folder/root path.
- **`messages`** gets a small in-`archive` `sms.db` stats reader (rusqlite, like
  `recover-deleted`'s `message_live_keys`), not the full `imessage-database`
  crate — archive deliberately does not re-implement message decoding.
- **Scope: 22 types.** Exclude `safari-bookmarks`, `apps`, `certificates`,
  `keyboard-lexicon` (flat lists with nothing meaningful to summarize).

## Architecture

New module `archive/src/summary.rs`:

```rust
pub struct Breakdown { pub title: String, pub rows: Vec<(String, usize)> }

pub struct Summary {
    pub data_type: String,          // "calls"
    pub title: String,              // "Hovory"
    pub total_label: String,        // "hovorů"
    pub total: usize,
    pub period: Option<(String, String)>, // Czech "D. M. YYYY" from/to
    pub counts: Vec<(String, usize)>,
    pub breakdowns: Vec<Breakdown>,
    pub notes: Vec<String>,
}

impl Summary {
    pub fn new(data_type, title, total_label, total) -> Self;
    pub fn count(self, label, n) -> Self;            // chainable
    pub fn breakdown(self, title, rows) -> Self;     // skips empty rows
    pub fn period_iso(self, from_iso, to_iso) -> Self; // cz_date both; skip if either empty
    pub fn note(self, text) -> Self;
}

// reusable aggregation helpers for builders:
pub fn tally(keys: impl IntoIterator<Item=String>) -> Vec<(String, usize)>; // count-desc, then name
pub fn year_rows<'a>(dates: impl IntoIterator<Item=&'a str>) -> Vec<(String, usize)>; // oldest-first, undated skipped
pub fn iso_range<'a>(dates: impl IntoIterator<Item=&'a str>) -> Option<(String, String)>; // min..max ISO, empties skipped

// rendering + IO:
pub fn summary_md(device: &DeviceInfo, generated_iso: &str, s: &Summary) -> String;
pub fn write_summary_md(out, generated_iso, device, s, outputs: &mut Vec<PathBuf>) -> io::Result<()>;
```

`cz_date` is promoted to `pub(crate)` in `format.rs` and reused.

### Markdown safety

Askama applies a **no-op escaper** to `.md`, so output is NOT auto-escaped.
Therefore `summary_md` builds the string directly (no askama template) and
sanitizes every dynamic value via `md_inline()`:
- collapse any whitespace/newline run to a single space, trim;
- escape ``\ * _ ` [ ] < > |`` and a leading `# > - + .` so arbitrary album /
  contact / folder names cannot corrupt structure.
No GFM tables — counts and breakdowns are rendered as `- Label: N` /
`- Name — N` lists, which are robust to arbitrary text.

### Markdown layout

```
# <title> — souhrn

**Zařízení:** <name> — <display_model> (<model>), iOS <ios>
**Vytvořeno:** <cz_date(generated)>

**Zachráněno:** <total> <total_label>
**Období:** <from> – <to>            # only when period present

## Přehled
- <count label>: <n>
...

## <breakdown title>
- <name> — <n>
...

> <note>                             # one blockquote line per note
```

### Per-folder emission (always-on)

Each recommended `run_<type>` (main.rs), after writing its main `<type>.<ext>`
and only when records were found (store-present path), builds its `Summary` and
calls `summary::write_summary_md(...)`, pushing the `.md` path into the envelope
`outputs[]`. Store-absent (`count:0` note) path writes nothing, as today.
`--no-files` / `--no-audio` do not suppress the summary (it is metadata-only);
media-recovered counts are simply omitted when extraction did not run.

### Root unified report (`recover`)

In `run_recover`, after the section loop and `generated` is computed (around the
`index.html` render), aggregate `rec.sections` (each carries `data_type`,
`label`, `count`, `media: Option<RecoverMedia>`):
- `recover::summary_sections(device, &generated, &rec.sections) -> String` (md)
  → write `<out>/summary.md`;
- the same content rendered through an HTML wrapper → `write_or_pdf(<out>/summary.pdf, Pdf)`.
  If no browser is available, **degrade gracefully**: keep `summary.md`, skip the
  PDF (log to stderr), do not fail the command.
Push both onto the recover `outputs`. Grand total = `sum(section.count)`; total
media recovered = sum over `section.media.extracted (+thumbnails)`.

## Per-type coverage (from the survey)

High value: messages, attachments, whatsapp, calls, safari-history,
photos-recently-deleted, interactions, device-usage.
Medium: contacts, voicemail, voice-memos, calendar, notes, reminders, health,
mail, accounts, significant-locations, bluetooth-devices, homescreen-layout,
data-usage. Plus known-networks.

Each builder uses real struct fields only (see survey output for exact fields).
Representative shapes:

- **calls** — total; incoming/outgoing/missed/answered; talk-time sum; phone vs
  FaceTime; distinct numbers. Period from `date`. Breakdowns: by year, by
  service, by direction, top contacts (`contact_name`||`number`).
- **attachments** — total; image/video/audio/other; total bytes; recoverable vs
  missing (`source_path.is_empty()`). Period `created`. Breakdowns: by media
  type, by year, by size bucket, by extension.
- **whatsapp** — total; sent/received; with text; with media; conversations.
  Period `date`. Breakdowns: by year, by conversation, by direction, by media type.
- **contacts** — total; with phone/email/address; company-only; with note. No
  period. Breakdowns: by phone label, by email label, by org, by country.
- **notes** — total; decoded/snippet/empty quality; titled; foldered. Period
  `created`. Breakdowns: by year, by folder, by recovery quality.
- **messages** (special) — read `Library/SMS/sms.db` directly: total; sent vs
  received (`is_from_me`); iMessage vs SMS (`service`); with attachments
  (`num_attachments`); distinct chats. Period from `date` (Cocoa ns).
  Breakdowns: by year, by service, by direction.

(Remaining types follow the same recipe; field-level detail in the survey
result `w3jghukcc`.)

## Files touched

- `archive/src/summary.rs` (new) — model, helpers, renderer, write helper, tests.
- `archive/src/format.rs` — `cz_date` → `pub(crate)`.
- `archive/src/<type>.rs` (×22) — `pub fn summary(&records, device) -> Summary` + unit test.
- `archive/src/messages.rs` (+ a small `sms.db` reader) — message stats.
- `archive/src/main.rs` — `mod summary;`; per-type emit call in each `run_<type>`;
  recover root aggregation; clap/integration tests.
- `archive/src/recover.rs` — `summary_sections` aggregator + tests.
- `AGENTS.md`, `archive/README.md`, `README.md` — document the `-summary.md`
  side artifact, the `recover` `summary.pdf`/`.md`, and the unchanged envelope.

## Testing

Inline `#[cfg(test)] mod tests` per module (no `tests/` dir). For each builder:
assert totals/labels/breakdown rows from a small fixture vector. For
`summary_md`: assert section headings + a count line render; assert a pipe /
newline in a name does not break a line (sanitization guard). For recover:
assert the unified report lists each section + grand total. Real-backup checks
stay gated behind `ARCHIVE_TEST_BACKUP`.

## Implementation phases

1. **Foundation** — `summary.rs` (model + helpers + `summary_md` + sanitizer +
   write helper), `cz_date` promotion, `mod summary;`. TDD. Must compile + green.
2. **Per-type builders** — one per `<type>.rs`, TDD, isolated files (parallel-safe).
3. **Wiring** — per-type emit in `run_<type>` (sequential, main.rs).
4. **messages reader** — `sms.db` stats + builder + emit.
5. **Recover aggregation** — root `summary.md` + `summary.pdf` (graceful no-browser).
6. **Docs + verify** — AGENTS.md/READMEs; `cargo test` + `clippy`; adversarial
   review; fix; commit; roborev.

## Out of scope (YAGNI)

- `--no-summary` opt-out flag (add only if the extra file proves noisy).
- Refactoring photos' bespoke summary onto the generic model (later cleanup).
- Per-section deep breakdowns inside the root `summary.pdf` (section-level
  totals first; drill-down later).

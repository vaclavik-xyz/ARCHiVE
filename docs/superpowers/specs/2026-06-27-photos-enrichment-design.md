# Photos Enrichment — Albums, Hidden, Metadata, Live/Burst — Design

**Status:** Approved (2026-06-27) — autonomous build (follow-up to program #3)
**Component:** `archive` (extends `photos.rs`, its formatters/template, and the
`make_photos` fixture). No new dependency.

## Goal

Extend the `photos` extractor so the export reflects how the user actually
organizes the Camera Roll: **album membership**, the **hidden** flag, richer
**metadata** (modified/added dates, edited, original filename, title), and
**Live Photo / burst** grouping. Today the extractor reads `ZASSET` as a flat
list and surfaces only a basic subset; hidden photos are recovered but
indistinguishable, and albums are absent.

## Grounded schema (Photos.sqlite)

All additions are **schema-tolerant**: each column is selected only when
`table_columns` reports it, and version-dependent values are also surfaced raw so
a wrong interpretation never loses data (the `call_type`/`video` precedent).

### New `ZASSET` columns

| field | column | notes |
|---|---|---|
| hidden | `ZHIDDEN` | 0/1 — in the Hidden album |
| modified | `ZMODIFICATIONDATE` | Cocoa → ISO |
| added | `ZADDEDDATE` | Cocoa → ISO (added to library) |
| edited | `ZHASADJUSTMENTS` | 0/1 — has edits/adjustments |
| kind_subtype | `ZKINDSUBTYPE` | raw subtype integer |
| burst_id | `ZAVALANCHEUUID` | same UUID ⇒ same burst; `None` when empty |

Derived: `live_photo = (kind == "image") && (kind_subtype == 2)` — best-effort
(the Live-Photo subtype is version-dependent), with `kind_subtype` kept raw.

### `ZADDITIONALASSETATTRIBUTES` join (per asset)

`ZASSET.ZADDITIONALATTRIBUTES` → `ZADDITIONALASSETATTRIBUTES.Z_PK`. Selected when
both the FK and the target table exist:

- `original_filename` ← `ZORIGINALFILENAME` (the name before any iCloud rename).
- `title` ← best-effort caption: `ZTITLE` when that column exists, else empty.

### Album membership (`ZGENERICALBUM` + dynamic join)

iOS stores album↔asset as a Core-Data many-to-many in a table named
`Z_<N>ASSETS` whose number `N` is schema-version-dependent. Discovery at runtime:

1. From `sqlite_master`, list tables matching `^Z_\d+ASSETS$`.
2. The album-asset join is the one whose columns include a `Z_\d+ALBUMS` column
   (the album FK) **and** a `Z_\d+ASSETS` column (the asset FK). Pick the first
   such table; capture `(table, album_col, asset_col)`.
3. Build `asset_pk → Vec<album_title>` via
   `SELECT j.<asset_col>, a.ZTITLE FROM <table> j
    JOIN ZGENERICALBUM a ON a.Z_PK = j.<album_col>
    WHERE a.ZTITLE IS NOT NULL AND a.ZTITLE <> ''`.

No join table / no `ZGENERICALBUM` → empty map (assets simply have no albums). A
photo can be in several albums (`albums: Vec<String>`, sorted, deduped).

## Data model (`photos::Photo` gains)

```rust
pub hidden: bool,
pub edited: bool,
pub live_photo: bool,
pub kind_subtype: Option<i64>,     // raw ZKINDSUBTYPE
pub modified: String,              // ISO 8601 UTC; empty if unset
pub added: String,                 // ISO 8601 UTC; empty if unset
pub original_filename: String,     // empty when unknown
pub title: String,                 // caption; empty when none
pub burst_id: Option<String>,      // ZAVALANCHEUUID; None when not a burst
pub albums: Vec<String>,           // album titles (sorted, deduped)
```

Existing fields are unchanged; this is purely additive (CSV/JSON gain columns,
HTML gains markers). `parse` now also selects `Z_PK` (to key the album map) — not
serialized.

## Parser flow

1. `album_membership(conn) -> HashMap<i64, Vec<String>>` (discovery + join above).
2. Main `ZASSET` query selects the new columns (schema-tolerant) plus `Z_PK` and
   the `ZADDITIONALASSETATTRIBUTES` join; for each row, attach
   `albums = map.get(&pk)` (sorted+deduped).

## Formatters

- **CSV** appends columns: `hidden, edited, live_photo, modified, added,
  original_filename, title, burst_id, albums` (albums joined by `; `).
- **JSON**: serde derive (all new fields; `Z_PK` not included).
- **HTML gallery** (`photos.html`): the caption line gains compact markers —
  🙈 hidden, ✎ edited, ◉ Live, and the album names; a burst shows its short id.
  All askama-escaped.

## Behavior / edge cases

| Situation | Behavior |
|---|---|
| Album join table absent / unknown schema | `albums: []` (no error) |
| `ZADDITIONALATTRIBUTES` FK or table absent | `original_filename`/`title` empty |
| `ZHIDDEN`/`ZHASADJUSTMENTS`/… column absent | field defaults (false/empty/None) |
| `ZKINDSUBTYPE` absent | `kind_subtype: null`, `live_photo: false` |
| Hidden photo | still extracted (recovered) **and** `hidden: true` |

Extraction (`extract_photos`) is unchanged — every asset's file is still fetched;
hidden/album status only enriches metadata, never excludes a file.

## Testing

- `album_membership`: fixture with a `Z_28ASSETS` join (`Z_28ALBUMS`,`Z_3ASSETS`)
  + `ZGENERICALBUM` → asserts an asset maps to its album title(s); a fixture with
  no join table → empty map.
- Parser: extend `make_photos` so one asset is hidden, one edited, one a Live
  Photo (`ZKINDSUBTYPE=2`), two share a `ZAVALANCHEUUID` (burst), one has an
  `original_filename`/title and album membership → assert every new field.
- Schema-tolerance: a minimal `ZASSET` lacking the new columns still parses (new
  fields default).
- Formatters: csv header includes the new columns; json roundtrips a new field;
  html shows a hidden/Live marker and an album name, and escapes a crafted album
  title.
- Existing photos tests stay green (additive change).

## Out of scope (YAGNI)

- Physically pairing a Live Photo's `.MOV` sidecar to its still (version-dependent
  storage; the `live_photo` flag + `burst_id` give the grouping signal).
- Smart-album / folder hierarchy semantics (only titled `ZGENERICALBUM` rows).
- Faces/people, keywords, moments, memories.
- Per-album output subfolders (the `albums` field + HTML markers suffice; a future
  `--by-album` could lay files out per album).

## Global constraints (carried from the project)

- Agent-first contract unchanged; additive envelope only.
- No new dependency; schema-tolerant; version-dependent values kept raw.
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

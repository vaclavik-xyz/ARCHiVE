# Apple Notes Extraction — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) — one extractor with a binary-blob body decoder
**New dependency:** `flate2` (gzip) — already in the workspace lock (1.1.9), so no
new resolution; pinned like the other deps.

## Goal

Add an `archive notes` command that recovers Apple Notes from an iOS backup:
each note's title, folder, created/modified timestamps, and **body text**. Unlike
the other extractors, the note body is not a plain column — it is a
gzip-compressed protobuf blob — so this sub-project owns a small, well-tested
binary decoder with a graceful fallback to the plaintext snippet.

## Source (grounded)

`AppDomainGroup-group.com.apple.notes` / `NoteStore.sqlite` (modern Core Data
Notes, iOS 9+). Two tables matter:

- `ZICCLOUDSYNCINGOBJECT` — the polymorphic object table. **Note** rows carry:
  - `ZTITLE1` — note title (plaintext).
  - `ZSNIPPET` — first-line preview (plaintext; the body fallback).
  - `ZNOTEDATA` — FK → `ZICNOTEDATA.Z_PK` (NON-NULL only for note rows; this is
    the filter that selects notes from folders/accounts).
  - `ZFOLDER` — FK → the folder's own row in the same table (its `ZTITLE2` is the
    folder name).
  - `ZCREATIONDATE`, `ZMODIFICATIONDATE1` — Cocoa/2001 epoch.
- `ZICNOTEDATA` — `Z_PK`, `ZDATA` (gzip-compressed protobuf = the rich note
  body), `ZNOTE` (back-reference).

Selection query (all non-key columns selected schema-tolerantly via
`table_columns`, since column suffixes drift across iOS versions):

```sql
SELECT c.ZTITLE1, c.ZSNIPPET, f.ZTITLE2, c.ZCREATIONDATE, c.ZMODIFICATIONDATE1, d.ZDATA
FROM ZICCLOUDSYNCINGOBJECT c
LEFT JOIN ZICNOTEDATA d ON d.Z_PK = c.ZNOTEDATA
LEFT JOIN ZICCLOUDSYNCINGOBJECT f ON f.Z_PK = c.ZFOLDER
WHERE c.ZNOTEDATA IS NOT NULL
ORDER BY c.ZMODIFICATIONDATE1 DESC
```

If `ZNOTEDATA` is absent from the schema entirely (very old/foreign DB), the
parser returns an empty list (nothing recognizable as a note) rather than erroring.

## Body decode (the novel part)

`ZICNOTEDATA.ZDATA` = gzip(protobuf). The protobuf is Apple's `NoteStoreProto`;
the plain text lives at field path **2 → 3 → 2** (`document` → `note` →
`note_text`). Decode pipeline:

1. **Decompress**: try gzip (`flate2::read::GzDecoder`); if that fails, try zlib
   (`ZlibDecoder`). On failure → `None`.
2. **Walk protobuf**: a minimal reader that, given a message's bytes and a field
   number, returns the bytes of the first length-delimited (wire-type-2) field
   with that number, skipping all other fields/wire-types. Apply it three times:
   `field(2)` → `field(3)` → `field(2)`; interpret the result as UTF-8
   (lossy). Empty/whitespace-only → `None`.
3. **Fallback**: when decode yields `None`, use `ZSNIPPET`. When the snippet is
   also empty, body is `""`.

`body_source` records which path produced the body: `"decoded"` (full text),
`"snippet"` (preview fallback), or `"empty"`. This keeps the output honest — a
consumer can see when only a preview was recoverable.

The protobuf reader is **pure and unit-tested** with a hand-built
`gzip(nested protobuf)` fixture, independent of any real backup. It never panics
on malformed input (every read is bounds-checked and returns `None`).

## CLI surface

```
archive --backup <dir> -o <out> notes -f <csv|json|html>
```

`-f` accepts `csv | json | html` (`vcf` rejected). Writes `<out>/notes.<ext>`.
Store absent → `count: 0`, `outputs: []`, `note`, exit 0.

## Data model

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    pub title: String,        // ZTITLE1; empty when untitled
    pub folder: String,       // folder ZTITLE2; empty when none
    pub created: String,      // ISO 8601 UTC (Cocoa); empty if unconvertible
    pub modified: String,     // ISO 8601 UTC (Cocoa); empty if unconvertible
    pub body: String,         // decoded text, snippet fallback, or ""
    pub body_source: String,  // "decoded" | "snippet" | "empty"
}
```

## Formatters

- **CSV** columns: `title, folder, created, modified, body_source, body`.
- **JSON**: serde derive.
- **HTML** (`archive/templates/notes.html`): a table; the body cell renders the
  text inside a `<pre>` (preserves note line breaks), askama-escaped — bodies are
  attacker-influenced free text, so escaping is essential and asserted by a test.

## inspect

Flip the existing placeholder `notes` entry to supported and point it at the real
location:
`("notes", true, "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite")`.
`count` = parsed note rows (best-effort).

## Error handling

| Situation | Behavior |
|---|---|
| No NoteStore.sqlite | `count: 0`, `outputs: []`, `note`, exit 0 |
| `ZNOTEDATA` column absent | empty list → `count: 0` |
| Body blob missing / undecodable | fall back to snippet, `body_source` reflects it |
| Present but unreadable DB | exit 1, `kind:"other"` |
| `-f vcf` / unknown | usage error, exit 1 |

## Testing

- Protobuf reader: unit tests for varint, field skipping (varint/64-bit/32-bit
  fields before the target), nested path extraction, and bounds-safety on
  truncated input (returns `None`, no panic).
- Body decode end-to-end: build `gzip(protobuf(2→3→2 = "Hello\nWorld"))` in the
  test, assert `decode_body` returns `"Hello\nWorld"`; assert a non-gzip blob →
  `None`.
- Parser: fixture `make_notes` with a note row + ZICNOTEDATA blob + a folder row
  → asserts title/folder/dates(Cocoa)/body=="decoded text", and a second note
  with no decodable body falls back to its snippet (`body_source == "snippet"`).
- Formatters: csv header+row, json roundtrip, html escapes a crafted `<script>`
  body.
- CLI: `notes` parses; `export_format` rejects vcf; inspect lists notes supported.

## Out of scope (YAGNI)

- Rich formatting (bold/links/checklists), inline images/attachments, tables — we
  recover the plain text run only. Attachments are a separate program feature.
- Encrypted/password-protected notes (the protobuf is itself encrypted) — these
  decode to `None` and fall back to snippet; not decrypted.
- Deleted/recently-deleted notes as a separate view.

## Global constraints (carried from the project)

- Agent-first contract: one JSON object on stdout; progress on stderr; exit
  `0`/`1`/`2`; clap errors the only non-JSON exit-2 case.
- Password: `--password` or `ARCHIVE_PASSWORD`; never prompts.
- New dependency limited to `flate2` (gzip), already in the workspace lock; the
  protobuf reader is hand-written (no protobuf crate) to keep the surface tiny.
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

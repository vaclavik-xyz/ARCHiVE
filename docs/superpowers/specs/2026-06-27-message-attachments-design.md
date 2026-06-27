# Message Attachments Extraction — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) — DB metadata + binary file extraction (same shape
as `photos`, reusing the now-streaming/atomic `Backup::fetch`)

## Goal

Add an `archive attachments` command that recovers the media sent/received in
Messages (iMessage/SMS): each attachment's original name, type, date, and the
file itself, with an HTML gallery. The `archive` CLI does not (yet) export message
*text* — this recovers the attachment *files*, which is the high-value media a
customer wants back.

## Source (grounded)

- **Metadata DB:** `HomeDomain` / `Library/SMS/sms.db`, table `attachment`:
  `filename` (on-device path), `mime_type`, `transfer_name` (the original
  filename), `total_bytes`, `created_date` (Cocoa epoch — see below). Read
  schema-tolerantly via `table_columns`.
- **Files:** `MediaDomain` / `Library/SMS/Attachments/...`. The `attachment.filename`
  column stores a path like `~/Library/SMS/Attachments/ab/12/GUID/IMG.jpg` or
  `/var/mobile/Library/SMS/Attachments/...`; the `MediaDomain` relative path is
  the substring from `Library/SMS/Attachments/` onward.

**Epoch:** `created_date` may be Cocoa **seconds** or **nanoseconds** depending on
iOS version. A new `datetime::cocoa_any_to_iso` detects nanoseconds (|value| ≥
1e12 → divide by 1e9) and converts; reused by future message extraction.

## CLI surface

```
archive --backup <dir> -o <out> attachments -f <csv|json|html> [--no-files]
```

- **Files extracted by default** into `<out>/attachments/` (the media is the
  point), `--no-files` for a metadata-only catalog — same contract as `photos`.
- `-f` accepts `csv | json | html` (`vcf` rejected). Writes `<out>/attachments.<ext>`.
- `--out` required. Store absent (no `sms.db`) → `count: 0`, `outputs: []`, `note`.

## Data model

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attachment {
    pub name: String,         // transfer_name (original), else basename of filename
    pub mime_type: String,    // e.g. image/jpeg; empty when unknown
    pub created: String,      // ISO 8601 UTC (Cocoa s or ns); empty if unconvertible
    pub total_bytes: i64,     // size per the DB (0 when unknown)
    pub source_path: String,  // MediaDomain-relative source (Library/SMS/Attachments/…)
    pub file: Option<String>, // output-relative extracted path, or None
}
```

Rows whose `filename` has no recoverable `Library/SMS/Attachments/` path get an
empty `source_path` and are never fetched (they still appear in the catalog).
Ordered by `created_date`.

## Extraction (`attachments.rs`)

Mirrors `photos::extract_photos` (DB-driven, no synthesis):

```rust
pub struct AttachmentSummary { pub dir: String, pub extracted: usize, pub missing: usize }
pub fn extract_attachments(backup, items: &mut [Attachment], out) -> std::io::Result<AttachmentSummary>;
```

Per row with a non-empty `source_path`: output name `<n>_<basename(name)>`
(1-based index for uniqueness, original name preserved); `fetch(MediaDomain,
source_path, <out>/attachments/<name>)`; set `file` on success. `extracted` =
rows with a `file`; `missing` = rows without (`extracted + missing == count`).
`--no-files` skips extraction (no dir, no `files` envelope).

## Output envelope

```json
{
  "ok": true, "command": "attachments", "count": 312,
  "outputs": ["<out>/attachments.json"],
  "files": { "dir": "attachments", "extracted": 300, "missing": 12 },
  "device": { ... }
}
```

`files` present only when extraction ran. Store absent → `count: 0`, `note`.

## Formatters

- **CSV** columns: `name, mime_type, created, total_bytes, file`.
- **JSON**: serde derive.
- **HTML** (`archive/templates/attachments.html`): a gallery — `image/*` mime
  types as a lazy `<img>` linking the file; everything else as a link with the
  name + mime. askama escapes name/path (injection-safe; asserted by a test).

## inspect

Add `("attachments", true, "HomeDomain", "Library/SMS/sms.db")`; `count` = parsed
`attachment` rows (best-effort).

## Error handling

| Situation | Behavior |
|---|---|
| No `sms.db` | `count: 0`, `outputs: []`, `note`, exit 0 |
| Attachment file absent in backup | `file: null`, `missing++`, continue |
| `filename` without an Attachments path | empty `source_path`, never fetched |
| Present but unreadable DB | exit 1, `kind:"other"` |
| `-f vcf` / unknown | usage error, exit 1 |

## Testing

- `cocoa_any_to_iso`: unit tests for a seconds value and a nanoseconds value
  mapping to the same instant; out-of-range → None.
- Parser: fixture `make_sms_attachments` (an image with transfer_name + ns date, a
  non-image with a `/var/mobile/` prefixed path, a row whose filename lacks the
  Attachments path) → asserts name/mime/created/source_path mapping and the
  unrecoverable-path row gets empty `source_path`.
- Extraction: gated end-to-end test (`extracted + missing == count`, linked files
  exist); plus an output-name unit test.
- Formatters: csv header+row, json roundtrip, html emits `<img>`/link + escapes a
  crafted name.
- CLI: `attachments` parses with/without `--no-files`; `export_format` rejects
  vcf; inspect lists attachments supported.

## Out of scope (YAGNI)

- Linking attachments to their message/conversation/sender (that needs the full
  message join — a separate feature; the `imessage-exporter` crate already does
  rich message export).
- Deduplication of identical attachments; thumbnailing/transcoding.
- Sticker/audio-message special handling beyond mime type.

## Global constraints (carried from the project)

- Agent-first contract; password via flag/env; never prompts.
- No new dependency (rusqlite + existing `Backup` API + the new datetime helper).
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

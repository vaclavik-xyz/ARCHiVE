# `recover` — One-Shot Customer Package — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI capstone) + a small `archive-core::DeviceInfo`
enrichment. Depends on the existing extractors (#1–#4 + contacts/calls/voicemail).

## Goal

Add an `archive recover` command that runs **every** supported extractor in one
shot into a single output folder, then writes a customer-facing `index.html`
landing page — a device/summary sheet plus links to each export and media folder.
This turns the per-type command set into a one-command, shop-ready deliverable.

## Device sheet enrichment (`archive-core`)

`DeviceInfo` today carries `device_name`, `product_version`, `udid`. Add two
fields from crabapple's lockdown record (already parsed): `serial` (`serial_number`)
and `model` (`product_type`, e.g. `iPhone14,2`). `device_json` in `main.rs` gains
`serial` and `model` for **every** command's `device` object (additive,
backward-compatible). The repair-relevant identity (model, serial) is then on
every envelope and on the recovery sheet.

## CLI surface

```
archive --backup <dir> -o <out> recover [--no-files]
```

- Runs all extractors into `<out>/`, producing one **HTML** file per data type
  (the customer-facing format) plus the media folders, plus `<out>/index.html`.
- `--no-files`: metadata only — skips the large media extraction
  (`voice_memos/`, `photos/`, `attachments/`, and voicemail audio).
- `--out` required. Encrypted backups: `--password`/`ARCHIVE_PASSWORD`.
- The backup is opened **once** and shared across all extractors (no repeated
  decrypt-key derivation).
- `--zip` is **out of scope** (would add a compression dependency); the `<out>`
  folder is the deliverable. Noted as a future option.

## Behavior

For each data type, `recover` loads it via the existing `load_*` helper, renders
HTML via the existing `format::*_html`, writes `<out>/<type>.html`, and (for media
types, unless `--no-files`) extracts files via the existing `extract_*`:

| type | html file | media |
|---|---|---|
| contacts | contacts.html | — |
| calls | calls.html | — |
| voicemail | voicemail.html | voicemail_audio/ (raw `.amr`) |
| voice-memos | voice-memos.html | voice_memos/ |
| safari-history | safari-history.html | — |
| safari-bookmarks | safari-bookmarks.html | — |
| calendar | calendar.html | — |
| notes | notes.html | — |
| photos | photos.html | photos/ |
| attachments | attachments.html | attachments/ |

A store absent from the backup is **skipped** (no file, not listed as an error) —
the section simply does not appear. Each extractor's failure is isolated: a parse
error on one type is logged to stderr and that section is skipped, so one bad
store never aborts the whole recovery (best-effort, like the media extractors).

## `index.html`

- **Device sheet:** name, model, iOS, serial, UDID (from the enriched `DeviceInfo`).
- **Sections table:** one row per recovered type — label, item count, a link to
  its `<type>.html`, and (for media types) a link to the media folder with the
  extracted/missing counts.
- Generation timestamp (UTC) for the record.
- New template `archive/templates/recover-index.html` (askama). All dynamic
  values are askama-escaped.

## Output envelope

```json
{
  "ok": true, "command": "recover",
  "outputs": ["<out>/index.html", "<out>/contacts.html", ...],
  "sections": [
    { "type": "contacts", "count": 1234, "file": "contacts.html" },
    { "type": "photos", "count": 1240, "file": "photos.html",
      "files": { "dir": "photos", "extracted": 1238, "missing": 2 } }
  ],
  "device": { "name": "...", "model": "iPhone14,2", "ios": "17.5",
              "serial": "...", "udid": "..." }
}
```

`outputs[0]` is always `index.html`. An empty backup (no supported stores) yields
`sections: []` and an index with just the device sheet.

## Data model (`recover.rs`)

```rust
pub struct RecoverSection {
    pub data_type: String,    // "contacts", "photos", …
    pub label: String,        // human label for the index
    pub file: String,         // "<type>.html"
    pub count: usize,
    pub media: Option<RecoverMedia>, // present for media types that extracted
}
pub struct RecoverMedia { pub dir: String, pub extracted: usize, pub missing: usize }
```

`recover.rs` owns the index rendering (`render_index(device, sections, generated)
-> String`) and the section list type; the per-type orchestration (load → render
→ write → extract) lives in `run_recover` in `main.rs`, reusing every existing
`load_*`/`format::*_html`/`extract_*` so there is no duplicated parsing/rendering.

## inspect

No change (recover is an action, not a store). `recover` is documented in
AGENTS.md as the one-shot command.

## Testing

- `DeviceInfo`: the gated real-backup test additionally asserts `model`/`serial`
  are exposed (non-fatal if empty).
- `device_json`: unit test asserts the object includes `model` and `serial`.
- `render_index`: unit test with a fixed device + a couple of sections (one media,
  one not) asserts the device sheet fields, the section rows/links/counts, and
  that a crafted `label`/device value is HTML-escaped.
- `RecoverSection`/`RecoverMedia` serialize into the envelope shape (serde).
- CLI: `recover` parses with/without `--no-files`; `--out` required.
- Gated end-to-end: against a real backup, `recover` writes `index.html` + at
  least the present sections, and the envelope `outputs[0]` ends with `index.html`.

## Out of scope (YAGNI)

- `--zip` packaging (compression dependency) — deferred.
- Message *text* export (separate; `imessage-exporter` covers rich messages).
- Per-type format choice (recover is the customer HTML package; the individual
  commands still offer csv/json/html).
- Thumbnails, PDF, localization of the index beyond the existing Czech UI strings.

## Global constraints (carried from the project)

- Agent-first contract: one JSON object on stdout; progress on stderr; exit
  `0`/`1`/`2`; clap errors the only non-JSON exit-2 case.
- Password via flag/env; never prompts.
- No new dependency (reuses chrono, already present, for the timestamp).
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

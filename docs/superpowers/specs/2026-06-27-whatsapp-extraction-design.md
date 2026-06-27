# WhatsApp Extraction — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) — DB (3-table join) + media file extraction, same
shape as `attachments`/`photos`. No new dependency.

## Goal

Add an `archive whatsapp` command that recovers WhatsApp messages and media from an
iOS backup: each message's chat, sender, direction, timestamp, text, and any
attached media file, with an HTML transcript. The most-requested third-party app in
a recovery context.

## Source (grounded)

WhatsApp stores data in its shared app-group container:

- **Domain:** `AppDomainGroup-group.net.whatsapp.WhatsApp.shared`.
- **DB:** `ChatStorage.sqlite` (Core Data). Relevant tables (read schema-tolerantly
  via `table_columns`; `Z*` suffixes drift by version):
  - `ZWAMESSAGE` — `ZTEXT` (body), `ZMESSAGEDATE` (Cocoa seconds), `ZISFROMME`
    (0/1), `ZFROMJID` (sender JID), `ZCHATSESSION` (FK → chat), `ZMEDIAITEM`
    (FK → media).
  - `ZWACHATSESSION` — `Z_PK`, `ZPARTNERNAME` (chat/contact display name),
    `ZCONTACTJID`.
  - `ZWAMEDIAITEM` — `Z_PK`, `ZMEDIALOCALPATH` (media path within the container).
- **Media files:** under `Message/Media/...` in the same domain. `ZMEDIALOCALPATH`
  is typically `Media/<chat>/<x>/<y>/<file>`; the domain-relative path is that with
  a `Message/` prefix. Mapping (best-effort, documented): if it already starts with
  `Message/` use as-is; if it starts with `Media/` prepend `Message/`; else use
  as-is. Unresolvable/empty paths are never fetched (the message still appears).

Join (one record per message, newest-ordering by date):

```sql
SELECT m.ZTEXT, m.ZMESSAGEDATE, m.ZISFROMME, m.ZFROMJID, s.ZPARTNERNAME, md.ZMEDIALOCALPATH
FROM ZWAMESSAGE m
LEFT JOIN ZWACHATSESSION s ON s.Z_PK = m.ZCHATSESSION
LEFT JOIN ZWAMEDIAITEM   md ON md.Z_PK = m.ZMEDIAITEM
ORDER BY m.ZMESSAGEDATE
```

`ZMESSAGEDATE` is the Cocoa/2001 epoch (seconds); converted with the
seconds-or-nanoseconds-tolerant `datetime::cocoa_any_to_iso`.

## CLI surface

```
archive --backup <dir> -o <out> whatsapp -f <csv|json|html> [--no-files]
```

- **Media extracted by default** into `<out>/whatsapp_media/`; `--no-files` for a
  metadata-only transcript — same contract as `attachments`/`photos`.
- `-f` accepts `csv | json | html` (`vcf` rejected). Writes `<out>/whatsapp.<ext>`.
- Store absent (no `ChatStorage.sqlite`) → `count: 0`, `outputs: []`, `note`.

## Data model

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WaMessage {
    pub chat: String,        // ZPARTNERNAME (chat/contact name); empty when unknown
    pub sender: String,      // ZFROMJID; empty when from me / unknown
    pub from_me: bool,       // ZISFROMME
    pub date: String,        // ISO 8601 UTC (Cocoa); empty if unconvertible
    pub text: String,        // ZTEXT; empty for media-only messages
    pub source_path: String, // domain-relative media path; empty when none
    pub media_file: Option<String>, // output-relative extracted media, or None
}
```

`WaMessage::is_image()` (`media_file` present and its extension is an image type)
drives inline display in the transcript.

## Extraction (`whatsapp.rs`)

Mirrors `attachments::extract_attachments` (DB-driven, no synthesis):

```rust
pub struct WaSummary { pub dir: String, pub extracted: usize, pub missing: usize }
pub fn extract_media(backup, items: &mut [WaMessage], out) -> std::io::Result<WaSummary>;
```

Per message with a non-empty `source_path`: output name `<n>_<basename>` (1-based
index for uniqueness); `fetch(<whatsapp-domain>, source_path, <out>/whatsapp_media/<name>)`;
set `media_file` on success. `extracted` = messages with a media file; `missing` =
messages whose media was not found (`extracted + missing` counts only media
messages, not all messages — see envelope). `--no-files` skips extraction.

## Output envelope

```json
{
  "ok": true, "command": "whatsapp", "count": 5821,
  "outputs": ["<out>/whatsapp.json"],
  "files": { "dir": "whatsapp_media", "extracted": 410, "missing": 12 },
  "device": { ... }
}
```

`count` = total messages. `files` (present only when extraction ran, named for
consistency with `photos`/`attachments`) counts media
files: `extracted` written, `missing` media-bearing messages whose file was absent.
Store absent → `count: 0`, `outputs: []`, `note`, no `files`.

## Formatters

- **CSV** columns: `chat, sender, from_me, date, text, media_file`.
- **JSON**: serde derive.
- **HTML** (`archive/templates/whatsapp.html`): a transcript table — chat, sender
  (or "me"), date, text, and media (inline `<img>` for image types, else a link).
  askama-escaped (asserted by a test).

## inspect

Add `("whatsapp", true, "AppDomainGroup-group.net.whatsapp.WhatsApp.shared", "ChatStorage.sqlite")`;
`count` = parsed `ZWAMESSAGE` rows (best-effort).

## Error handling

| Situation | Behavior |
|---|---|
| No `ChatStorage.sqlite` | `count: 0`, `outputs: []`, `note`, exit 0 |
| Message media file absent | `media_file: null`, `missing++`, continue |
| `ZMEDIALOCALPATH` empty/unresolvable | empty `source_path`, never fetched |
| Present but unreadable DB | exit 1, `kind:"other"` |
| `-f vcf` / unknown | usage error, exit 1 |

## Testing

- Parser: fixture `make_whatsapp` (a from-me text message, an incoming message with
  a `Media/...` media path, a message with no media) → asserts chat/sender/from_me/
  date(Cocoa)/text/source_path mapping (the `Message/` prefix) and ordering.
- Path mapping unit test: `Media/x/y.jpg` → `Message/Media/x/y.jpg`;
  `Message/Media/a.jpg` → unchanged; empty → empty.
- Extraction: gated end-to-end (`extracted + missing` == media-message count, files
  exist); output-name unit test.
- Formatters: csv header+row, json roundtrip, html inline image vs link + escapes a
  crafted `text`/`sender`.
- CLI: `whatsapp` parses with/without `--no-files`; `export_format` rejects vcf;
  inspect lists whatsapp supported.

## Out of scope (YAGNI)

- Group-member resolution beyond the raw JID; contact-name enrichment from the
  address book; reactions, replies-to, system messages decoding.
- Status/stories, call logs, deleted-message recovery.
- Media transcoding/thumbnailing.

## Global constraints (carried from the project)

- Agent-first contract; password via flag/env; never prompts.
- No new dependency.
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

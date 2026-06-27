# Agent guide: archive

`archive` extracts personal data from an on-disk iOS backup. It is
built to be driven by agents: every command prints exactly one JSON object to
**stdout**, human progress goes to **stderr**, and exit codes are stable.

## Invocation

```
archive --backup <DIR> [--password <PW>] [-o <OUT>] <COMMAND> [ARGS]
```

- `--backup <DIR>` (required): the iOS backup directory. Must appear **before**
  the subcommand.
- `--password <PW>` (optional): encrypted-backup password. May also be supplied
  via the `ARCHIVE_PASSWORD` environment variable. Not needed for
  unencrypted backups. Headless runs never prompt.
- `-o, --out <OUT>`: output directory. Required for export commands; ignored by
  `inspect`.

`--password` and `--out` are global flags — they may appear before **or** after
the subcommand name, which is agent-friendly for programmatic invocation.

## Commands

### `inspect` — discover what is extractable (read-only)

```
archive --backup <DIR> [--password <PW>] inspect
```

stdout:

```json
{
  "ok": true,
  "command": "inspect",
  "device": { "name": "iPhone", "model": "iPhone14,2", "ios": "17.5", "serial": "F2L...", "udid": "00008..." },
  "stores": [
    { "type": "contacts", "present": true, "supported": true, "count": 1234 },
    { "type": "calls", "present": true, "supported": true, "count": 5678 },
    { "type": "voicemail", "present": true, "supported": true, "count": 42 },
    { "type": "voice-memos", "present": true, "supported": true, "count": 8 },
    { "type": "safari-history", "present": true, "supported": true, "count": 940 },
    { "type": "safari-bookmarks", "present": true, "supported": true, "count": 31 },
    { "type": "calendar", "present": true, "supported": true, "count": 212 },
    { "type": "notes", "present": true, "supported": true, "count": 87 },
    { "type": "photos", "present": true, "supported": true, "count": 1240 },
    { "type": "attachments", "present": true, "supported": true, "count": 312 }
  ]
}
```

`supported` = this build can export the type; `present` = the store exists in
this backup; `count` is best-effort: filled for supported + present stores, and
`null` otherwise (including the rare case where a present store cannot be read).

### `contacts` — export contacts

```
archive --backup <DIR> [--password <PW>] -o <OUT> contacts -f <FORMAT>
```

`FORMAT` is one of `csv | json | vcf | html`. Writes `<OUT>/contacts.<ext>`.

stdout:

```json
{
  "ok": true,
  "command": "contacts",
  "count": 1234,
  "outputs": ["<OUT>/contacts.vcf"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

If the backup has no contacts: `"ok": true, "count": 0, "outputs": []` plus a
`"note"` field.

Each contact also carries an `addresses` array (postal addresses): objects with
`label`, `street`, `city`, `state`, `zip`, `country`, `country_code` (empty
strings for absent parts). In vCard output these become `ADR` properties; in CSV
they are joined into one `addresses` column.

### `calls` — export call history

```
archive --backup <DIR> [--password <PW>] -o <OUT> calls -f <FORMAT>
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/calls.<ext>`. stdout envelope:

```json
{
  "ok": true,
  "command": "calls",
  "count": 5678,
  "outputs": ["<OUT>/calls.json"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

Each call object: `number` (phone number, or an Apple ID/email for FaceTime),
`date` (ISO 8601 UTC), `duration_seconds`, `direction` (`incoming`/`outgoing`),
`answered` (bool), `service` (`phone`/`facetime`/raw bundle id/`unknown`),
`video` (best-effort FaceTime video flag — **version-dependent and undocumented**,
may be `null`), `call_type` (raw `ZCALLTYPE` integer, the honest backing for
`video`; `null` when absent), `location` (or `null` when absent), `country` (or
`null` when absent). No call history → `count: 0`, `outputs: []`, plus a `note`.

### `voicemail` — export voicemail metadata and audio

```
archive --backup <DIR> [--password <PW>] -o <OUT> voicemail -f <FORMAT> [--audio] [--audio-format <amr|m4a|wav>]
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/voicemail.<ext>`.

**Audio extraction (optional):**

With `--audio`, each voicemail's audio is fetched from `HomeDomain/Library/Voicemail/<rowid>.amr` into `<out>/voicemail_audio/` and linked from each record's `audio_file` field. Each audio file is named `<date>_<sender>_<rowid>.<ext>` (date is `YYYY-MM-DD_HHMMSS` UTC or `unknown`; sender is the sanitized caller number or `unknown` when withheld). `--audio-format` defaults to `amr` (raw copy, no dependencies); `m4a`/`wav` transcode via `ffmpeg` (required only then). `--audio-format` without `--audio` is a usage error.

Each record gains `rowid` (stable per-backup id, JSON output only) and `audio_file` (output-relative path in all formats, or `null` when no audio exists for that row).

stdout envelope:

```json
{
  "ok": true,
  "command": "voicemail",
  "count": 42,
  "outputs": ["<OUT>/voicemail.json"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." },
  "audio": { "format": "m4a", "dir": "voicemail_audio", "extracted": 10, "missing": 2 }
}
```

The `audio` envelope is present only when `--audio` ran. `extracted` counts files written (including raw `.amr` kept when a transcode fails); `missing` counts rows with no audio in the backup.

Each voicemail object: `sender` (caller number; empty when withheld), `date`
(ISO 8601 UTC), `duration_seconds`, `trashed` (bool — moved to Deleted),
`trashed_at` (ISO 8601 UTC or `null`), `expiration` (ISO 8601 UTC or `null`),
`flags` (raw bitmask, **not decoded** — use `trashed` for deletion status), `rowid` (stable per-backup identifier, JSON output only), `audio_file` (output-relative path to extracted audio file, e.g. `voicemail_audio/<name>`, or `null` when no audio exists for that row). No
voicemail → `count: 0`, `outputs: []`, plus a `note`.

### `voice-memos` — export Voice Memos metadata and audio

```
archive --backup <DIR> [--password <PW>] -o <OUT> voice-memos -f <FORMAT> [--no-audio] [--audio-format <m4a|wav>]
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/voice-memos.<ext>`.

**Audio extraction is on by default** — unlike `voicemail` (opt-in `--audio`),
because a voice memo *is* its audio and the native `.m4a` copy needs no external
dependency. Each recording is fetched from
`AppDomainGroup-group.com.apple.VoiceMemos/Recordings/` (legacy fallback:
`MediaDomain/Recordings/`) into `<OUT>/voice_memos/` and linked from each record's
`audio_file`. Pass `--no-audio` for metadata only. `--audio-format` defaults to a
**raw native copy** (no dependency); `m4a`/`wav` transcode via `ffmpeg` (required
only then). `--audio-format` with `--no-audio`, or `--audio-format amr`, is a
usage error.

Each audio file is named `<date>_<title>_<n>.<ext>` (date is `YYYY-MM-DD_HHMMSS`
UTC or `unknown`; title is the sanitized memo name or `unknown`; `n` is a 1-based
index ensuring uniqueness). Recordings present on disk but absent from the
metadata DB are still recovered and listed with an empty title.

stdout envelope:

```json
{
  "ok": true,
  "command": "voice-memos",
  "count": 8,
  "outputs": ["<OUT>/voice-memos.json"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." },
  "audio": { "format": "raw", "dir": "voice_memos", "extracted": 8, "missing": 0 }
}
```

The `audio` envelope is present only when extraction ran (i.e. not `--no-audio`);
`format` is `raw` for native copies, else `m4a`/`wav`. Each voice-memo object:
`title` (memo name; empty when unnamed), `date` (ISO 8601 UTC), `duration_seconds`,
`source_file` (the recording's filename in the backup), `audio_file`
(output-relative path, e.g. `voice_memos/<name>`, or `null`). No Voice Memos store
→ `count: 0`, `outputs: []`, plus a `note`.

### `safari-history` — export Safari browsing history

```
archive --backup <DIR> [--password <PW>] -o <OUT> safari-history -f <FORMAT>
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/safari-history.<ext>`. One
record per visit: `url`, `title` (page title at visit time; empty when unknown),
`date` (ISO 8601 UTC), `visit_count` (total visits for the URL). No history →
`count: 0`, `outputs: []`, plus a `note`.

### `safari-bookmarks` — export Safari bookmarks

```
archive --backup <DIR> [--password <PW>] -o <OUT> safari-bookmarks -f <FORMAT>
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/safari-bookmarks.<ext>`. One
record per leaf bookmark: `title`, `url`, `folder` (containing folder's name;
empty at the root). No bookmarks → `count: 0`, `outputs: []`, plus a `note`.

### `calendar` — export calendar events

```
archive --backup <DIR> [--password <PW>] -o <OUT> calendar -f <FORMAT>
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/calendar.<ext>`. One record
per event: `summary`, `start` / `end` (ISO 8601 UTC; `end` empty when unset),
`all_day` (bool), `calendar` (owning calendar's title). Dates are the raw
Cocoa→UTC conversion; all-day/floating events are stored timezone-less by iOS and
are not normalized. Reminders are **not** exported (their backup storage varies).
No calendar → `count: 0`, `outputs: []`, plus a `note`.

### `notes` — export Apple Notes

```
archive --backup <DIR> [--password <PW>] -o <OUT> notes -f <FORMAT>
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/notes.<ext>`. One record
per note: `title`, `folder` (containing folder's name; empty when none), `created`
/ `modified` (ISO 8601 UTC), `body` (the note text), `body_source`. The body is
stored on-device as a gzip-compressed protobuf; it is decoded best-effort and
`body_source` says which path produced `body`: `decoded` (full text recovered),
`snippet` (only the preview was recoverable — e.g. an encrypted/undecodable note),
or `empty`. Rich formatting, inline images, and attachments are not recovered
(plain text only). No notes store → `count: 0`, `outputs: []`, plus a `note`.

### `photos` — export Camera Roll metadata and files

```
archive --backup <DIR> [--password <PW>] -o <OUT> photos -f <FORMAT> [--no-files]
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/photos.<ext>`. **Files are
extracted by default** into `<OUT>/photos/` (the camera roll can be many
gigabytes); pass `--no-files` for a metadata-only catalog. Files are copied
byte-for-byte (no transcoding/thumbnailing). The HTML output is a gallery linking
the extracted files (HEIC/HEVC may not render inline in every browser; the files
are present regardless).

Each asset: `filename`, `kind` (`image`/`video`/`unknown`), `created` (ISO 8601
UTC), `favorite`, `trashed` (in Recently Deleted), `width`/`height`, `latitude`/
`longitude` (`null` when no GPS fix), `duration_seconds` (videos; `null`
otherwise), `source_path` (backup-relative source), `file` (output-relative
extracted path, or `null` when the asset is iCloud-only / absent from the backup).

stdout envelope (the `files` object is present only when extraction ran):

```json
{
  "ok": true, "command": "photos", "count": 1240,
  "outputs": ["<OUT>/photos.json"],
  "files": { "dir": "photos", "extracted": 1238, "missing": 2 },
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

No photos store → `count: 0`, `outputs: []`, plus a `note`.

### `attachments` — export Messages attachment files

```
archive --backup <DIR> [--password <PW>] -o <OUT> attachments -f <FORMAT> [--no-files]
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/attachments.<ext>`. **Files
are extracted by default** into `<OUT>/attachments/`; pass `--no-files` for a
metadata-only catalog. The HTML output is a gallery (images inline, other types
as links). This recovers the media sent/received in Messages (iMessage/SMS); it
does not export the message *text* or link attachments to conversations.

Each attachment: `name` (original transfer name, else the on-device basename),
`mime_type`, `created` (ISO 8601 UTC), `total_bytes`, `source_path`
(MediaDomain-relative source), `file` (output-relative extracted path, or `null`
when absent from the backup). Envelope carries a `files` object (`dir`,
`extracted`, `missing`) when extraction ran. No Messages store → `count: 0`,
`outputs: []`, plus a `note`.

### `recover` — one-shot customer package

```
archive --backup <DIR> [--password <PW>] -o <OUT> recover [--no-files]
```

Runs **every** supported extractor in one shot into `<OUT>/`, writing one HTML
file per data type plus the media folders, plus a customer-facing
`<OUT>/index.html` landing page (device sheet — name, model, serial, iOS, UDID —
and a table linking each export with counts). The backup is opened once.
`--no-files` skips the large media extraction (metadata + HTML only). A store
absent from the backup is skipped; one unreadable store is logged and skipped, not
fatal.

stdout envelope:

```json
{
  "ok": true,
  "command": "recover",
  "outputs": ["<OUT>/index.html", "<OUT>/contacts.html", "..."],
  "sections": [
    { "data_type": "contacts", "label": "Kontakty", "file": "contacts.html", "count": 1234, "media": null },
    { "data_type": "photos", "label": "Fotky a videa", "file": "photos.html", "count": 1240,
      "media": { "dir": "photos", "extracted": 1238, "missing": 2 } }
  ],
  "device": { "name": "iPhone", "model": "iPhone14,2", "ios": "17.5", "serial": "F2L...", "udid": "00008..." }
}
```

`outputs[0]` is always `index.html`. `--zip` packaging is not yet implemented (zip
the `<OUT>` folder yourself).

## Result envelope (every command)

The `device` object on every command carries `name`, `model` (hardware id, e.g.
`iPhone14,2`), `ios`, `serial`, and `udid`.


- Success → the per-command object above (always `"ok": true`).
- Failure →

```json
{ "ok": false, "error": "<human message>", "kind": "auth|usage|other" }
```

`kind` is machine-stable; the `error` text may change.

## Exit codes

A command that runs far enough to produce a result prints the JSON envelope
(above) to **stdout** and exits with:

| code | meaning |
|------|---------|
| 0 | success (including "store absent — nothing to export") |
| 2 | `{ "ok": false, "kind": "auth" }` — encrypted backup locked, or wrong/missing password |
| 1 | `{ "ok": false, "kind": "usage" }` or `"other"` — unknown `-f` format, missing `--out`, parse/IO error |

**Argument-parsing errors are a separate channel.** If the invocation itself is
malformed (a missing required flag such as `--backup`, or an unknown flag),
`clap` prints plain-text usage to **stderr**, writes **nothing** to stdout, and
also exits **2** — there is no JSON envelope. Disambiguate exit 2 by inspecting
stdout: a JSON object with `"kind": "auth"` is an authentication failure; empty
stdout (with usage text on stderr) is a malformed invocation. When unsure, run
`--help` or `inspect` first to learn the contract.

> Note: argument/usage errors (an unknown `-f` format, a missing `--out`) are detected **before** the backup is opened, so they surface as exit 1 even when the backup is an encrypted one that would otherwise report an auth error.

## Examples

```bash
# Discover
archive --backup ~/Backup/UDID inspect

# Export contacts as importable vCard
archive --backup ~/Backup/UDID -o /tmp/out contacts -f vcf

# Encrypted backup via env var (no prompt)
ARCHIVE_PASSWORD=secret \
  archive --backup ~/Backup/UDID -o /tmp/out contacts -f json
```

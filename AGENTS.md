# Agent guide: archive

`archive` extracts personal data from an on-disk iOS backup. It is
built to be driven by agents: every command prints exactly one JSON object to
**stdout**, human progress goes to **stderr**, and exit codes are stable.

## Invocation

```
archive --backup <DIR> [--password <PW>] [-o <OUT>] <COMMAND> [ARGS]
```

- `--backup <DIR>`: the iOS backup directory. Must appear **before** the
  subcommand. Required for every command **except `backup`** (which creates a new
  backup and has no input directory); a read command without it fails with a
  usage error (exit 1).
- `--password <PW>` (optional): encrypted-backup password. May also be supplied
  via the `ARCHIVE_PASSWORD` environment variable. Not needed for
  unencrypted backups. Headless runs never prompt.
- `-o, --out <OUT>`: output directory. Required for export commands; ignored by
  `inspect`.
- `--chrome-path <PATH>` (optional): headless browser for `-f pdf` (see below).

`--password`, `--out`, and `--chrome-path` are global flags — they may appear
before **or** after the subcommand name, which is agent-friendly for programmatic
invocation.

**PDF output:** the **in-process** export commands that list `html` (contacts,
calls, accounts, known-networks, homescreen-layout, data-usage, device-usage, bluetooth-devices, voicemail, voice-memos, safari-history/bookmarks,
calendar, reminders, mail, notes, photos, photos-recently-deleted, attachments,
whatsapp, timeline, stats, app-databases, app-files, recover-deleted, health, apps, keychain-inventory) also accept **`pdf`**: their HTML is printed to `<OUT>/<name>.pdf` by a
headless Chrome/Chromium/Edge (auto-detected on `PATH`/standard locations or set
with `--chrome-path`; a missing browser is a usage error, exit 1), with the JSON
envelope unchanged (`outputs` points at the `.pdf`). `messages -f pdf` is produced
**separately** by the bundled imessage-exporter using its own PDF engine (Quartz
on macOS, a headless browser elsewhere; `--chrome-path` is forwarded), and keeps
the messages envelope (`output` directory). The `recover` one-shot package has no
`-f pdf` and stays HTML.

**Contact enrichment:** when the backup has an address book, `calls`,
`voicemail`, `whatsapp`, `timeline` and `recover` automatically resolve phone
numbers / emails / WhatsApp JIDs to a `contact_name`. Matching is best-effort:
phone numbers compare on their **last 9 digits** (so country-code and formatting
differences still match), emails match case-insensitively, and a JID matches on
its numeric local part. The field is **omitted from JSON** (and blank in
CSV/HTML) when nothing matched; in `timeline` the resolved name replaces the raw
number in the event summary. No contacts in the backup ⇒ no enrichment, never an
error.

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
    { "type": "attachments", "present": true, "supported": true, "count": 312 },
    { "type": "whatsapp", "present": true, "supported": true, "count": 5821 },
    { "type": "health", "present": true, "supported": true, "count": 9 },
    { "type": "reminders", "present": true, "supported": true, "count": 64 },
    { "type": "mail", "present": false, "supported": true, "count": null }
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
`contact_name` (address-book name resolved from `number`; **omitted from JSON
when no contact matched** — see *Contact enrichment* below),
`date` (ISO 8601 UTC), `duration_seconds`, `direction` (`incoming`/`outgoing`),
`answered` (bool), `service` (`phone`/`facetime`/raw bundle id/`unknown`),
`video` (best-effort FaceTime video flag — **version-dependent and undocumented**,
may be `null`), `call_type` (raw `ZCALLTYPE` integer, the honest backing for
`video`; `null` when absent), `location` (or `null` when absent), `country` (or
`null` when absent). No call history → `count: 0`, `outputs: []`, plus a `note`.

### `homescreen-layout` — reconstruct the Home Screen layout

```
archive --backup <DIR> [--password <PW>] -o <OUT> homescreen-layout -f <FORMAT>
```

`FORMAT` is one of `csv | json | html | pdf`. Reads SpringBoard's
`HomeDomain/Library/SpringBoard/IconState.plist` (a normal backup file, so it
works on **any** backup — no password needed) and writes
`<OUT>/homescreen-layout.<ext>`. No file extraction.

Output is a flat, ordered list of placed icons. Each `IconSlot`: `container`
(`dock`, `page N` 1-based, or `folder:<name>`), `position` (0-based within the
container; continuous across a folder's pages), `kind` (`app`, `webclip`,
`folder`, `widget`, or `widget-stack`), `identifier` (bundle/display id; empty
for an unnamed folder or a widget stack), `label` (folder title, app/web-clip
caption, or — for a widget stack — the comma-joined bundle ids of its member
widgets). Apps are commonly stored as bare bundle-id strings; folders expand one
level deep (iOS does not nest folders); a widget stack's opaque UUID is hidden in
favour of its member apps. Read leniently — unknown entries are skipped.

stdout envelope:

```json
{
  "ok": true, "command": "homescreen-layout", "count": 45,
  "outputs": ["<OUT>/homescreen-layout.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

No `IconState.plist` in the backup → `count: 0` + a `note`; a present-but-unparseable
plist → `count: 0` + a different `note`.

### `data-usage` — per-process network data usage

```
archive --backup <DIR> [--password <PW>] -o <OUT> data-usage -f <FORMAT>
```

`FORMAT` is one of `csv | json | html | pdf`. Writes `<OUT>/data-usage.<ext>`.
Reads `WirelessDomain/Library/Databases/DataUsage.sqlite` and aggregates the
`ZLIVEUSAGE` time-windowed counters per `ZPROCESS`, sorted by total bytes
descending. Each row: `process` (`ZPROCNAME`), `bundle` (`ZBUNDLENAME`; empty
when unrecorded), `wwan_in` / `wwan_out` (cellular bytes) and `wifi_in` /
`wifi_out` (Wi-Fi bytes) summed across windows, plus `first_seen` / `last_seen`
(ISO 8601 UTC). CSV/JSON also carry `wwan_total` / `wifi_total`. Note: on many
devices only the cellular counters are populated (Wi-Fi columns read 0) — that is
the database's own behaviour, reported verbatim. Schema-tolerant: a missing
counter column degrades to 0, and an absent/foreign-key-less database yields
`count: 0` + a `note`.

stdout envelope:

```json
{
  "ok": true, "command": "data-usage", "count": 413,
  "outputs": ["<OUT>/data-usage.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

### `device-usage` — per-app foreground usage (knowledgeC.db)

```
archive --backup <DIR> [--password <PW>] -o <OUT> device-usage -f <FORMAT>
```

`FORMAT` is one of `csv | json | html | pdf`. Writes `<OUT>/device-usage.<ext>`.
Reads CoreDuet's `knowledgeC.db` (probing a few candidate domain/paths) and
aggregates the `ZOBJECT` `/app/usage` stream per bundle: `bundle`,
`total_seconds` (sum of session durations), `sessions` (count), and
`first_used` / `last_used` (ISO 8601 UTC), sorted by total time descending.
Schema-tolerant: a `ZOBJECT` missing the needed columns yields an empty result.

**Availability:** `knowledgeC.db` is increasingly protected and is frequently
**excluded from iOS 16+ backups**; when absent the command returns `count: 0`
with a `note` (no store), and an empty-but-present store returns `count: 0` with
a different `note`. A **store**, so it is listed by `inspect` (presence probed
across the candidate paths) and included in `recover` when non-empty.

stdout envelope:

```json
{
  "ok": true, "command": "device-usage", "count": 87,
  "outputs": ["<OUT>/device-usage.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

### `bluetooth-devices` — paired and previously-seen Bluetooth devices

```
archive --backup <DIR> [--password <PW>] -o <OUT> bluetooth-devices -f <FORMAT>
```

`FORMAT` is one of `csv | json | html | pdf`. Writes `<OUT>/bluetooth-devices.<ext>`.
Merges three sources in the shared Bluetooth domain
(`SysSharedContainerDomain-systemgroup.com.apple.bluetooth`): the LE
`com.apple.MobileBluetooth.ledevices.paired.db` (`PairedDevices`) and
`…ledevices.other.db` (`OtherDevices`) databases, plus the classic
`Library/Preferences/com.apple.MobileBluetooth.devices.plist`. Each device has
`name`, `address`, `resolved_address` (the LE identity address behind a rotating
random one, when known and different), and `kind` (`paired` | `classic` |
`other`). De-duplicated by address (strongest kind wins) and ordered paired →
classic → named-other → anonymous-other. The databases' `LastSeenTime` /
`LastConnectionTime` columns hold device-relative counters (not a Unix/Cocoa
wall-clock epoch) on the backups inspected, so they are **not** surfaced as
dates. Schema-tolerant: an unexpected schema or malformed plist is skipped, never
fatal. A **store** (listed by `inspect`, included in `recover` when non-empty);
when none of the sources exist, returns `count: 0` with a `note`.

stdout envelope:

```json
{
  "ok": true, "command": "bluetooth-devices", "count": 1005, "named": 5,
  "outputs": ["<OUT>/bluetooth-devices.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

### `voicemail` — export voicemail metadata and audio

```
archive --backup <DIR> [--password <PW>] -o <OUT> voicemail -f <FORMAT> [--audio] [--audio-format <amr|m4a|wav>]
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/voicemail.<ext>`.

**Audio extraction (optional):**

With `--audio`, each voicemail's audio is fetched from `HomeDomain/Library/Voicemail/<rowid>.amr` into `<out>/voicemail_audio/` and linked from each record's `audio_file` field. Each audio file is named `<date>_<sender>_<rowid>.<ext>` (date is `YYYY-MM-DD_HHMMSS` UTC or `unknown`; sender is the sanitized caller number or `unknown` when withheld). `--audio-format` defaults to `amr` (raw copy, no dependencies); `m4a`/`wav` transcode via `ffmpeg` (required only then). `--audio-format` without `--audio` is a usage error.

Each record gains `rowid` (stable per-backup id, JSON output only), `audio_file` (output-relative path in all formats, or `null` when no audio exists for that row), and `contact_name` (resolved from `sender`; omitted from JSON when no contact matched — see *Contact enrichment*).

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

Each asset: `filename`, `kind` (`image`/`video`/`unknown`), `created` /
`modified` / `added` (ISO 8601 UTC; empty when unset), `favorite`, `hidden` (in
the Hidden album — still recovered, just flagged), `trashed` (in Recently
Deleted), `trashed_date` (ISO 8601 UTC when binned; empty otherwise),
`edited` (has adjustments), `live_photo` (best-effort) with raw
`kind_subtype`, `width`/`height`, `latitude`/`longitude` (`null` when no GPS fix),
`duration_seconds` (videos; `null` otherwise), `burst_id` (`ZAVALANCHEUUID`; same
id ⇒ same burst; `null` when not a burst), `original_filename`, `title` (caption),
`albums` (array of album names this asset is in), `source_path` (backup-relative
source), `file` (output-relative extracted path, or `null` when the asset is
iCloud-only / absent from the backup). Album membership is resolved by discovering
the version-dependent `Z_<n>ASSETS` join table at runtime; version-dependent
values (`live_photo`, album discovery) are best-effort and `kind_subtype` is kept
raw for fidelity.

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

### `photos-recently-deleted` — recover trashed Camera Roll assets

```
archive --backup <DIR> [--password <PW>] -o <OUT> photos-recently-deleted -f <FORMAT> [--no-files]
```

`FORMAT` is one of `csv | json | html | pdf`. Writes
`<OUT>/photos-recently-deleted.<ext>` and, by default, recovers the files into
`<OUT>/recently-deleted/` (`--no-files` for a metadata-only catalog). Reads the
same `Photos.sqlite` as `photos` but keeps only assets still in the Recently
Deleted album (`ZTRASHEDSTATE`). iOS purges those after ~30 days, so a backup
taken inside that window still holds the original files.

Each item carries every `photos` field (flattened) plus `purge_after`: the
estimated permanent-deletion time (`trashed_date` + 30 days, ISO 8601 UTC; empty
when the trashed date is unknown). The JSON is an array of those flattened
objects.

stdout envelope (the `files` object is present only when extraction ran):

```json
{
  "ok": true, "command": "photos-recently-deleted", "count": 7,
  "outputs": ["<OUT>/photos-recently-deleted.json"],
  "files": { "dir": "recently-deleted", "extracted": 7, "missing": 0 },
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

No photos store → `count: 0` + `note: this backup has no photos`; a photos store
with nothing trashed → `count: 0` + `note: this backup has no recently-deleted
photos`.

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

### `whatsapp` — export WhatsApp messages and media

```
archive --backup <DIR> [--password <PW>] -o <OUT> whatsapp -f <FORMAT> [--no-files]
```

`FORMAT` is one of `csv | json | html`. Writes `<OUT>/whatsapp.<ext>`. **Media is
extracted by default** into `<OUT>/whatsapp_media/`; pass `--no-files` for a
text-only transcript. Reads `ChatStorage.sqlite` from the WhatsApp shared app-group
container. Each message: `chat` (contact/group name), `sender` (JID; empty when
from me), `from_me` (bool), `date` (ISO 8601 UTC), `text`, `media_file`
(output-relative extracted media, or `null`), `contact_name` (resolved from the
`sender` JID; omitted from JSON when no contact matched — see *Contact
enrichment*). Envelope carries a `files` object
(`dir`, `extracted`, `missing`) when extraction ran — consistent with `photos`/
`attachments`; `count` is total messages. No WhatsApp store → `count: 0`,
`outputs: []`, plus a `note`.

### `messages` — export iMessage/SMS/RCS conversation transcript

```
archive --backup <DIR> [--password <PW>] -o <OUT> messages -f <FORMAT>
```

`FORMAT` is one of `txt | html | pdf` (an unsupported value is a usage error,
exit 1, before anything else runs). Unlike the other commands, `archive` does
**not** decode messages in-process: it drives the bundled `imessage-exporter`
binary as a subprocess and writes the full transcript tree (per-conversation
files plus an `attachments/` directory) under `<OUT>/messages/`.

The exporter binary is resolved in this order: the `ARCHIVE_IMESSAGE_EXPORTER`
env var (if set and non-empty) → a sibling of the running `archive` executable →
the bare name `imessage-exporter` on `PATH`. A resolution/spawn failure is exit 1
with a hint to build the workspace or set the env var.

The backup is opened with `archive-core` first, so the auth contract holds: an
encrypted backup without the right password fails with exit 2 **before** the
subprocess starts. The password is forwarded to the exporter (`-x`) only for
encrypted backups. The exporter's own progress is redirected to stderr so stdout
stays a single JSON object.

stdout envelope:

```json
{
  "ok": true,
  "command": "messages",
  "format": "html",
  "output": "<OUT>/messages",
  "device": { "name": "iPhone", "model": "iPhone14,2", "ios": "17.5", "serial": "F2L...", "udid": "00008..." }
}
```

The envelope points at the output directory rather than embedding message bodies
(consistent with `backup`/`recover`). `messages` is **not** part of the `recover`
one-shot package and is **not** listed by `inspect` (presence of message data is
already visible via the `attachments` store, which reads the same `sms.db`).

### `health` — export Apple Health

```
archive --backup <DIR> [--password <PW>] -o <OUT> health -f <FORMAT>
```

`FORMAT` is `csv | json | html`. Reads `HealthDomain/Health/healthdb_secure.sqlite`.
Two sections: **workouts** (`activity_type` friendly name + raw `activity_type_id`,
`start`/`end` ISO 8601 UTC, `duration_seconds`, `total_distance` in meters,
`total_energy_burned` in kcal) and a **quantity summary** per known type
(`data_type_id`, `name`, `count`, `sum`, `min`/`avg`/`max`, `first`/`last` ISO
date range) for step count, distance, heart rate, active/basal energy, flights.
Handles both the legacy denormalized `workouts` schema and the modern split
(`samples` + `workout_activities`); unknown quantity types are skipped. `csv`
writes **two** files (`health-workouts.csv`, `health-quantity.csv`); `json`/`html`
write one (`health.json` / `health.html`). Envelope carries `count` (workouts +
quantity types), `workouts`, `quantity_types`, and `outputs`. No Health DB →
`count: 0` + note; present-but-empty → `count: 0` + note.

### `reminders` — export Apple Reminders

```
archive --backup <DIR> [--password <PW>] -o <OUT> reminders -f <FORMAT>
```

`FORMAT` is `csv | json | html`. Writes `<OUT>/reminders.<ext>`. Reads the
Reminders Core Data store under `AppDomainGroup-group.com.apple.reminders` (a
`Data-<UUID>.sqlite` located by shape, not a fixed name). Per reminder: `list`,
`title`, `notes`, `due` (ISO 8601 UTC or null), `completed` (bool),
`completed_date` (or null), `priority` (raw int; Apple uses 1/5/9), `created`
(or null), `flagged` (bool). Version-dependent `Z`-tables/columns are discovered
at runtime (the `Z_ENT` discriminator is resolved from `Z_PRIMARYKEY` by name).
No store → `count: 0` + note.

### `mail` — export Apple Mail (`.emlx`)

```
archive --backup <DIR> [--password <PW>] -o <OUT> mail -f <FORMAT>
```

`FORMAT` is `csv | json | html`. Writes `<OUT>/mail.<ext>`. Enumerates `.emlx`
files under `MailDomain` and parses each (RFC 822 headers, RFC 2047
encoded-words; snippet from the first `text/plain` part, capped). Per message:
`date` (ISO 8601 UTC or null), `from`, `to`, `subject`, `snippet`. **iOS backs
up mail only for local / POP3 / "On My iPhone" mailboxes**, so for most backups
this reports `count: 0` + a note. The snippet is a preview (no body
transfer-encoding decoding).

### `apps` — list installed apps

```
archive --backup <DIR> [--password <PW>] -o <OUT> apps -f <FORMAT>
```

`FORMAT` is `csv | json | html`. Writes `<OUT>/apps.<ext>`. Lists distinct
third-party app bundle ids derived from the manifest's `AppDomain-<bundle id>`
domains (each installed user app has one), sorted; `json` is a flat array of
bundle-id strings. Manifest-derived, not a data store: **not** listed by
`inspect` and **not** part of `recover`.

### `timeline` — unified chronological timeline

```
archive --backup <DIR> [--password <PW>] -o <OUT> timeline -f <FORMAT> [--redact]
```

`--redact` masks the strongest direct identifiers in each summary for shareable
output — a digit run of ≥5 keeps only its last two (`+420776452878` →
`+••••••••••78`), an email local part keeps only its first character
(`jan@firma.cz` → `j••@firma.cz`); names are kept. The JSON envelope reports
`redacted: <bool>`. `FORMAT` is `csv | json | html`. Writes `<OUT>/timeline.<ext>`. Merges every
in-process extractor (calls, voicemail, voice memos, Safari history, calendar,
notes, photos, attachments, WhatsApp, reminders, Health workouts, mail) into one
stream of events, each `{ timestamp (ISO 8601 UTC), kind, summary }`, sorted
chronologically (undated records are dropped). Each source is best-effort: an
absent or unreadable store is skipped, not fatal. A **view** over the other
extractors, not a data store — like `apps`, it is **not** listed by `inspect`
and **not** part of `recover`. `messages` (conversation text, exported out of
process) and `apps` (not an event stream) are excluded. Envelope: `ok`,
`command`, `count` (total events), `outputs`, `device`.

### `stats` — activity dashboard

```
archive --backup <DIR> [--password <PW>] -o <OUT> stats -f <FORMAT>
```

`FORMAT` is `csv | json | html | pdf`. Writes `<OUT>/stats.<ext>`. Aggregates the
same event stream as `timeline` (it shares the collector) into per-category
statistics: for each `kind`, the event `count` and the `earliest` / `latest`
dated event (ISO 8601 UTC), sorted by descending count. Also reports the grand
`total_events` and the overall `earliest` / `latest` span. Undated events are
counted but excluded from ranges; a category whose records carry no usable date
shows empty ranges. Because it reports the data verbatim, outlier dates (e.g. a
sentinel calendar date, or a future recurring event) appear as-is and are
localized to their category rather than silently dropped — useful for spotting
anomalies. A **view** over the extractors, like `timeline`: **not** listed by
`inspect` and **not** part of `recover`.

stdout envelope (the export file holds the full per-category table; the envelope
carries the headline figures):

```json
{
  "ok": true, "command": "stats",
  "total_events": 40198, "categories": 7,
  "earliest": "2009-06-13T...", "latest": "2029-12-26T...",
  "outputs": ["<OUT>/stats.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

### `app-databases` — per-app database recoverability report

```
archive --backup <DIR> [--password <PW>] -o <OUT> app-databases -f <FORMAT>
```

`FORMAT` is one of `csv | json | html | pdf`. Writes `<OUT>/app-databases.<ext>`.
Walks every third-party app domain (`AppDomain-<bundle>`, from the same source as
`apps`), and for each database-like file (`.sqlite` / `.sqlite3` / `.sqlitedb` /
`.db` / `.data`) fetches it and classifies it. Each row: `app` (bundle id),
`path` (within the app domain), `bytes`, `readable` (begins with the SQLite magic
— a plain, openable database, **not** an encrypted SQLCipher / Core-Data binary
store), and `tables` (table count when readable, else `null`). Sorted by app then
size; the envelope adds a `readable` count.

**Why it exists:** many modern messaging apps (Viber, Messenger, …) keep no
readable message store in a normal backup — the database is excluded or
encrypted. This report makes that explicit per app, so you can see what is
actually extractable rather than guessing. A **view** over the backup manifest:
**not** a `KNOWN_STORE` (so not in `inspect`) and **not** part of `recover`.

stdout envelope:

```json
{
  "ok": true, "command": "app-databases", "count": 29, "readable": 28,
  "outputs": ["<OUT>/app-databases.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

No app database files at all → `count: 0`, `outputs: []`, plus a `note`.

### `app-files` — extract a named app's document/media files

```
archive --backup <DIR> [--password <PW>] -o <OUT> app-files --app <NAME> -f <FORMAT> [--all]
```

`FORMAT` is one of `csv | json | html | pdf`. `--app <NAME>` selects the app by a
case-insensitive **substring** of its backup domain — both `AppDomain-<bundle>`
and `AppDomainGroup-<group>` containers match, Apple's own domains never do (e.g.
`viber`, `whatsapp`, `com.burbn.instagram`). Every matching domain is walked and
each file is fetched (decrypted if the backup is encrypted) to
`<OUT>/app-files/<domain>/<relpath>`, preserving the in-domain layout (the domain
becomes one path segment — `/` and `\` replaced with `_`; absolute paths and `..`
traversal are rejected). By default only **media** is copied (image / video /
audio, classified by extension); `--all` copies every file in the container.

The manifest is written to `<OUT>/app-files.<ext>`; `html`/`pdf` render a gallery
(images inline, video/audio/other as links). Each row: `domain`, `path` (within
the domain), `bytes`, `category` (`image` / `video` / `audio` / `other`), and
`file` (the output-relative path of the copied file). Sorted by domain then path.

**Why it exists:** when an app's message database is excluded or encrypted (see
`app-databases`), the **files** it stored — photos, videos, voice messages,
documents — are usually still present in the backup and recoverable. This pulls
them out even though the conversation itself cannot be reconstructed. A **view**
over the backup manifest: **not** a `KNOWN_STORE` (so not in `inspect`) and
**not** part of `recover`.

stdout envelope:

```json
{
  "ok": true, "command": "app-files", "count": 412, "bytes": 73400320,
  "domains": 2, "outputs": ["<OUT>/app-files.json"],
  "device": { "name": "iPhone", "ios": "16.0.3", "udid": "c61ff..." }
}
```

`outputs` lists only the manifest; the extracted files live under
`<OUT>/app-files/`. No matching domain → `count: 0`, `outputs: []`, `note: "no
third-party app domain matched '<app>'"`. A matching domain with no qualifying
files → `count: 0` plus a `note` naming how many domains matched.

### `recover-deleted` — carve deleted SQLite rows

```
archive --backup <DIR> [--password <PW>] -o <OUT> recover-deleted -f <FORMAT> [--store messages|calls|contacts|notes|calendar|safari|photos|all]
```

`FORMAT` is `csv | json | html`; `--store` defaults to `all` (an unknown value is
a usage error, exit 1). Writes `<OUT>/deleted.<ext>`. Recovers **deleted** rows
from the backup's SQLite databases by carving freed regions — freelist pages,
in-page freeblocks, the unallocated gap, and the `-wal` sidecar — with a generic
schema-less parser (`archive-core::carve`), then attributes each carved record to
a store via heuristic signatures: `messages` (`sms.db`, anchored by a 36-char
GUID), `calls` (`CallHistory.storedata`, anchored by a Cocoa-seconds REAL date),
`contacts` (`AddressBook.sqlitedb` / `ABMultiValue`, softest: each value is
classified into a name/org field, an email, or a phone and reassembled as
`Name · Org — phone, email`; a lone deleted phone/email value with no name is
recovered too — labels live in a separate `ABMultiValueLabel` table, so a carved
label literal is dropped as noise, not annotated onto a handle), `notes`
(`NoteStore.sqlite`, title/snippet texts + Cocoa date — the body is gzipped
protobuf and not recovered here), `calendar` (`Calendar.sqlitedb`, event title +
location + earliest associated Cocoa date — schema-less carving cannot single out
the start column from end/created, so the earliest is reported), `safari`
(`History.db`, a URL or a titled visit with a Cocoa visit date), and `photos`
(`Photos.sqlite` `ZASSET`, a media `ZFILENAME` + the earliest/capture Cocoa date —
recovers a deleted Camera Roll asset's identity even after it left "Recently
Deleted", though the pixels are usually gone). Each output row is `{ store, source (freelist|freeblock|
unallocated|wal), rowid, date (ISO 8601 UTC or null), summary, truncated }`,
sorted chronologically. `truncated` is true when the carved cell ran past the
available bytes (record-size cap or an overwritten tail), so its trailing columns
are partial — the CSV shows `yes`, the HTML marks the row `✂ oříznuto`.

Rows still **live** in the database are excluded (a carved candidate whose cell
rowid — or, for messages, GUID — is still present in the live table is dropped).
Because `-wal` frames are full page images mixing live and deleted cells, every
store except `messages` lacks a strong content anchor and therefore **ignores WAL
candidates entirely** (their deletions still surface from the main file's free
regions); `messages` use the GUID anchor and do recover from the WAL.

To suppress false positives the soft signatures demand **corroboration**, not a
single coincidental value: a text anchor (name/title/snippet/message body) must
read as genuine human text (a string dominated by control/non-printable bytes is
rejected as carved binary that merely decoded as UTF-8), a `calls` row needs a
number *or* a duration beside its date, and a `calendar` event needs a date *or* a
location beside its title. (On a VACUUM-compacted backup this can correctly yield
zero recovered rows rather than a binary-noise "event".)

**Best-effort and partial**: recoverability depends on whether SQLite has reused
the space (VACUUM/auto_vacuum/page reuse and checkpointed WALs destroy remnants),
and carved rows can include false positives. The envelope carries `count`,
`stores: [{store, recovered}]`, `outputs`, `device`, and a `note` stating this.
Read-only; never writes to the backup. Absent databases are skipped.

### `schema-check` — detect SQLite schema drift

```
archive --backup <DIR> [--password <PW>] -o <OUT> schema-check -f <FORMAT>
```

`FORMAT` is `csv | json | html | pdf`; writes `<OUT>/schema-check.<ext>`. For every
SQLite store the tool extracts from (contacts, calls, accounts, data-usage,
voicemail, voice-memos, safari-history, safari-bookmarks, calendar, notes, photos,
whatsapp, health, device-usage), it resolves the database from the manifest —
trying each candidate location in order for stores whose DB moved across iOS
versions (device-usage probes all six `knowledgeC.db` paths) — opens it
**read-only**, and compares the live columns (`PRAGMA table_info`) against what that
store's extractor needs. Columns are split into two tiers mirroring how the
extractors build their SQL: **required** columns appear unconditionally (or guard
an early return), so their absence breaks extraction; **optional** columns are
gated with a NULL/COALESCE/skip fallback, so the export still succeeds without
them. Only a missing *required* column (or table) flags drift; a missing optional
column is reported for visibility but keeps the store `ok`. SQLite's implicit
`ROWID` is never in the required set (it is not a declared column, so
`PRAGMA table_info` never lists it). Each store reports `ok`, `drifted` (a required
table/column is gone — typically renamed/removed by a newer iOS, which makes that
extractor silently return empty), or `db_absent` (no candidate location is in this
backup — not a drift). Per store: `{ command, domain, rel_path, status, tables:
[{ table, status (ok|missing_columns|table_absent), missing_required,
missing_optional }] }`. The envelope carries `checked`, `ok_stores`, `drifted`,
`db_absent`, `stores`, `outputs`, `device`, and a `note`. The expectations are
empirically grounded against a real iOS 16 backup (all present stores report `ok`).
Use it to explain an unexpectedly-empty export: `drifted` means the schema moved,
`db_absent` means the data was never there. Read-only; never logs any row data,
only schema names.

### `search` — case-file search

```
archive --backup <DIR> [--password <PW>] -o <OUT> search -q <TERM> -f <FORMAT> [--redact]
```

`--redact` masks phone numbers and email local parts in the matched snippets
(same scheme as `timeline --redact`); matching still runs on the raw text, so a
phone-number query finds its records and then masks them in the output. The
envelope reports `redacted: <bool>`.

`FORMAT` is `csv | json | html | pdf`; `-q`/`--query` is the term to find (a phone
number, name, or keyword) and must be non-empty (else a usage error). Writes
`<OUT>/search.<ext>`. Runs every in-process extractor, builds the unified timeline,
and also loads the address book, then keeps the records whose one-line summary (or
the contact's name/organisation/note/number/email) contains `TERM` as a
case-insensitive substring. Each hit is `{ store (the source category: call,
whatsapp, note, safari, contacts, …), timestamp (ISO 8601 UTC or null), snippet }`,
timeline hits in chronological order followed by contact hits. **The match snippets
are personal data and are written only to the output file** — never to stderr; the
stdout envelope carries only `query` (the caller's own term), `matches` (the count),
`outputs`, and `device`. Best-effort: it searches the salient summaries the timeline
builds (message bodies, call numbers, titles, URLs, resolved names), so it answers
"what mentions X" rather than performing a full-text index of every column.
Read-only.

### `db-export` — combined SQLite database

```
archive --backup <DIR> [--password <PW>] -o <OUT> db-export
```

Takes no format flag — it always writes one SQLite database, `<OUT>/archive.sqlite`
(replacing any existing file), so an examiner can run cross-store SQL (joins,
time-range filters, `LIKE`) instead of opening one CSV per store. Tables: `timeline`
(`timestamp, kind, summary` — every dated event from every in-process extractor),
`contacts` (`first, last, organization, phones, emails, note`; phones/emails are
`; `-joined), `calls` (`date, number, contact_name, duration_seconds, direction,
answered, service`), and `whatsapp` (`date, chat, contact_name, from_me, text,
has_media`). Calls and WhatsApp carry a contact-name-enriched `contact_name` column;
the `timeline` table is the finalized (dated, chronological) view the `timeline`
command exports. The database is **plain unencrypted SQLite**
and holds personal data; the JSON envelope carries only per-table row counts
(`tables: { timeline, contacts, calls, whatsapp }`), `outputs` and `device` —
never the rows. Read-only on the backup.

### `diff` — file-level diff of two backups

```
archive --backup <A> [--password <PW>] -o <OUT> diff --against <B> -f <FORMAT>
```

`--backup` is backup **A** (older), `--against <B>` the second (newer) backup;
both open with the same `--password`. `FORMAT` is `csv | json | html | pdf`; writes
`<OUT>/backup-diff.<ext>`. Compares the two manifests keyed by `(domain,
relative_path)` and reports each file as `added` (only in B), `removed` (only in A),
or `modified` (present in both, logical size changed) — unchanged files are counted,
not listed. Each change row is `{ change, domain, path, size_a, size_b }`. The size
is the manifest's logical size, so the comparison works for **encrypted** backups
too (it never compares on-disk ciphertext lengths). It is a *structural* diff: it
flags that a file's content changed, not what changed inside it. The envelope
carries `summary: { added, removed, modified, unchanged }`, `outputs` and `device`.
Read-only on both backups.

### `package` — AES-256 encrypted zip of an export

```
archive -o <OUT> package --source <DIR> --zip-password <PW>
```

Bundles every file under `--source` (recursively, paths kept relative, sorted)
into `<OUT>/archive-package.zip` as a **WinZip AES-256 encrypted** zip — any
standard zip tool opens it with the password. The encryption covers each file's
**contents**; as with any encrypted zip the central directory is not encrypted, so
`unzip -l` still lists entry names and sizes (here the names are export/store names
like `contacts.csv`) — only their contents are protected. It does **not** open the backup (a
pure file operation; no `--backup`/`--password` needed), so it wraps a prior
`recover`/export output for delivery. The encryption password comes from
`--zip-password` or the `ARCHIVE_ZIP_PASSWORD` env var and is **required** (an
empty one is a usage error) and **never logged** — only the file and byte counts
are. If the output zip already lives under `--source`, it is skipped so the archive
never contains itself. Envelope: `ok`, `command`, `encrypted: true`, `cipher:
"AES-256"`, `files`, `bytes`, `outputs`.

### `wifi` — recover saved Wi-Fi passwords

```
archive --backup <DIR> --password <PW> -o <OUT> wifi -f <FORMAT>
```

`FORMAT` is `csv | json | html` (**no `pdf`** — it is intentionally rejected with
exit 1, because the shared PDF path writes a temporary plaintext HTML sidecar).
Writes `<OUT>/wifi.<ext>`. Recovers saved Wi-Fi PSKs from the backup keychain
(`KeychainDomain/keychain-backup.plist`), which is present **only in ENCRYPTED
backups** — on an unencrypted backup, one with no saved networks, or an
unsupported keychain layout it returns `count: 0` with a note. Each row is
`{ ssid, password }` with the password in **plaintext**. Passwords are written
only to the output file; they are **never logged** to stderr (progress prints a
count). Decryption: crabapple decrypts the keychain file and
exposes the protection-class keys; per-item keys are RFC3394-unwrapped and
AES-256-GCM-decrypted in `archive-core`. Wi-Fi PSKs come from the `genp`
AirPort items; website/app passwords have their own command (`passwords`).
Certs and legacy AES-CBC items remain out of scope. Envelope: `count`, `outputs`,
`device`, and a `note` flagging plaintext secrets. **Sensitive** — handle and
transmit securely.

### `passwords` — recover saved website/app passwords

```
archive --backup <DIR> --password <PW> -o <OUT> passwords -f <FORMAT>
```

Same model and security handling as `wifi` (`csv | json | html`, **no `pdf`**;
plaintext secrets **never logged**, only a count; encrypted backups only).
Recovers Safari/WebKit AutoFill website logins and third-party app passwords from
the keychain `inet` array. Each row is `{ service, account, password, protocol }`
with the password in **plaintext**. Items are filtered by their
entitlement-enforced access group: Safari/WebKit (`com.apple.cfnetwork`) and
third-party app groups are kept; Apple's internal iCloud-Keychain-sync groups
(`com.apple.security.ckks`, `com.apple.ProtectedCloudStorage`, …) — which hold
machine secrets, not passwords — are excluded. Returns `count: 0` with a note when
the backup has no keychain or no saved logins. **Sensitive** — handle and transmit
securely.

### `keychain-inventory` — non-secret keychain census

```
archive --backup <DIR> --password <PW> -o <OUT> keychain-inventory -f <FORMAT>
```

`FORMAT` is `csv | json | html | pdf` (**pdf allowed** — this output carries no
secrets). Lists per-item metadata across the keychain `genp`/`inet`/`cert`/`keys`
arrays: `{ array, service, account, access_group, protection_class, version,
decrypted }` — **never** a password or secret value. `decrypted: false` ≈ a
ThisDeviceOnly item not transferable in a portable backup. The envelope adds a
`summary` of per-array totals and decrypted counts. Encrypted backups only;
`count: 0` with a note otherwise. Useful to triage scope before exporting secrets
with `wifi`/`passwords`.

### `recover` — one-shot customer package

```
archive --backup <DIR> [--password <PW>] -o <OUT> recover [--no-files]
```

Runs **every in-process data-store extractor** in one shot into `<OUT>/`, writing
one HTML file per data type plus the media folders, plus a customer-facing
`<OUT>/index.html` landing page (device sheet — name, model, serial, iOS, UDID —
and a table linking each export with counts). The backup is opened once.
`--no-files` skips the large media extraction (metadata + HTML only). A store
absent from the backup is skipped; one unreadable store is logged and skipped, not
fatal. The `photos-recently-deleted` section (recovered into `recently-deleted/`)
is added only when the Camera Roll has trashed assets still in the purge window. Two things are **not** in the package: iMessage/SMS/RCS conversation
transcripts (run the standalone `messages` command) and the installed-app
inventory (run `apps`) — the first is a subprocess export, the second is
manifest-derived rather than a data store.

stdout envelope:

```json
{
  "ok": true,
  "command": "recover",
  "outputs": ["<OUT>/index.html", "<OUT>/contacts.html", "..."],
  "sections": [
    { "type": "contacts", "file": "contacts.html", "count": 1234 },
    { "type": "photos", "file": "photos.html", "count": 1240,
      "files": { "dir": "photos", "extracted": 1238, "missing": 2 } },
    { "type": "photos-recently-deleted", "file": "photos-recently-deleted.html", "count": 7,
      "files": { "dir": "recently-deleted", "extracted": 7, "missing": 0 } },
    { "type": "homescreen-layout", "file": "homescreen-layout.html", "count": 45 },
    { "type": "data-usage", "file": "data-usage.html", "count": 413 },
    { "type": "device-usage", "file": "device-usage.html", "count": 87 },
    { "type": "bluetooth-devices", "file": "bluetooth-devices.html", "count": 1005 }
  ],
  "device": { "name": "iPhone", "model": "iPhone14,2", "ios": "17.5", "serial": "F2L...", "udid": "00008..." }
}
```

`outputs[0]` is always `index.html`. `--zip` packaging is not yet implemented (zip
the `<OUT>` folder yourself).

### `backup` — create a backup from a connected iPhone

```
archive -o <OUT> backup [--full]
```

Creates a fresh iOS backup from a USB-connected iPhone via `libimobiledevice`'s
`idevicebackup2`, into `<OUT>/<udid>/`. Does **not** take `--backup`. `--full`
forces a full backup (default is incremental when `<OUT>` already holds one).
Requires `idevicebackup2` and `idevice_id` on `PATH` (missing → exit 1 with an
install hint such as `brew install libimobiledevice`); no connected device → exit
1. Live progress from `idevicebackup2` streams to stderr.

stdout envelope:

```json
{
  "ok": true,
  "command": "backup",
  "dir": "<OUT>/<udid>",
  "udid": "<udid>",
  "device": { "name": "...", "model": "iPhone14,2", "ios": "17.5", "serial": "...", "udid": "..." }
}
```

`device` is omitted (with an explanatory `note`) if the fresh backup cannot be
read for device info (e.g. it is encrypted). Afterward, point the other commands
at it: `archive --backup <OUT>/<udid> recover`.

### `integrity` — verify the backup is complete (read-only)

```
archive --backup <DIR> [--password <PW>] integrity
```

Read-only; no `--out`. Checks that every regular-file entry in the backup's
`Manifest.db` has its stored file present on disk (and, for **unencrypted**
backups only, that the on-disk size matches the recorded size — encrypted blobs
are AES-padded so size is not checked there). Content hashing is not possible from
the manifest (the file id is path-derived, not content-derived).

stdout:

```json
{
  "ok": true,
  "command": "integrity",
  "complete": false,
  "total_files": 48213,
  "present": 48200,
  "missing": 13,
  "size_checked": true,
  "size_mismatch": 0,
  "missing_sample": ["CameraRollDomain:Media/DCIM/100APPLE/IMG_0007.HEIC"],
  "mismatch_sample": [],
  "device": { "name": "...", "model": "...", "ios": "...", "serial": "...", "udid": "..." }
}
```

`complete` = `missing == 0 && size_mismatch == 0`. `ok` stays `true` even when
incomplete (a report, not a failure — exit 0); the sample lists are capped (the
counts carry the true totals). Encrypted backups set `size_checked: false` and add
a `note`.

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
malformed (a missing subcommand, or an unknown flag), `clap` prints plain-text
usage to **stderr**, writes **nothing** to stdout, and also exits **2** — there is
no JSON envelope. (A missing `--backup` on a read command is **not** a clap error
— `--backup` is optional at parse time so the `backup` command can omit it — it is
a runtime usage error: a JSON `"kind": "usage"` envelope on stdout, exit 1.)
Disambiguate exit 2 by inspecting stdout: a JSON object with `"kind": "auth"` is an
authentication failure; empty stdout (with usage text on stderr) is a malformed
invocation. When unsure, run `--help` or `inspect` first to learn the contract.

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

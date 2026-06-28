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
calls, voicemail, voice-memos, safari-history/bookmarks, calendar, reminders,
mail, notes, photos, attachments, whatsapp, timeline, recover-deleted, health,
apps) also accept **`pdf`**: their HTML is printed to `<OUT>/<name>.pdf` by a
headless Chrome/Chromium/Edge (auto-detected on `PATH`/standard locations or set
with `--chrome-path`; a missing browser is a usage error, exit 1), with the JSON
envelope unchanged (`outputs` points at the `.pdf`). `messages -f pdf` is produced
**separately** by the bundled imessage-exporter using its own PDF engine (Quartz
on macOS, a headless browser elsewhere; `--chrome-path` is forwarded), and keeps
the messages envelope (`output` directory). The `recover` one-shot package has no
`-f pdf` and stays HTML.

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

Each asset: `filename`, `kind` (`image`/`video`/`unknown`), `created` /
`modified` / `added` (ISO 8601 UTC; empty when unset), `favorite`, `hidden` (in
the Hidden album — still recovered, just flagged), `trashed` (in Recently
Deleted), `edited` (has adjustments), `live_photo` (best-effort) with raw
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
(output-relative extracted media, or `null`). Envelope carries a `files` object
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
archive --backup <DIR> [--password <PW>] -o <OUT> timeline -f <FORMAT>
```

`FORMAT` is `csv | json | html`. Writes `<OUT>/timeline.<ext>`. Merges every
in-process extractor (calls, voicemail, voice memos, Safari history, calendar,
notes, photos, attachments, WhatsApp, reminders, Health workouts, mail) into one
stream of events, each `{ timestamp (ISO 8601 UTC), kind, summary }`, sorted
chronologically (undated records are dropped). Each source is best-effort: an
absent or unreadable store is skipped, not fatal. A **view** over the other
extractors, not a data store — like `apps`, it is **not** listed by `inspect`
and **not** part of `recover`. `messages` (conversation text, exported out of
process) and `apps` (not an event stream) are excluded. Envelope: `ok`,
`command`, `count` (total events), `outputs`, `device`.

### `recover-deleted` — carve deleted SQLite rows

```
archive --backup <DIR> [--password <PW>] -o <OUT> recover-deleted -f <FORMAT> [--store messages|calls|contacts|all]
```

`FORMAT` is `csv | json | html`; `--store` defaults to `all` (an unknown value is
a usage error, exit 1). Writes `<OUT>/deleted.<ext>`. Recovers **deleted** rows
from the backup's SQLite databases by carving freed regions — freelist pages,
in-page freeblocks, the unallocated gap, and the `-wal` sidecar — with a generic
schema-less parser (`archive-core::carve`), then attributes each carved record to
a store via heuristic signatures: `messages` (`sms.db`, anchored by a 36-char
GUID), `calls` (`CallHistory.storedata`, anchored by a Cocoa-seconds REAL date),
`contacts` (`AddressBook.sqlitedb`, name texts — softest). Each output row is
`{ store, source (freelist|freeblock|unallocated|wal), rowid, date (ISO 8601 UTC
or null), summary }`, sorted chronologically.

Rows still **live** in the database are excluded (a carved candidate whose cell
rowid — or, for messages, GUID — is still present in the live table is dropped).
Because `-wal` frames are full page images mixing live and deleted cells, and
calls/contacts lack a strong content anchor, **`calls` and `contacts` ignore WAL
candidates entirely** (their deletions still surface from the main file's free
regions); `messages` use the GUID anchor and do recover from the WAL.

**Best-effort and partial**: recoverability depends on whether SQLite has reused
the space (VACUUM/auto_vacuum/page reuse and checkpointed WALs destroy remnants),
and carved rows can include false positives. The envelope carries `count`,
`stores: [{store, recovered}]`, `outputs`, `device`, and a `note` stating this.
Read-only; never writes to the backup. Absent databases are skipped.

### `wifi` — recover saved Wi-Fi passwords

```
archive --backup <DIR> --password <PW> -o <OUT> wifi -f <FORMAT>
```

`FORMAT` is `csv | json | html | pdf`. Writes `<OUT>/wifi.<ext>`. Recovers saved
Wi-Fi PSKs from the backup keychain (`KeychainDomain/keychain-backup.plist`),
which is present **only in ENCRYPTED backups** — on an unencrypted backup, one
with no saved networks, or an unsupported keychain layout it returns `count: 0`
with a note. Each row is `{ ssid, password }` with the password in **plaintext**.
Passwords are written only to the output file; they are **never logged** to stderr
(progress prints a count). Decryption: crabapple decrypts the keychain file and
exposes the protection-class keys; per-item keys are RFC3394-unwrapped and
AES-256-GCM-decrypted in `archive-core`. v1 is Wi-Fi only (website/app passwords,
certs, and legacy AES-CBC items are out of scope). Envelope: `count`, `outputs`,
`device`, and a `note` flagging plaintext secrets. **Sensitive** — handle and
transmit securely.

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
fatal. Two things are **not** in the package: iMessage/SMS/RCS conversation
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
      "files": { "dir": "photos", "extracted": 1238, "missing": 2 } }
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

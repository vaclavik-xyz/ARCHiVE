# archive

Extract personal data from an on-disk iOS backup (encrypted or not) into
machine-readable and human-readable formats.

**Agents:** read [`../AGENTS.md`](../AGENTS.md) — every command emits one JSON
object on stdout with stable exit codes, progress on stderr.

## Quick start

```
# Create a fresh backup from a USB-connected iPhone (needs libimobiledevice)
archive -o <out> backup            # writes <out>/<udid>/

# Discover what the backup contains (read-only)
archive --backup <backup-dir> inspect

# Verify the backup is complete / not truncated (read-only)
archive --backup <backup-dir> integrity

# One-shot: recover everything into <out>/ with a customer index.html (--no-files for metadata only)
archive --backup <backup-dir> -o <out> recover

# Export each data type to <out>/
archive --backup <backup-dir> -o <out> contacts  -f vcf    # csv | json | vcf | html
archive --backup <backup-dir> -o <out> calls     -f json   # csv | json | html
archive --backup <backup-dir> -o <out> voicemail -f json   # csv | json | html

# Extract voicemail metadata + audio (raw .amr; pass --audio-format m4a|wav to transcode via ffmpeg)
archive --backup <backup-dir> -o <out> voicemail -f json --audio

# Extract Voice Memos metadata + audio (audio on by default; --no-audio for metadata only)
archive --backup <backup-dir> -o <out> voice-memos -f html

# Safari history / bookmarks and calendar events
archive --backup <backup-dir> -o <out> safari-history   -f json
archive --backup <backup-dir> -o <out> safari-bookmarks -f json
archive --backup <backup-dir> -o <out> calendar         -f html

# Apple Notes (body decoded from gzip+protobuf, snippet fallback)
archive --backup <backup-dir> -o <out> notes -f html

# Camera Roll: metadata + extract photo/video files into <out>/photos/ (--no-files for catalog only)
archive --backup <backup-dir> -o <out> photos -f html

# Messages attachments: extract media into <out>/attachments/ (--no-files for catalog only)
archive --backup <backup-dir> -o <out> attachments -f html

# WhatsApp messages + media (--no-files for transcript only)
archive --backup <backup-dir> -o <out> whatsapp -f html

# iMessage/SMS/RCS conversation transcript (drives the bundled imessage-exporter)
archive --backup <backup-dir> -o <out> messages -f html   # txt | html | pdf

# Apple Health: workouts + per-type quantity summaries (steps, heart rate, …)
archive --backup <backup-dir> -o <out> health -f html

# Apple Reminders (lists, items, due/completion, priority)
archive --backup <backup-dir> -o <out> reminders -f html

# Apple Mail (.emlx; iOS backs up mail only for local/POP3 mailboxes — often empty)
archive --backup <backup-dir> -o <out> mail -f html

# Installed third-party apps (bundle ids) from the backup manifest
archive --backup <backup-dir> -o <out> apps -f json
```

Encrypted backups: pass `--password` or set `ARCHIVE_PASSWORD` (never prompts).

`messages` drives the `imessage-exporter` binary built alongside `archive`. It
is found next to the `archive` executable or on `PATH`; set
`ARCHIVE_IMESSAGE_EXPORTER` to point at it explicitly. The transcript tree is
written under `<out>/messages`.

## Status

- [x] inspect — store discovery (read-only)
- [x] contacts — csv, json, vcf, html (incl. postal addresses)
- [x] calls — csv, json, html
- [x] voicemail — csv, json, html (metadata) + audio extraction (`--audio`, raw `.amr` or ffmpeg `m4a`/`wav`)
- [x] voice-memos — csv, json, html (metadata) + audio extraction (native copy by default, or ffmpeg `m4a`/`wav`)
- [x] safari-history · safari-bookmarks · calendar — csv, json, html
- [x] notes — csv, json, html (body decoded from gzip+protobuf, snippet fallback)
- [x] photos — csv, json, html gallery + file extraction; albums, hidden, Live/burst, edited, GPS, original name/title
- [x] attachments — csv, json, html gallery + Messages attachment file extraction
- [x] recover — one-shot: all in-process data-store extractors + customer index.html (device sheet + links; excludes `messages` and `apps`)
- [x] backup — create a fresh backup from a connected iPhone (libimobiledevice)
- [x] integrity — verify backup completeness (manifest file presence + size)
- [x] whatsapp — csv, json, html transcript + media extraction
- [x] messages — iMessage/SMS/RCS transcript (txt, html, pdf) via the bundled imessage-exporter
- [x] health — csv, json, html: workouts + per-type quantity summaries (HealthDomain)
- [x] reminders — csv, json, html: lists, items, due/completion, priority (Core Data store)
- [x] mail — csv, json, html: local/POP3 `.emlx` messages (best-effort; usually absent on iOS)
- [x] apps — csv, json, html: installed third-party app bundle ids (manifest-derived)
- [ ] pdf output for the in-process extractors

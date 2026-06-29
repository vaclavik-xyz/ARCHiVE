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

# Configured accounts (Apple ID, Google, Exchange, IMAP, CalDAV/CardDAV, …); metadata only, no passwords
archive --backup <backup-dir> -o <out> accounts  -f json   # csv | json | html

# Remembered Wi-Fi networks (SSID list, no passwords); works on any backup, but the
# plaintext list is usually empty on iOS 16+ (use `wifi` for keychain SSIDs+passwords)
archive --backup <backup-dir> -o <out> known-networks -f json   # csv | json | html

# Home Screen layout (pages, dock, folders, widget stacks) from IconState.plist; works on any backup
archive --backup <backup-dir> -o <out> homescreen-layout -f html   # csv | json | html | pdf

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

# Recently Deleted: recover trashed photos/videos still in the 30-day window into <out>/recently-deleted/
archive --backup <backup-dir> -o <out> photos-recently-deleted -f html

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

# Unified chronological timeline merging every in-process extractor
archive --backup <backup-dir> -o <out> timeline -f html

# Activity dashboard: per-category event counts + date ranges (a view over the timeline)
archive --backup <backup-dir> -o <out> stats -f html   # csv | json | html | pdf

# Recover DELETED rows by carving freed SQLite pages/WAL (best-effort)
archive --backup <backup-dir> -o <out> recover-deleted -f html   # --store messages|calls|contacts|all

# Recover saved Wi-Fi passwords from the keychain (ENCRYPTED backups only)
archive --backup <backup-dir> --password <pw> -o <out> wifi -f html

# Recover saved website/app passwords from the keychain (ENCRYPTED backups only; plaintext)
archive --backup <backup-dir> --password <pw> -o <out> passwords -f html

# Keychain census: per-item metadata (service, account, group, class) with NO secrets
archive --backup <backup-dir> --password <pw> -o <out> keychain-inventory -f html
```

Encrypted backups: pass `--password` or set `ARCHIVE_PASSWORD` (never prompts).

The in-process HTML commands also accept `-f pdf`: their HTML is rendered to PDF
with a headless Chrome/Chromium/Edge (auto-detected, or pass `--chrome-path`; a
missing browser is a usage error). `messages -f pdf` goes through the bundled
imessage-exporter instead (its own PDF engine); the `recover` package stays HTML.

`messages` drives the `imessage-exporter` binary built alongside `archive`. It
is found next to the `archive` executable or on `PATH`; set
`ARCHIVE_IMESSAGE_EXPORTER` to point at it explicitly. The transcript tree is
written under `<out>/messages`.

## Status

- [x] inspect — store discovery (read-only)
- [x] contacts — csv, json, vcf, html (incl. postal addresses)
- [x] calls — csv, json, html (numbers resolved to `contact_name` from the address book when available)
- [x] accounts — csv, json, html: configured accounts (Apple ID, Google, Exchange, …) from `Accounts3.sqlite`; metadata only, no passwords
- [x] known-networks — csv, json, html: remembered Wi-Fi SSIDs (no passwords) from `com.apple.wifi*.plist`; works on any backup, but the plaintext list is usually empty on iOS 16+ (the inventory moved to the keychain — see `wifi`)
- [x] homescreen-layout — csv, json, html, pdf: Home Screen pages, dock, folders and widget stacks from SpringBoard's `IconState.plist`; works on any backup
- [x] voicemail — csv, json, html (metadata) + audio extraction (`--audio`, raw `.amr` or ffmpeg `m4a`/`wav`)
- [x] voice-memos — csv, json, html (metadata) + audio extraction (native copy by default, or ffmpeg `m4a`/`wav`)
- [x] safari-history · safari-bookmarks · calendar — csv, json, html
- [x] notes — csv, json, html (body decoded from gzip+protobuf, snippet fallback)
- [x] photos — csv, json, html gallery + file extraction; albums, hidden, Live/burst, edited, GPS, original name/title
- [x] photos-recently-deleted — csv, json, html + file recovery of trashed assets still in the 30-day purge window (with estimated purge date)
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
- [x] timeline — csv, json, html: every in-process extractor merged into one chronological stream
- [x] stats — csv, json, html, pdf: activity dashboard (per-category event counts + date ranges; a view over the timeline)
- [x] recover-deleted — csv, json, html: carve DELETED rows (messages/calls/contacts) from freed SQLite pages (+ WAL for messages) (best-effort)
- [x] wifi — csv, json, html: recover saved Wi-Fi passwords from the keychain (encrypted backups only; passwords in plaintext)
- [x] passwords — csv, json, html: recover saved website/app passwords from the keychain `inet` array (Safari/WebKit `com.apple.cfnetwork` + third-party app groups; Apple-internal keychain-sync items excluded); encrypted backups only, plaintext
- [x] keychain-inventory — csv, json, html, pdf: non-secret census of the keychain (per-item service/account/group/protection-class/version across genp/inet/cert/keys; NO passwords) — triage scope before exporting secrets; encrypted backups only
- [x] pdf output — `-f pdf` on every HTML-bearing command, rendered from the HTML via a headless browser

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

# Per-process network data usage (cellular/Wi-Fi byte counters) from DataUsage.sqlite
archive --backup <backup-dir> -o <out> data-usage -f html   # csv | json | html | pdf

# Per-app foreground usage (time, sessions) from CoreDuet's knowledgeC.db (often excluded from iOS 16+ backups)
archive --backup <backup-dir> -o <out> device-usage -f html   # csv | json | html | pdf

# Paired + previously-seen Bluetooth devices (names, MAC addresses) from the system Bluetooth databases
archive --backup <backup-dir> -o <out> bluetooth-devices -f html   # csv | json | html | pdf

# Recorded location history from the routined "Significant Locations" DB (usually excluded from standard backups)
archive --backup <backup-dir> -o <out> significant-locations -f html   # csv | json | html | pdf

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

# Unified chronological timeline merging every in-process extractor (--redact masks phone numbers/emails for sharing)
archive --backup <backup-dir> -o <out> timeline -f html [--redact]

# Activity dashboard: per-category event counts + date ranges (a view over the timeline)
archive --backup <backup-dir> -o <out> stats -f html   # csv | json | html | pdf

# Per-app database recoverability: readable plain SQLite (with table count) vs encrypted/other
archive --backup <backup-dir> -o <out> app-databases -f html   # csv | json | html | pdf

# Extract a named app's document/media files (use when the message DB is excluded/encrypted)
archive --backup <backup-dir> -o <out> app-files --app viber -f html   # media only; add --all for every file

# Recover DELETED rows by carving freed SQLite pages/WAL (best-effort)
archive --backup <backup-dir> -o <out> recover-deleted -f html   # --store messages|calls|contacts|notes|calendar|safari|photos|all

# Schema drift self-check: do the live SQLite stores still carry the columns each extractor needs?
archive --backup <backup-dir> -o <out> schema-check -f html   # csv | json | html | pdf

# Case-file search: find one term (phone, name, keyword) across every record + the address book
archive --backup <backup-dir> -o <out> search -q "+420776452878" -f html [--redact]   # csv | json | html | pdf

# Combined SQLite export: all data in one queryable <out>/archive.sqlite (timeline + contacts + calls + whatsapp)
archive --backup <backup-dir> -o <out> db-export

# Diff two backups at the file level: which manifest files were added/removed/changed between A and B
archive --backup <backup-A> -o <out> diff --against <backup-B> -f html   # csv | json | html | pdf

# Package a prior export directory into one AES-256 encrypted zip for delivery
archive -o <out> package --source <export-dir> --zip-password <pw>   # or set ARCHIVE_ZIP_PASSWORD

# Recover saved Wi-Fi passwords from the keychain (ENCRYPTED backups only)
archive --backup <backup-dir> --password <pw> -o <out> wifi -f html

# Recover saved website/app passwords from the keychain (ENCRYPTED backups only; plaintext)
archive --backup <backup-dir> --password <pw> -o <out> passwords -f html

# Keychain census: per-item metadata (service, account, group, class) with NO secrets
archive --backup <backup-dir> --password <pw> -o <out> keychain-inventory -f html

# X.509 certificates from the keychain → certificates.pem bundle + metadata (no private keys)
archive --backup <backup-dir> --password <pw> -o <out> certificates -f html
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
- [x] data-usage — csv, json, html, pdf: per-process cellular/Wi-Fi byte counters from `DataUsage.sqlite` (ZLIVEUSAGE aggregated per process)
- [x] device-usage — csv, json, html, pdf: per-app foreground time + sessions from CoreDuet's `knowledgeC.db` (`/app/usage` stream); the store is often excluded from iOS 16+ backups, then reports an honest 0
- [x] bluetooth-devices — csv, json, html, pdf: paired, classic and previously-seen Bluetooth devices (name, address, resolved identity address) from the LE `com.apple.MobileBluetooth.ledevices.{paired,other}.db` databases and the classic `com.apple.MobileBluetooth.devices.plist`; the DBs' last-seen/connection columns are device-relative counters (not a wall-clock epoch) and are deliberately not exported as dates
- [x] significant-locations — csv, json, html, pdf: recorded location-fix history (timestamp, latitude/longitude, altitude, accuracy, speed) from the routined `Cache.sqlite`/`cloud.sqlite`/`local.sqlite` (`ZRTCLLOCATIONMO`) — the store behind iOS *Significant Locations*. The routined database lives under `Library/Caches`, which iOS excludes from ordinary iTunes/Finder backups, so this usually reports an honest 0; it still recovers history from full filesystem extractions that include the caches
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
- [x] timeline — csv, json, html: every in-process extractor merged into one chronological stream; `--redact` masks phone numbers and email local parts (names kept) for shareable output
- [x] search `--redact` · timeline `--redact` — mask the strongest identifiers (phone numbers → last 2 digits, email local part → first char) in the output so a report can be shared; matching still runs on the raw text
- [x] stats — csv, json, html, pdf: activity dashboard (per-category event counts + date ranges; a view over the timeline)
- [x] app-databases — csv, json, html, pdf: per-app database recoverability report (readable plain SQLite + table count vs encrypted/Core-Data/other); shows that many third-party messaging apps keep no readable store in the backup
- [x] app-files — csv, json, html, pdf: extract a named app's document/media files from its `AppDomain-…`/`AppDomainGroup-…` containers (media only by default, `--all` for every file); recovers the photos/videos/voice messages that survive even when the message DB is excluded
- [x] recover-deleted — csv, json, html: carve DELETED rows (messages/calls/contacts/notes/calendar/safari/photos) from freed SQLite pages (+ WAL for messages) (best-effort); `truncated` flags partially-recovered rows
- [x] schema-check — csv, json, html, pdf: validate each live SQLite store against the columns its extractor depends on, flagging drift (renamed/removed columns across iOS versions) vs `ok` vs `db_absent`; explains an unexpectedly-empty export
- [x] search — csv, json, html, pdf: case-file search of one term across every in-process record (the unified timeline) and the address book; match snippets go only to the output file (never stderr), the JSON envelope carries just the count
- [x] db-export — one queryable `archive.sqlite` consolidating the unified timeline plus structured contacts/calls/whatsapp tables, for cross-store SQL (joins, time ranges, LIKE); the JSON envelope carries only per-table row counts
- [x] diff — csv, json, html, pdf: file-level diff of two backups (`--backup` A vs `--against` B), flagging manifest files added/removed/modified (logical size changed); manifest sizes make it work for encrypted backups; envelope carries the added/removed/modified/unchanged counts
- [x] package — bundle a prior export directory into one **WinZip AES-256 encrypted** `archive-package.zip` for secure delivery (password via `--zip-password` or `ARCHIVE_ZIP_PASSWORD`, never logged); any standard zip tool opens it with the password
- [x] wifi — csv, json, html: recover saved Wi-Fi passwords from the keychain (encrypted backups only; passwords in plaintext)
- [x] passwords — csv, json, html: recover saved website/app passwords from the keychain `inet` array (Safari/WebKit `com.apple.cfnetwork` + third-party app groups; Apple-internal keychain-sync items excluded); encrypted backups only, plaintext
- [x] keychain-inventory — csv, json, html, pdf: non-secret census of the keychain (per-item service/account/group/protection-class/version across genp/inet/cert/keys; NO passwords) — triage scope before exporting secrets; encrypted backups only
- [x] certificates — csv, json, html, pdf: recover X.509 certificates from the keychain `cert` array → a `certificates.pem` bundle plus a metadata table (subject/issuer CN, serial, validity, CA flag, and whether a matching private key makes it an identity); **public certificates only — no private key material is exported**. Encrypted backups only; certs stored under a *ThisDeviceOnly* protection class are not transferable in a portable backup and cannot be decrypted (then reports an honest 0)
- [x] pdf output — `-f pdf` on every HTML-bearing command, rendered from the HTML via a headless browser

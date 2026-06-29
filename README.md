# ARCHiVE

Extract your personal data from iOS backups — messages, photos & videos,
contacts, calls, voicemail, voice memos, notes, Safari history/bookmarks,
calendar, WhatsApp, and more — from the command line, built to be driven equally
well by humans and AI agents.

ARCHiVE is a Rust workspace bundling two CLIs that read an on-disk iPhone/iPad
backup (encrypted or not) and give you full ownership of your data in open,
portable formats:

- **`archive`** — a full iOS-backup **recovery toolset**: contacts, calls,
  voicemail, voice memos, Safari, calendar, notes, photos & videos, message
  attachments, WhatsApp, iMessage/SMS/RCS transcripts, Health, Reminders, Mail,
  an installed-app inventory, a configured-accounts inventory, and a unified
  chronological `timeline` — plus a
  one-shot `recover` package, deleted-record recovery (`recover-deleted`, SQLite
  carving), saved Wi-Fi passwords (`wifi`, from the keychain), on-device `backup`
  capture, and a backup `integrity` check (agent-first JSON output)
- **`imessage-exporter`** — iMessage / SMS / RCS conversations and attachments

```bash
cargo build --release   # binaries: target/release/{archive, imessage-exporter}
```

---

## `archive` — iOS backup recovery toolset

An **agent-first** extractor: every command prints exactly one JSON object to
stdout, human progress goes to stderr, and exit codes are stable. Export formats
are `csv` / `json` / `vcf` / `html` depending on the data type; the per-type
export commands also render **`pdf`** via a headless Chrome/Chromium/Edge (the
`messages` transcript renders PDF through the bundled exporter instead; the
`recover` package stays HTML). Media-bearing types also extract the actual files
(photos, videos, audio, attachments).

```bash
# Triage a backup (read-only): what's in it, and is it complete?
archive --backup ~/Backup/<UDID> inspect
archive --backup ~/Backup/<UDID> integrity

# One-shot: recover every in-process data-store extractor into out/ with a
# customer-ready index.html (the `messages` transcript and `apps` inventory are
# separate commands)
archive --backup ~/Backup/<UDID> -o out recover        # --no-files for metadata only

# Or capture a fresh backup from a USB-connected iPhone first (libimobiledevice)
archive -o out backup                                   # writes out/<UDID>/

# Per data type (csv | json | vcf | html; media types extract files by default)
archive --backup ~/Backup/<UDID> -o out contacts        -f vcf   # incl. postal addresses
archive --backup ~/Backup/<UDID> -o out calls           -f json
archive --backup ~/Backup/<UDID> -o out accounts        -f json  # configured accounts (Apple ID, Google, Exchange, …)
archive --backup ~/Backup/<UDID> -o out voicemail       -f json --audio
archive --backup ~/Backup/<UDID> -o out voice-memos     -f html
archive --backup ~/Backup/<UDID> -o out safari-history  -f json
archive --backup ~/Backup/<UDID> -o out safari-bookmarks -f json
archive --backup ~/Backup/<UDID> -o out calendar        -f html
archive --backup ~/Backup/<UDID> -o out notes           -f html  # body decoded from gzip+protobuf
archive --backup ~/Backup/<UDID> -o out photos          -f html  # gallery: albums, hidden, Live/burst, GPS
archive --backup ~/Backup/<UDID> -o out attachments     -f html  # Messages media gallery
archive --backup ~/Backup/<UDID> -o out whatsapp        -f html  # transcript + media
archive --backup ~/Backup/<UDID> -o out messages        -f html  # iMessage/SMS/RCS transcript (txt|html|pdf)
archive --backup ~/Backup/<UDID> -o out health          -f html  # workouts + quantity summaries
archive --backup ~/Backup/<UDID> -o out reminders       -f html  # lists, items, due/completion
archive --backup ~/Backup/<UDID> -o out mail            -f html  # local/POP3 .emlx (often empty on iOS)
archive --backup ~/Backup/<UDID> -o out apps            -f json  # installed app bundle ids
archive --backup ~/Backup/<UDID> -o out timeline        -f html  # everything merged chronologically
archive --backup ~/Backup/<UDID> -o out recover-deleted -f html  # carve deleted rows (best-effort)
archive --backup ~/Backup/<UDID> -o out wifi            -f html  # saved Wi-Fi passwords (encrypted backups)
```

The `messages` command drives the `imessage-exporter` binary (built in the same
workspace, found next to `archive` or on `PATH`, or via
`ARCHIVE_IMESSAGE_EXPORTER`) and writes the transcript under `<out>/messages`.

Encrypted backups: pass `--password` or set `ARCHIVE_PASSWORD` (never prompts).
The canonical, machine-readable contract (every command's flags, envelope, and
exit codes) lives in **[AGENTS.md](AGENTS.md)**; a per-type checklist is in
**[archive/README.md](archive/README.md)**. Crates: `archive` (the CLI) over
`archive-core` (the crabapple-backed open/decrypt/fetch layer); neither depends
on the Messages tooling below.

---

## `imessage-exporter` — iMessage, SMS & RCS

This component provides both a library to interact with iMessage data and a
binary that performs useful read-only operations using that data. The aim is to
provide the most comprehensive and accurate representation of iMessage data
available. It can:

- Save, export, backup, and archive iMessage data to open, portable formats
- Preserve multimedia content (images, videos, audio) from conversations
- Facilitate easy migration of message history between devices and platforms
- Run diagnostics on the iMessage database
- Give you full ownership and control over your communication history
- Support compliance with data retention policies or legal requirements
- Run on macOS, Linux, and Windows

### Example Export

![HTML Export Sample](/docs/hero.png)

### Binary

The `imessage-exporter` binary exports iMessage data to `txt`, `html`, or `pdf`
formats. PDF export renders one document per conversation using Apple's Quartz
engine on macOS or a headless Chrome/Chromium/Edge browser elsewhere. It can also
run diagnostics to find problems with the iMessage database.

Installation instructions for the binary are located [here](imessage-exporter/README.md).

### Library

The `imessage_database` library provides models that allow us to access iMessage
information as native, cross-platform data structures.

Documentation for the library is located [here](imessage-database/README.md).

### Supported Features

This component supports every iMessage feature as of macOS Tahoe 26.5.1 (25F80) and iOS 26.5.1 (23F81):

- iMessage, RCS, SMS, and MMS
- Multi-part messages
- Replies/Threads
- Formatted text
- Attachments
- Expressives
- Tapbacks
- Stickers
- Apple Pay
- Group chats
- Digital Touch
- URL Previews
- Audio messages
- App Integrations
- Edited messages
- Business messages
- Handwritten messages

See more detail about supported features [here](docs/features.md).

This component tracks [ReagentX/imessage-exporter](https://github.com/ReagentX/imessage-exporter)
upstream so it stays current with its active Messages development.

## Frequently Asked Questions

The FAQ document is located [here](/docs/faq.md).

## Provenance & license

The `imessage-database` and `imessage-exporter` crates originate from
[ReagentX/imessage-exporter](https://github.com/ReagentX/imessage-exporter) and
are licensed **GPL-3.0**. ARCHiVE is a derivative work and is therefore released
under **GPL-3.0-or-later** (see [LICENSE](LICENSE)). The `archive` and
`archive-core` crates are original to this project.

## Special Thanks

- All of my friends, for putting up with me sending them random messages to test things
- [SQLiteFlow](https://www.sqliteflow.com), the SQL viewer I used to explore and reverse engineer the iMessage database
- [Xplist](https://github.com/ic005k/Xplist), an invaluable tool for reverse engineering the `payload_data` plist format
- [Compart](https://www.compart.com/en/unicode/), an amazing resource for looking up esoteric unicode details
- [GNU Project](https://github.com/gnustep/libobjc) and [Archive.org](https://archive.org/details/darwin_0.1), for hosting source code referenced to reverse engineer the `typedstream` format

# ARCHiVE

Extract your personal data from iOS backups — messages, contacts, calls, and
voicemail — from the command line, built to be driven equally well by humans and
AI agents.

ARCHiVE is a Rust workspace bundling two CLIs that read an on-disk iPhone/iPad
backup (encrypted or not) and give you full ownership of your data in open,
portable formats:

- **`archive`** — contacts, calls, and voicemail (agent-first JSON output)
- **`imessage-exporter`** — iMessage / SMS / RCS conversations and attachments

```bash
cargo build --release   # binaries: target/release/{archive, imessage-exporter}
```

---

## `archive` — contacts · calls · voicemail

An **agent-first** extractor: every command prints exactly one JSON object to
stdout, human progress goes to stderr, and exit codes are stable. Output formats
are `csv` / `json` / `vcf` / `html` depending on the data type.

```bash
# Discover what a backup contains (read-only)
archive --backup ~/Backup/<UDID> inspect

# Export each data type to out/
archive --backup ~/Backup/<UDID> -o out contacts  -f vcf    # incl. postal addresses
archive --backup ~/Backup/<UDID> -o out calls     -f json
archive --backup ~/Backup/<UDID> -o out voicemail -f json
```

Encrypted backups: pass `--password` or set `ARCHIVE_PASSWORD` (never prompts).
The canonical, machine-readable contract lives in **[AGENTS.md](AGENTS.md)**.
Crates: `archive` (the CLI) over `archive-core` (the crabapple-backed
open/decrypt/fetch layer); neither depends on the Messages tooling below.

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

The `imessage-exporter` binary exports iMessage data to `txt` or `html` formats.
It can also run diagnostics to find problems with the iMessage database.

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

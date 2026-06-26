# ARCHiVE

Extract your personal data from iOS backups — messages, contacts, calls, and
voicemail — from the command line, built to be driven equally well by humans and
AI agents.

ARCHiVE is a Rust workspace with two binaries that read an on-disk iPhone/iPad
backup (encrypted or not) and turn it into useful, portable files.

## Components

### `archive` — personal data (contacts · calls · voicemail)

An **agent-first** extractor: every command prints exactly one JSON object to
stdout, human progress goes to stderr, and exit codes are stable. Output formats
are `csv` / `json` / `vcf` / `html` depending on the data type.

```bash
# Discover what a backup contains (read-only)
archive --backup ~/Backup/<UDID> inspect

# Export contacts (with postal addresses) as importable vCard
archive --backup ~/Backup/<UDID> -o out contacts -f vcf

# Export call history and voicemail metadata as JSON
archive --backup ~/Backup/<UDID> -o out calls -f json
archive --backup ~/Backup/<UDID> -o out voicemail -f json
```

Encrypted backups: pass `--password` or set `ARCHIVE_PASSWORD` (never prompts).
The canonical, machine-readable contract lives in **[AGENTS.md](AGENTS.md)**.

Crates: `archive` (the CLI) over `archive-core` (the crabapple-backed
open/decrypt/fetch layer). Neither depends on the Messages tooling below.

### `imessage-exporter` — iMessage & SMS conversations

Exports message conversations (and attachments) to `txt` / `html` from a live
macOS `chat.db` or an iOS backup. Built on the `imessage-database` library.

```bash
imessage-exporter -p ~/Backup/<UDID> -f html
```

This component tracks **[ReagentX/imessage-exporter](https://github.com/ReagentX/imessage-exporter)**
upstream so it stays current with its active Messages development.

## Build

```bash
cargo build --release
# binaries: target/release/archive, target/release/imessage-exporter
```

## How the pieces fit

Both binaries read the **same** iOS backup directory but take different data
from it: `archive` for contacts/calls/voicemail, `imessage-exporter` for
conversations. They share the backup as a data source, not code.

## Provenance & license

The `imessage-database` and `imessage-exporter` crates originate from
[ReagentX/imessage-exporter](https://github.com/ReagentX/imessage-exporter) and
are licensed **GPL-3.0**. ARCHiVE is a derivative work and is therefore released
under **GPL-3.0-or-later** (see [LICENSE](LICENSE)). The `archive` and
`archive-core` crates are original to this project.

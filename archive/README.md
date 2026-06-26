# archive

Extract personal data from an on-disk iOS backup (encrypted or not) into
machine-readable and human-readable formats.

**Agents:** read [`../AGENTS.md`](../AGENTS.md) — every command emits one JSON
object on stdout with stable exit codes, progress on stderr.

## Quick start

```
# Discover what the backup contains (read-only)
archive --backup <backup-dir> inspect

# Export each data type to <out>/
archive --backup <backup-dir> -o <out> contacts  -f vcf    # csv | json | vcf | html
archive --backup <backup-dir> -o <out> calls     -f json   # csv | json | html
archive --backup <backup-dir> -o <out> voicemail -f json   # csv | json | html

# Extract voicemail metadata + audio (raw .amr; pass --audio-format m4a|wav to transcode via ffmpeg)
archive --backup <backup-dir> -o <out> voicemail -f json --audio
```

Encrypted backups: pass `--password` or set `ARCHIVE_PASSWORD` (never prompts).

## Status

- [x] inspect — store discovery (read-only)
- [x] contacts — csv, json, vcf, html (incl. postal addresses)
- [x] calls — csv, json, html
- [x] voicemail — csv, json, html (metadata) + audio extraction (`--audio`, raw `.amr` or ffmpeg `m4a`/`wav`)
- [ ] photos · notes · attachment audio · pdf output

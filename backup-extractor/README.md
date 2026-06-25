# backup-extractor

Extract personal data from an on-disk iOS backup (encrypted or not) into
machine-readable and human-readable formats.

**Agents:** read [`../AGENTS.md`](../AGENTS.md) — every command emits one JSON
object on stdout with stable exit codes.

## Quick start

```
backup-extractor --backup <backup-dir> inspect
backup-extractor --backup <backup-dir> -o <out> contacts -f vcf
```

## Status

- [x] inspect (store discovery)
- [x] contacts (csv, json, vcf, html)
- [ ] calls · photos · notes · pdf output

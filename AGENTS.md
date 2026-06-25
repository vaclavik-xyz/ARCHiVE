# Agent guide: backup-extractor

`backup-extractor` extracts personal data from an on-disk iOS backup. It is
built to be driven by agents: every command prints exactly one JSON object to
**stdout**, human progress goes to **stderr**, and exit codes are stable.

## Invocation

```
backup-extractor --backup <DIR> [--password <PW>] [-o <OUT>] <COMMAND> [ARGS]
```

- `--backup <DIR>` (required): the iOS backup directory. Must appear **before**
  the subcommand.
- `--password <PW>` (optional): encrypted-backup password. May also be supplied
  via the `BACKUP_EXTRACTOR_PASSWORD` environment variable. Not needed for
  unencrypted backups. Headless runs never prompt.
- `-o, --out <OUT>`: output directory. Required for export commands; ignored by
  `inspect`.

`--password` and `--out` are global flags — they may appear before **or** after
the subcommand name, which is agent-friendly for programmatic invocation.

## Commands

### `inspect` — discover what is extractable (read-only)

```
backup-extractor --backup <DIR> [--password <PW>] inspect
```

stdout:

```json
{
  "ok": true,
  "command": "inspect",
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." },
  "stores": [
    { "type": "contacts", "present": true, "supported": true, "count": 1234 },
    { "type": "calls", "present": true, "supported": false, "count": null },
    { "type": "photos", "present": true, "supported": false, "count": null },
    { "type": "notes", "present": false, "supported": false, "count": null }
  ]
}
```

`supported` = this build can export the type; `present` = the store exists in
this backup; `count` is best-effort: filled for supported + present stores, and
`null` otherwise (including the rare case where a present store cannot be read).

### `contacts` — export contacts

```
backup-extractor --backup <DIR> [--password <PW>] -o <OUT> contacts -f <FORMAT>
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

## Result envelope (every command)

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
malformed (a missing required flag such as `--backup`, or an unknown flag),
`clap` prints plain-text usage to **stderr**, writes **nothing** to stdout, and
also exits **2** — there is no JSON envelope. Disambiguate exit 2 by inspecting
stdout: a JSON object with `"kind": "auth"` is an authentication failure; empty
stdout (with usage text on stderr) is a malformed invocation. When unsure, run
`--help` or `inspect` first to learn the contract.

> Note: argument/usage errors (an unknown `-f` format, a missing `--out`) are detected **before** the backup is opened, so they surface as exit 1 even when the backup is an encrypted one that would otherwise report an auth error.

## Examples

```bash
# Discover
backup-extractor --backup ~/Backup/UDID inspect

# Export contacts as importable vCard
backup-extractor --backup ~/Backup/UDID -o /tmp/out contacts -f vcf

# Encrypted backup via env var (no prompt)
BACKUP_EXTRACTOR_PASSWORD=secret \
  backup-extractor --backup ~/Backup/UDID -o /tmp/out contacts -f json
```

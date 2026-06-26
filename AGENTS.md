# Agent guide: archive

`archive` extracts personal data from an on-disk iOS backup. It is
built to be driven by agents: every command prints exactly one JSON object to
**stdout**, human progress goes to **stderr**, and exit codes are stable.

## Invocation

```
archive --backup <DIR> [--password <PW>] [-o <OUT>] <COMMAND> [ARGS]
```

- `--backup <DIR>` (required): the iOS backup directory. Must appear **before**
  the subcommand.
- `--password <PW>` (optional): encrypted-backup password. May also be supplied
  via the `ARCHIVE_PASSWORD` environment variable. Not needed for
  unencrypted backups. Headless runs never prompt.
- `-o, --out <OUT>`: output directory. Required for export commands; ignored by
  `inspect`.

`--password` and `--out` are global flags — they may appear before **or** after
the subcommand name, which is agent-friendly for programmatic invocation.

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
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." },
  "stores": [
    { "type": "contacts", "present": true, "supported": true, "count": 1234 },
    { "type": "calls", "present": true, "supported": true, "count": 5678 },
    { "type": "voicemail", "present": true, "supported": true, "count": 42 },
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

### `voicemail` — export voicemail metadata

```
archive --backup <DIR> [--password <PW>] -o <OUT> voicemail -f <FORMAT>
```

`FORMAT` is one of `csv | json | html` (`vcf` is rejected with exit 1). Writes
`<OUT>/voicemail.<ext>`. Audio files (`.amr`) are not extracted. stdout envelope:

```json
{
  "ok": true,
  "command": "voicemail",
  "count": 42,
  "outputs": ["<OUT>/voicemail.json"],
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

Each voicemail object: `sender` (caller number; empty when withheld), `date`
(ISO 8601 UTC), `duration_seconds`, `trashed` (bool — moved to Deleted),
`trashed_at` (ISO 8601 UTC or `null`), `expiration` (ISO 8601 UTC or `null`),
`flags` (raw bitmask, **not decoded** — use `trashed` for deletion status). No
voicemail → `count: 0`, `outputs: []`, plus a `note`.

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
archive --backup ~/Backup/UDID inspect

# Export contacts as importable vCard
archive --backup ~/Backup/UDID -o /tmp/out contacts -f vcf

# Encrypted backup via env var (no prompt)
ARCHIVE_PASSWORD=secret \
  archive --backup ~/Backup/UDID -o /tmp/out contacts -f json
```

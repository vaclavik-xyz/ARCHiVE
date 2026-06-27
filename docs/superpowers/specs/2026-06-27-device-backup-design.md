# `backup` — Create a Backup from a Connected iPhone — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) — a process wrapper over `libimobiledevice`
(`idevicebackup2` + `idevice_id`). No new Rust dependency (external binaries,
like the optional `ffmpeg`).

## Goal

Add an `archive backup` command that creates a fresh iOS backup from a USB-connected
iPhone via `idevicebackup2`, removing the "you must already have a backup" friction
for a repair/recovery workflow: plug in the phone → one command → a backup the other
`archive` commands can read.

## Why a wrapper (grounded)

Reading iMessage/photos/etc. directly off a connected iPhone is not possible — the
data lives in the app sandbox under `/var/mobile/`, reachable only through the
`mobilebackup2` service, i.e. by performing a backup. `libimobiledevice`'s
`idevicebackup2` is the open-source client for that service. `archive` shells out
to it (it does not reimplement the protocol) and then reads the resulting backup
with the existing `archive-core`.

## CLI surface

```
archive backup -o <out> [--full]
```

- `-o <out>` (required): destination directory. `idevicebackup2` writes the backup
  to `<out>/<udid>/`.
- `--full`: force a full backup (`idevicebackup2 backup --full`); default is an
  incremental/normal backup (faster when `<out>` already holds a prior backup).
- `--backup <DIR>` is **not** used by this command (it has no input backup). The
  global flag becomes optional; read commands now validate its presence at runtime
  (a clear usage error when missing) instead of at parse time.
- Requires `idevicebackup2` and `idevice_id` on `PATH`. Missing → **fail fast**
  (exit 1) with an install hint (e.g. `brew install libimobiledevice`).
- No device connected (`idevice_id -l` empty) → exit 1 with a clear message.
- Encrypted-backup handling is the device's own setting; `archive backup` does not
  toggle it. (Reading an encrypted result later still uses `--password`.)

## Behavior

1. **Tool check:** `tool_available("idevicebackup2")` and
   `tool_available("idevice_id")` (run `<tool> --version`/`-h`; success = present).
   Either missing → fail fast with the install hint.
2. **Device check:** run `idevice_id -l`, parse stdout into a list of UDIDs (one per
   non-empty line). Empty → "no iOS device connected" error.
3. **Run backup:** spawn `idevicebackup2 backup [--full] <out>` inheriting
   stdout/stderr so the user sees live progress; wait for exit. Non-zero exit →
   error carrying the status.
4. **Report:** the backup is at `<out>/<udid>/`. Open it with
   `archive_core::Backup::open` (no password — fresh backups are readable; an
   encrypted one still opens enough for device info or surfaces an auth note) to
   read device info; emit the success envelope.

## Output envelope

```json
{
  "ok": true,
  "command": "backup",
  "dir": "<out>/<udid>",
  "udid": "<udid>",
  "device": { "name": "...", "model": "iPhone14,2", "ios": "17.5", "serial": "...", "udid": "..." }
}
```

If multiple devices are connected, the first UDID from `idevice_id -l` is used and a
note records the others (selecting a specific device is out of scope). When the
result cannot be opened for device info (e.g. encrypted), `device` is omitted and a
`note` explains, but `ok` stays true (the backup itself succeeded).

## Module (`device_backup.rs`)

Pure, unit-testable helpers (the actual spawn is integration territory, gated):

```rust
pub fn backup_args(udid: &str, out: &Path, full: bool) -> Vec<OsString>; // ["--udid", udid, "backup", ("--full"?), out]
pub fn parse_udids(idevice_id_stdout: &str) -> Vec<String>;  // non-empty trimmed lines
pub fn tool_available(tool: &str) -> bool;                   // mirrors audio::ffmpeg_available
pub const BACKUP_TOOL: &str = "idevicebackup2";
pub const DEVICE_TOOL: &str = "idevice_id";
```

`run_backup` (in main.rs) orchestrates: tool check → device check → spawn → open →
envelope. The spawn/`idevice_id` execution is gated behind an env opt-in
(`ARCHIVE_TEST_DEVICE`) so CI never requires hardware.

## Global-flag change

`Cli.backup` becomes `Option<PathBuf>`. A single `open_backup(cli, password)` helper
replaces the 12 identical `Backup::open(&cli.backup, password).map_err(open_error)?`
call sites: it resolves `--backup` (usage error when absent) and opens. This both
enables the no-input `backup` command and DRYs the open logic.

## Error handling

| Situation | Behavior |
|---|---|
| `idevicebackup2`/`idevice_id` missing | fail fast, exit 1, install hint |
| No device connected | exit 1, clear message |
| `idevicebackup2` non-zero exit | exit 1, status in message |
| Result not openable (encrypted) | `ok: true`, `device` omitted, `note` |
| read command without `--backup` | usage error, exit 1 |

## Testing

- `backup_args`: `["backup", "<out>"]` and, with `--full`, `["backup", "--full", "<out>"]`.
- `parse_udids`: multi-line stdout → trimmed non-empty UDIDs; blank/whitespace input → empty.
- `tool_available`: returns false for a clearly absent binary name (deterministic).
- `open_backup`: `None` backup → usage error (exit 1); a value passes through to open.
- CLI: `backup` parses with/without `--full`; read commands still parse; the
  `--backup`-missing case is now a runtime usage error (test via `open_backup`).
- Gated end-to-end (`ARCHIVE_TEST_DEVICE`): runs a real backup; skipped otherwise.

## Out of scope (YAGNI)

- Selecting among multiple devices, Wi-Fi sync, pairing/trust prompts (the user
  trusts the device on the phone as usual).
- Toggling backup encryption, restore, app management.
- Auto-chaining into `recover` (run `archive --backup <out>/<udid> recover` after).
- Bundling/installing libimobiledevice.

## Global constraints (carried from the project)

- Agent-first contract: one JSON object on stdout; progress on stderr; exit
  `0`/`1`/`2`; clap errors the only non-JSON exit-2 case.
- No new Rust dependency; `idevicebackup2`/`idevice_id` are optional external tools
  required only by this command.
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

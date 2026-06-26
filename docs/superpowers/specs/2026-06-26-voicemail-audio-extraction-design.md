# Voicemail Audio Extraction — Design

**Status:** Approved (2026-06-26)
**Component:** `archive` (CLI) + `archive-core` (backup access, already sufficient)
**Depends on:** existing voicemail metadata extractor (increment 2)

## Goal

Extend the `archive voicemail` command so it can optionally extract the actual
voicemail audio files from an iOS backup (not just metadata), link each audio
file from its metadata record, and optionally transcode the native `.amr` to a
more widely playable format. This is the first binary-file extraction in
`archive` (every prior extractor produced text from SQLite only).

## Background (grounded)

- iOS visual voicemail stores metadata in `HomeDomain` →
  `Library/Voicemail/voicemail.db` (already read by `voicemail::parse`).
- Each `voicemail` row's audio is a separate file in the **same domain**:
  `HomeDomain` → `Library/Voicemail/<ROWID>.amr`, named by the row's `ROWID`.
- `archive_core::Backup::fetch(domain, relative_path, dest)` already
  materializes any backup file (handling encrypted and unencrypted backups) and
  returns `Ok(None)` when the file is absent. No change to `archive-core` is
  required — the mapping `ROWID → <ROWID>.amr` is all we need.
- The current `Voicemail` struct does **not** carry `ROWID`, so the parser must
  start selecting it to build the audio path.

## CLI surface

```
archive --backup <dir> -o <out> voicemail -f <csv|json|html> \
        [--audio] [--audio-format <amr|m4a|wav>]
```

- `--audio` (flag, default off): enable audio extraction. With it off, behavior
  is exactly as today (metadata only).
- `--audio-format <amr|m4a|wav>` (default `amr`):
  - `amr` — raw copy of the backup file, **no external dependency**.
  - `m4a` / `wav` — transcode the fetched `.amr` via `ffmpeg`.
- `--audio-format` without `--audio` is a usage error (exit 1):
  `"--audio-format requires --audio"`.
- `--audio` still requires `--out` (already required for `voicemail` export).
- Audio extraction is orthogonal to the metadata format: `--audio` works with
  `-f csv`, `-f json`, and `-f html` alike.

## Data model

`voicemail::Voicemail` gains two fields:

- `rowid: i64` — the row's primary key (also a stable per-backup identifier;
  serialized in output).
- `audio_file: Option<String>` — output-relative path to the extracted audio
  (e.g. `voicemail_audio/2020-09-13_122640_+420776452878_3.m4a`). `None` when
  audio extraction did not run, or when the backup has no audio for that row.

`parse(db_path)` adds `ROWID` to its `SELECT` and populates `rowid`;
`audio_file` is always `None` from the parser (it is filled in later by the
extraction step, only when `--audio` is set).

## Audio extraction (`archive/src/voicemail_audio.rs`, new module)

Entry point (shape):

```rust
/// Outcome counts for the run, surfaced in the JSON envelope.
pub struct AudioSummary { pub format: AudioFormat, pub dir: String,
                          pub extracted: usize, pub missing: usize }

pub fn extract_audio(
    backup: &archive_core::Backup,
    items: &mut [Voicemail],     // audio_file is filled in place
    out: &Path,
    format: AudioFormat,
) -> Result<AudioSummary, AppError>;
```

Per voicemail row:

1. Build the source path `Library/Voicemail/<rowid>.amr` in `HomeDomain`.
2. For `amr`: `backup.fetch(..)` directly into
   `<out>/voicemail_audio/<name>.amr`.
   For `m4a`/`wav`: `fetch(..)` into a scratch temp `.amr`, then `ffmpeg`
   transcode into `<out>/voicemail_audio/<name>.<ext>`; the temp `.amr` is
   discarded (auto-cleaned `TempDir`).
3. On success, set `item.audio_file = Some("voicemail_audio/<name>.<ext>")` and
   bump `extracted`.
4. `fetch` returns `None` (no audio for this row): leave `audio_file = None`,
   bump `missing`, continue.

### File naming

Readable and collision-free:
`<date>_<sender>_<rowid>.<ext>`, where:

- `date` = the record's timestamp as `YYYY-MM-DD_HHMMSS` (derived from the
  already-parsed ISO date; `unknown` when the date is empty/unconvertible).
- `sender` = the caller number, sanitized to `[A-Za-z0-9+]`, other chars → `_`;
  `unknown` when withheld/empty.
- `rowid` guarantees uniqueness even if date+sender collide.
- `ext` = `amr` | `m4a` | `wav`.

The `voicemail_audio/` subdirectory does not clash with the `voicemail.<ext>`
metadata file written into `<out>/`.

### Transcoding (ffmpeg)

- Discovery: probe `ffmpeg` once (run `ffmpeg -version`, success = available).
- If a non-`amr` format is requested and ffmpeg is **not** available: **fail
  fast** before any extraction — exit 1, message naming ffmpeg and the
  requested format. (We never silently hand back `.amr` when the user asked for
  `.m4a`.)
- Commands (run quietly; stdin/stdout/stderr null; check exit status):
  - m4a: `ffmpeg -y -i <in.amr> -c:a aac <out.m4a>`
  - wav: `ffmpeg -y -i <in.amr> <out.wav>`
- Per-file transcode failure is best-effort: keep the raw `.amr` (written to the
  audio dir as `<name>.amr`), point `audio_file` at it, log one line to stderr,
  bump `extracted`, continue.
- `archive` performs ffmpeg discovery/command building itself; it does **not**
  depend on `imessage-exporter` internals (the crates stay independent).

## Error handling summary

| Situation | Behavior |
|---|---|
| Audio file absent for a row | `audio_file: null`, count `missing`, continue |
| Transcode of one file fails | keep raw `.amr`, link it, log stderr, continue |
| ffmpeg missing, format ≠ amr | fail fast, exit 1, clear message (before work) |
| `--audio-format` without `--audio` | usage error, exit 1 |
| Backup has no voicemail store | unchanged: `count: 0` note, no audio dir |

## Output envelope

The success envelope gains an `audio` object when `--audio` ran:

```json
{
  "ok": true, "command": "voicemail", "count": 12,
  "outputs": ["out/voicemail.json"],
  "audio": { "format": "m4a", "dir": "voicemail_audio",
             "extracted": 10, "missing": 2 },
  "device": { ... }
}
```

`outputs` continues to list the metadata file. When `--audio` is not set, the
envelope is unchanged (no `audio` key). `extracted` counts audio files written
to the audio dir (including raw `.amr` files kept as a transcode fallback);
`missing` counts rows with no audio present in the backup.

## Formatters

`audio_file` is rendered in every metadata format:

- **CSV**: new `audio_file` column (empty cell when `None`).
- **JSON**: the `audio_file` field (null when `None`); `rowid` also serialized.
- **HTML**: when present, an `<audio controls src="...">` player; the `src` is
  HTML-attribute-escaped by askama, and the filename is already restricted to a
  safe character set, so it is injection-safe. When `None`, render nothing (or a
  muted dash).

## Testing strategy

- Parser: fixture with explicit `ROWID`s → `rowid` populated correctly;
  `audio_file` is `None` from the parser.
- Naming: unit tests for `<date>_<sender>_<rowid>.<ext>` including sanitization,
  empty sender → `unknown`, empty date → `unknown`, and uniqueness via rowid.
- ffmpeg command builder: unit-test the argv produced for m4a and wav **without
  running ffmpeg**; unit-test the fail-fast path when ffmpeg is reported absent.
- Best-effort: a row whose audio is absent yields `audio_file: None` and is
  counted as `missing` (drive via a fake/empty backup or a stubbed fetch).
- Formatters: csv has the `audio_file` column; json has the field + `rowid`;
  html emits an `<audio>` player and is XSS-safe for a crafted filename.
- Real fetch: a gated integration test behind `ARCHIVE_TEST_BACKUP` that pulls a
  real `.amr` and asserts it is non-empty (skips when the env var is unset).

## Out of scope (YAGNI)

- Bulk/parallel transcoding (voicemails are few; sequential is fine).
- Transcoding to formats other than m4a/wav.
- Audio for any other data type (messages/attachments) — separate feature.
- Changing `archive-core` (its `fetch`/`has` already suffice).

## Global constraints (carried from the project)

- **Agent-first contract:** exactly one JSON object on stdout; human progress on
  stderr; exit `0` success / `1` handled error / `2` usage; clap argument errors
  are the only non-JSON exit-2 case.
- **Password:** `--password` or `ARCHIVE_PASSWORD`; never prompts.
- **No new mandatory dependency:** ffmpeg is required **only** when a non-amr
  `--audio-format` is requested; the default (`amr`) and all metadata paths stay
  dependency-free.
- **Crate independence:** `archive` does not depend on the `imessage-*` crates.
- **Licensing/process:** GPL-3.0-or-later; conventional commits; never squash.

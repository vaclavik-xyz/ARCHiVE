# Voice Memos Extraction — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) + `archive-core` (one new directory-listing primitive)
**Sibling of:** voicemail audio extraction (this reuses its audio/ffmpeg patterns)

## Goal

Add an `archive voice-memos` command that extracts the Voice Memos app's
recordings from an iOS backup: the audio files **and** their metadata (title,
date, duration). Voice Memos are the warm-up of the larger "extract everything"
program — they are audio-centric like voicemail but stored natively as `.m4a`
(no transcoding needed by default), and they introduce one reusable building
block: listing the files under a directory in a backup domain.

## Background (grounded)

iOS stores Voice Memos in an app *group container*. In a backup that surfaces as:

- **Domain:** `AppDomainGroup-group.com.apple.VoiceMemos`
- **Metadata DB:** `Recordings/CloudRecordings.db` (Core Data; table
  `ZCLOUDRECORDING`).
- **Audio files:** `Recordings/<name>.m4a` (occasionally `.caf`, `.wav`,
  `.aifc`). `<name>` comes from the DB column `ZPATH` and is usually a UUID or a
  date-stamped name.

**Legacy fallback (iOS ≤ 11):** the same data lived under `MediaDomain` with
`Recordings/Recordings.db`. The extractor tries the group domain first, then
falls back to `MediaDomain`. Both use a `Recordings/` prefix.

`ZCLOUDRECORDING` columns we read (all best-effort, schema-tolerant via the
existing `table_columns` helper — column sets vary by iOS version):

| field | column | epoch / notes |
|---|---|---|
| title | `ZCUSTOMLABEL` (fallback `ZENCRYPTEDTITLE`) | user-visible name; may be empty |
| date | `ZDATE` | Cocoa/Core-Data 2001 epoch → ISO 8601 UTC |
| duration | `ZDURATION` | seconds (REAL → rounded to i64) |
| path | `ZPATH` | filename under `Recordings/`; joins audio ↔ metadata |

**Why a new primitive:** every existing extractor (`contacts`, `calls`,
`voicemail`) fetches one file at a *deterministic* path. Voice Memos audio
filenames are data, not derivable from a row id, so we must enumerate the
`Recordings/` directory. `archive_core::Backup` currently exposes only
`has`/`fetch` (exact path). We add `list(domain, prefix)`. This same primitive
is needed by later program features (Photos, message attachments), so it belongs
in `archive-core`, not in one command.

## archive-core change

```rust
/// Relative paths of every backup entry in `domain` whose `relative_path`
/// starts with `prefix`. Empty `prefix` lists the whole domain. Read-only;
/// does not decrypt anything (operates on the manifest entry list).
pub fn list(&self, domain: &str, prefix: &str) -> Result<Vec<String>, BackupError>;
```

- Implementation: filter `self.raw.entries()` by `domain` and
  `relative_path.starts_with(prefix)`, collect `relative_path` strings.
- Order is unspecified by the manifest; `list` returns them **sorted** so output
  is deterministic.
- Directory entries (zero-length, no real file) are not filtered out here — the
  caller decides what to fetch; in practice fetching a directory entry yields
  empty bytes, which the extractor treats as a 0-byte file and still counts. To
  avoid that, the extractor filters to known audio extensions (below).

## CLI surface

```
archive --backup <dir> -o <out> voice-memos -f <csv|json|html> \
        [--no-audio] [--audio-format <m4a|wav>]
```

- **Audio is extracted by default** (unlike `voicemail`, where it is opt-in).
  Rationale: a voice memo *is* its audio; metadata alone is rarely the goal, and
  the native `.m4a` copy needs no external dependency. The divergence is
  documented in AGENTS.md.
- `--no-audio`: metadata-only listing (no files written to `voice_memos/`).
- `--audio-format`:
  - default (omitted) — **raw copy**, preserving each file's native extension
    (`.m4a`/`.caf`/…). No ffmpeg.
  - `m4a` / `wav` — transcode every recording to that format via `ffmpeg`
    (reusing the voicemail transcode helpers). Transcoding a `.caf` source to a
    portable container is the real use case.
- `--audio-format` together with `--no-audio` is a usage error (exit 1):
  `"--audio-format conflicts with --no-audio"`.
- A non-raw `--audio-format` when ffmpeg is **not** on PATH: **fail fast** before
  any extraction (exit 1), message naming ffmpeg and the format — same contract
  as voicemail.
- `-f` accepts `csv | json | html` (`vcf` rejected, exit 1). `--out` required.

## Data model

`voice_memos::VoiceMemo`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceMemo {
    pub title: String,            // ZCUSTOMLABEL; "" when unnamed
    pub date: String,             // ISO 8601 UTC (Cocoa epoch); "" if unconvertible
    pub duration_seconds: i64,    // rounded ZDURATION
    pub source_file: String,      // ZPATH basename, e.g. "A1B2.m4a"; "" if unknown
    pub audio_file: Option<String>, // output-relative path once extracted, else None
}
```

`parse(db_path)` reads `ZCLOUDRECORDING`, ordered by `ZDATE`, and returns the
records with `audio_file = None`. Recordings present on disk but absent from the
DB are still extracted (see below) and surface as synthetic records with an
empty title and the file's own name.

## Extraction (`voice_memos.rs`)

```rust
pub struct VmSummary { pub dir: String, pub extracted: usize, pub missing: usize }

pub fn extract_voice_memos(
    backup: &archive_core::Backup,
    items: &mut Vec<VoiceMemo>,
    out: &Path,
    format: Option<AudioFormat>, // None = raw copy; Some = transcode via ffmpeg
) -> std::io::Result<VmSummary>;
```

Algorithm:

1. `list(domain, "Recordings/")`, keep entries whose extension is in
   `{m4a, caf, wav, aifc, aiff, mp4, m4r}` (case-insensitive). This is the set of
   audio files to recover.
2. Build a map `basename → &mut VoiceMemo` from `items` (via `source_file`).
3. For each listed audio path:
   - Find its metadata record by basename, or **synthesize** one (empty title,
     empty date, `source_file = basename`) and push it — so disk files with no DB
     row are still recovered and listed.
   - Output filename: `<date>_<title>_<n>.<ext>` where `date` is the compacted
     ISO (`YYYY-MM-DD_HHMMSS` or `unknown`), `title` is sanitized to
     `[A-Za-z0-9+]` (`unknown` when empty), and `n` is a 1-based running index
     guaranteeing uniqueness (Voice Memos have no stable small integer id like
     voicemail's rowid; the index plays that role).
   - Raw mode: `fetch` straight into `<out>/voice_memos/<name><nativeExt>`.
   - Transcode mode: `fetch` to a scratch temp, then ffmpeg into
     `<out>/voice_memos/<name>.<fmtExt>`; on transcode failure keep the raw file
     (best-effort, exactly like voicemail).
   - Link `audio_file = Some("voice_memos/<name>")`, bump `extracted`.
4. A DB record whose `source_file` was **not** found on disk: leave
   `audio_file = None`, bump `missing`.
5. Return `VmSummary { dir: "voice_memos", extracted, missing }`.

`--no-audio`: skip the whole extraction; `items` keep `audio_file = None`; no
`voice_memos/` directory is created; no `audio` envelope key.

The `AudioFormat` enum (`m4a`/`wav` only — `amr` stays meaningful only for
voicemail, but the shared enum already carries it and is harmless here),
`compact_date`, the `sanitize`-style filename helper, `ffmpeg_available`, the
transcode argv builder, and the temp-dir/best-effort flow are **reused from the
voicemail audio module**. The implementer promotes the genuinely shared helpers
(format enum, ffmpeg discovery, transcode argv, `compact_date`, sanitizer) into a
small shared `audio` module (e.g. `src/audio.rs`) that both `voicemail_audio` and
`voice_memos` use, rather than duplicating the logic. Voice Memos accepts only
`--audio-format m4a|wav`; passing `amr` is a usage error for this command.

## Output envelope

```json
{
  "ok": true, "command": "voice-memos", "count": 8,
  "outputs": ["<out>/voice-memos.json"],
  "audio": { "format": "m4a", "dir": "voice_memos", "extracted": 8, "missing": 0 },
  "device": { "name": "iPhone", "ios": "17.5", "udid": "00008..." }
}
```

- `outputs` lists the metadata file (`voice-memos.<ext>` in `<out>`).
- `audio` present only when audio extraction ran (i.e. not `--no-audio`).
  `format` is the produced format (`m4a`/`wav`, or the literal `raw` when no
  transcode — i.e. native copies). `dir` is `voice_memos`.
- No Voice Memos store at all → `count: 0`, `outputs: []`, `note`, no `audio`.

## Formatters

- **CSV** columns: `title, date, duration_seconds, source_file, audio_file`.
- **JSON**: serde derive (all fields).
- **HTML** (`archive/templates/voice-memos.html`): a table with an
  `<audio controls src="…">` player per row when `audio_file` is present;
  askama escapes the `src`, and filenames are restricted to a safe set, so it is
  injection-safe (mirrors `voicemail.html`). Empty audio cell when `None`.

## inspect

Add to `KNOWN_STORES`:
`("voice-memos", true, "AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db")`.
`count` for inspect = number of parsed `ZCLOUDRECORDING` rows (best-effort, like
the others). The legacy `MediaDomain` location is not probed by inspect (kept
simple; the extractor still falls back at export time).

## Error handling summary

| Situation | Behavior |
|---|---|
| No Voice Memos store (neither domain) | `count: 0`, `note`, no audio dir |
| DB present, audio file missing on disk | record kept, `audio_file: null`, `missing++` |
| Disk audio file with no DB row | synthesized record, extracted, `extracted++` |
| Transcode of one file fails | keep raw native file, link it, stderr log, `extracted++` |
| ffmpeg missing, `--audio-format` non-raw | fail fast, exit 1, clear message |
| `--audio-format` with `--no-audio` | usage error, exit 1 |
| `-f vcf` | usage error, exit 1 |

## Testing strategy

- `archive-core::list`: gated integration test against `ARCHIVE_TEST_BACKUP`
  (lists `Recordings/` and asserts every result starts with the prefix and is in
  the domain); a pure unit test is not possible without a manifest, matching the
  existing `fetch` test convention.
- Parser: fixture `make_voicememos(db)` building a minimal `ZCLOUDRECORDING`
  (two rows, one with empty title) → asserts title/date(Cocoa)/duration/
  source_file and `audio_file == None`.
- Filename/format helpers: reuse/extend the voicemail unit tests (compacted date,
  sanitization, uniqueness via running index, transcode argv).
- Formatters: csv header+row, json roundtrip, html emits an `<audio>` player and
  is XSS-safe for a crafted filename.
- CLI: `voice-memos` parses; `--audio-format` + `--no-audio` is a usage error;
  format resolution fails fast without ffmpeg for non-raw.
- Gated end-to-end: against a real backup, `extracted + missing == items.len()`
  and every linked file exists and is non-empty.

## Out of scope (YAGNI)

- Decoding `ZENCRYPTEDTITLE` when it is actually encrypted (rare; we read the
  plaintext title columns and fall back to the filename).
- Folders/Smart-folders organization, favorites, trashed memos as a separate
  view (a `trashed`/folder column may be surfaced later if a real backup shows
  it; not designed in now).
- Parallel transcoding (recordings are few).
- Any data type other than Voice Memos.

## Global constraints (carried from the project)

- **Agent-first contract:** exactly one JSON object on stdout; human progress on
  stderr; exit `0` success / `1` handled (usage/other) / `2` auth; clap argument
  errors are the only non-JSON exit-2 case.
- **Password:** `--password` or `ARCHIVE_PASSWORD`; never prompts.
- **No new mandatory dependency:** ffmpeg required **only** for a non-raw
  `--audio-format`; the default (raw native copy) and all metadata paths stay
  dependency-free.
- **Crate independence:** `archive` does not depend on the `imessage-*` crates.
- **Licensing/process:** GPL-3.0-or-later; conventional commits; never squash.
```

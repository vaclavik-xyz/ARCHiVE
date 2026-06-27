# Photos & Videos Extraction — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive` (CLI) — DB metadata + binary file extraction (reuses the
`Backup::list`/`fetch` + filename helpers from the voice-memos work)

## Goal

Add an `archive photos` command that recovers the Camera Roll from an iOS backup:
each asset's metadata (filename, kind, date, dimensions, GPS, favorite/trashed
flags, video duration) **and** the actual photo/video files, with an HTML gallery.
This is the single highest-value recovery feature for a service/repair context.

## Source (grounded)

`CameraRollDomain`:

- **Metadata DB:** `Media/PhotoData/Photos.sqlite`, table `ZASSET` (modern Photos,
  iOS 10+). Columns we read (schema-tolerant via `table_columns` — `Z*` suffixes
  drift by version):
  | field | column | notes |
  |---|---|---|
  | filename | `ZFILENAME` | e.g. `IMG_0001.HEIC` |
  | directory | `ZDIRECTORY` | e.g. `DCIM/100APPLE` (relative to `Media/`) |
  | created | `ZDATECREATED` | Cocoa/2001 epoch → ISO UTC |
  | kind | `ZKIND` | 0 = image, 1 = video |
  | favorite | `ZFAVORITE` | int bool |
  | trashed | `ZTRASHEDSTATE` | int (1 = in Recently Deleted) |
  | width/height | `ZWIDTH` / `ZHEIGHT` | pixels |
  | latitude/longitude | `ZLATITUDE` / `ZLONGITUDE` | `-180.0` = no location |
  | duration | `ZDURATION` | video length seconds |
- **Files:** the asset's backup path is `Media/<ZDIRECTORY>/<ZFILENAME>` in
  `CameraRollDomain`.

Extraction is **DB-driven** (the camera-roll DB is authoritative); orphan files
on disk with no `ZASSET` row are not synthesized (unlike voice memos). Documented.

## CLI surface

```
archive --backup <dir> -o <out> photos -f <csv|json|html> [--no-files]
```

- **Files are extracted by default** into `<out>/photos/` (the photos *are* the
  point of recovery), consistent with `voice-memos`. The camera roll can be large
  (gigabytes); this is documented and a one-line progress note goes to stderr.
- `--no-files`: metadata-only catalog (no files written, no `photos/` dir).
- `-f` accepts `csv | json | html` (`vcf` rejected). Writes `<out>/photos.<ext>`.
- `--out` required. Store absent → `count: 0`, `outputs: []`, `note`, exit 0.

No transcoding/thumbnailing (no ffmpeg/image deps): files are copied byte-for-byte
from the backup. The HTML gallery references the extracted files directly.

**Large-file handling:** `Backup::fetch` stream-copies unencrypted entries
straight to disk (no full-file buffer), so large videos do not spike memory.
Encrypted backups are the exception — crabapple's `decrypt_entry` returns the
plaintext in full, so an encrypted asset is buffered in memory while written
(a documented limitation bounded by the largest single asset).

## Data model

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Photo {
    pub filename: String,            // ZFILENAME
    pub kind: String,                // "image" | "video" | "unknown"
    pub created: String,             // ISO 8601 UTC (Cocoa); empty if unconvertible
    pub favorite: bool,
    pub trashed: bool,               // in Recently Deleted
    pub width: i64,
    pub height: i64,
    pub latitude: Option<f64>,       // None when unset (sentinel -180 / out of range)
    pub longitude: Option<f64>,
    pub duration_seconds: Option<i64>, // videos only
    pub source_path: String,         // backup-relative source (Media/<dir>/<file>)
    pub file: Option<String>,        // output-relative extracted path, or None
}
```

GPS sentinel handling: a coordinate is `Some` only when latitude ∈ [-90, 90],
longitude ∈ [-180, 180], and not the Apple "no fix" sentinel `-180.0`.

## Extraction (`photos.rs`)

```rust
pub struct PhotoSummary { pub dir: String, pub extracted: usize, pub missing: usize }

pub fn extract_photos(
    backup: &archive_core::Backup,
    items: &mut [Photo],
    out: &Path,
) -> std::io::Result<PhotoSummary>;
```

Per asset (mirrors `voice_memos::extract_from`, minus transcoding/synthesis):

1. Source = `Media/<directory>/<filename>` (skip when filename empty).
2. Output name = `<n>_<basename(filename)>` (1-based index `n` guarantees
   uniqueness across directories; original name preserved for recognizability).
3. `fetch(CameraRollDomain, source, <out>/photos/<name>)`; on success set
   `file = Some("photos/<name>")`, else leave `None`.
4. `extracted` = assets with a `file`; `missing` = assets with none
   (`extracted + missing == items.len()` by construction). Dir created only when
   extracting; `--no-files` skips the whole step.

## Output envelope

```json
{
  "ok": true, "command": "photos", "count": 1240,
  "outputs": ["<out>/photos.json"],
  "files": { "dir": "photos", "extracted": 1238, "missing": 2 },
  "device": { ... }
}
```

`files` present only when extraction ran (not `--no-files`). Store absent →
`count: 0`, `outputs: []`, `note`, no `files`.

## Formatters

- **CSV** columns: `filename, kind, created, favorite, trashed, width, height,
  latitude, longitude, duration_seconds, file`.
- **JSON**: serde derive.
- **HTML** (`archive/templates/photos.html`): a gallery — each image as a
  lazy-loaded `<img>` (CSS-capped width) linking to the full file; each video as a
  link with a ▶ marker; caption shows date and, when present, GPS. askama escapes
  the `src`/filename (safe set). **Caveat (documented):** HEIC/HEVC may not render
  inline in every browser; the extracted file is always present regardless.

## inspect

Flip the existing placeholder `photos` entry to supported (it already points at
`CameraRollDomain` / `Media/PhotoData/Photos.sqlite`); `count` = parsed `ZASSET`
rows (best-effort).

## Error handling

| Situation | Behavior |
|---|---|
| No Photos.sqlite | `count: 0`, `outputs: []`, `note`, exit 0 |
| Asset file absent in backup | `file: null`, `missing++`, continue |
| Present but unreadable DB | exit 1, `kind:"other"` |
| `-f vcf` / unknown | usage error, exit 1 |

## Testing

- Parser: fixture `make_photos` (a photo with GPS + favorite, a video with
  duration, a trashed asset, an asset with the `-180` GPS sentinel) → asserts
  kind/date(Cocoa)/flags/dimensions/GPS Some-vs-None/duration/source_path and
  `file == None` from the parser.
- GPS sentinel: the `-180`/out-of-range asset yields `latitude/longitude == None`.
- Extraction: gated end-to-end test (real backup) asserting
  `extracted + missing == count` and every linked file exists non-empty; plus a
  pure unit test of the output-name builder.
- Formatters: csv header+row, json roundtrip, html emits `<img>`/video link and
  escapes a crafted filename.
- CLI: `photos` parses with/without `--no-files`; `export_format` rejects vcf;
  inspect lists photos supported.

## Out of scope (YAGNI)

- Thumbnail/preview generation, HEIC→JPEG transcoding (no image/ffmpeg deps).
- Albums, faces, memories, edits/adjustments, burst grouping.
- Orphan-file synthesis (DB is authoritative for the camera roll).
- iCloud-only assets with no local file (they simply count as `missing`).

## Global constraints (carried from the project)

- Agent-first contract: one JSON object on stdout; progress on stderr; exit
  `0`/`1`/`2`; clap errors the only non-JSON exit-2 case.
- Password: `--password` or `ARCHIVE_PASSWORD`; never prompts.
- No new dependency (rusqlite + the existing `Backup` API).
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```

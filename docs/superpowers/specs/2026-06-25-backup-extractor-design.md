# Backup Extractor — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) — ready for implementation plan

## Goal

A Rust command-line tool that extracts personal data from on-disk iOS backups
(Finder / Apple Devices / iTunes), encrypted or not, into both machine-readable
and human-readable formats. It is the foundation for paid, client-facing
data-extraction deliverables (the same use case that drove the iMessage PDF
export), broadened beyond messages.

## Scope (v1)

Four data types, each a self-contained extractor:

- **Contacts**
- **Call history**
- **Photos / videos**
- **Notes**

Messages already ship in `imessage-exporter` and stay there for v1; a unified
`messages` subcommand is a later step (see Out of Scope).

**Source:** on-disk iOS backups only (encrypted and unencrypted).

**Output:** every type supports a *structured* format (CSV / JSON, plus vCard for
contacts) and a *human-readable* format (HTML, and PDF via the WebKit/Quartz
engine). The format is chosen at export time with `-f`, exactly like
`imessage-exporter`.

## Out of scope (v1)

- Live USB device acquisition (lockdown / AFC) — an order of magnitude more work.
- Reading the same types from a live macOS install (different schemas/source).
- Unifying messages into this tool (kept in `imessage-exporter` for now).
- Notes attachments, drawings, tables, scanned docs — plaintext only in v1.
- Live Photo pairing / edit-render selection beyond copying the asset files.
- iCloud backups (only local backups).

## Agent-friendliness (first-class requirement)

The tool must be drivable by autonomous agents without guesswork — this is a
primary design goal, not a nicety:

- **Machine-readable outcome.** Every command prints exactly one JSON object to
  **stdout** describing the result: on success `{ "ok": true, "command": "...",
  "count": N, "outputs": ["<path>", …], "device": {…} }`; on failure
  `{ "ok": false, "error": "<message>", "kind": "<machine-stable-kind>" }`.
  Extracted data and reports go to files under `--out`; human-oriented progress
  goes to **stderr**. An agent never has to scrape prose.
- **Discovery before extraction.** An `inspect` command reports, as JSON, the
  device info plus which stores are present and their record counts, so an agent
  can decide what to extract before running an export.
- **Never block on a prompt.** The encrypted-backup password comes from
  `--password` or the `BACKUP_EXTRACTOR_PASSWORD` environment variable; an
  interactive prompt is used *only* when stdout/stderr is a TTY and neither is
  set. Headless/agent runs get a clean `kind: "auth"` error instead of hanging.
- **Stable exit codes.** `0` success (including "store absent — nothing to
  export"); `2` backup locked / wrong or missing password; `1` usage or other
  errors. Codes are documented and will not change meaning.
- **Self-describing + a written contract.** `clap` provides `--help` for every
  command; a top-level `AGENTS.md` is the canonical reference an agent reads
  first: every command, flag, the JSON result schema, exit codes, and copy-paste
  examples. All public `backup-core` items carry rustdoc.
- **Deterministic.** `--out` is required; output filenames are fixed per type and
  format (e.g. `contacts.vcf`); no hidden state or current-directory surprises.

## Prior art (why build anyway)

Researched 2026-06-25. Extracting these artifacts from iOS backups is solved
many times over, but not in the niche this targets:

- **OpenExtract** (MIT, active) — does almost exactly this feature set
  (messages, photos, voicemail, calls, contacts, notes → PDF/HTML/CSV/text,
  encrypted backups) but is an **Electron + React + Python** desktop GUI, not a
  Rust CLI.
- **iLEAPP** (mature, 1.1k★, Python) — broad forensic parser → HTML/TSV reports.
  Forensic-report oriented, not client deliverables, and not a clean
  export-one-type CLI. **Its open-source parsers are the canonical reference for
  the SQL queries and per-iOS-version schema handling and will be used as such.**
- **ibackuptool2** (Rust, 33★) — file extraction only (`ls`/`extract` by
  domain/path), no per-type parsing, marked "not suitable for normal use yet".
- **crabapple** (ReagentX, Rust) — the backup read/decrypt library this builds
  on; explicitly "a foundation for any project that needs iOS backup data".

**Gap filled:** no maintained Rust CLI parses these types into reports. Building
gives full control, reuse of the existing WebKit/Quartz PDF report pipeline, and
integration with the existing toolchain — not novelty.

## Architecture

Three layers; two new crates added to the workspace
(`members = ["imessage-database", "imessage-exporter"]` →
`+ "backup-core", "backup-extractor"`).

```
backup-core/         (lib)  open + decrypt a backup, fetch a store by
                            domain+relativePath, copy media. Only place that
                            knows the backup format and encryption.
backup-extractor/    (bin)  CLI; per-type extractors (parser -> model) and
                            format-agnostic formatters.
```

1. **`backup-core`** wraps `crabapple`. It is the single owner of backup access
   and decryption. Everything above it works with ordinary files and SQLite.
2. **Extractors** (`contacts`, `calls`, `photos`, `notes`) each locate their
   store via `backup-core`, parse it, and produce a plain Rust model. An
   extractor knows nothing about output formats.
3. **Formatters** turn a model into bytes for a chosen format. CSV/JSON are
   generic over any `Serialize` model; HTML/PDF use per-type templates; vCard is
   contacts-specific.

**PDF engine reuse:** the `webkit2pdf` helper + Rust wrapper from the PDF-export
work (PR #1, branch `feat/pdf-export`) is reused. For v1 it is copied into
`backup-extractor` (or a small shared `pdf-render` crate is extracted once PR #1
merges); the design does not block on that PR.

## `backup-core` API

```rust
/// An opened (and, if needed, unlocked) iOS backup.
pub struct Backup { /* wraps crabapple handle */ }

pub struct DeviceInfo {
    pub device_name: String,
    pub product_version: String, // iOS version, drives schema selection
    pub udid: String,
    pub last_backup: Option<String>,
}

impl Backup {
    /// Open a backup directory. `password` is required for encrypted backups
    /// (Err with a clear message if encrypted and password is missing/wrong).
    pub fn open(dir: &Path, password: Option<&str>) -> Result<Self, BackupError>;

    pub fn device_info(&self) -> &DeviceInfo;

    /// Decrypt (if needed) the file at `domain` + `relative_path` to a temp
    /// file and return its path. `None` when the file is absent from the backup.
    pub fn fetch(&self, domain: &str, relative_path: &str)
        -> Result<Option<PathBuf>, BackupError>;

    /// Stream/copy every file whose (domain, relative_path) matches `pred`
    /// into `dest`, preserving a chosen relative layout. Used for media.
    pub fn copy_files(
        &self,
        pred: &dyn Fn(&str, &str) -> bool,
        dest: &Path,
    ) -> Result<u64, BackupError>; // returns count copied
}
```

`backup-core` never parses application data — it only yields files. Extractors
open the returned SQLite files read-only (`rusqlite`, already a workspace dep).

## CLI

```
backup-extractor --backup <dir> [--password <pw>] \
                 <contacts|calls|photos|notes> \
                 -f <format> -o <out> [filters]
```

Global options:

- `--backup <dir>` (required): path to the backup directory.
- `--password <pw>` / interactive prompt: for encrypted backups (mirrors
  `imessage-exporter`'s `-x` / prompt behavior; never required for unencrypted).
- `-o <out>` (required): output directory.
- `--no-progress`: as in `imessage-exporter`.

Per-subcommand:

- `-f <format>`: allowed values depend on the type (see below). Required.
- `--start-date` / `--end-date`: where a type has timestamps (calls, photos,
  notes).

A missing store (the app/data is not in this backup) is reported and the command
exits cleanly (success, nothing to write), never a crash.

## Extractors

### Contacts

- **Store:** `HomeDomain` / `Library/AddressBook/AddressBook.sqlitedb`.
- **Schema:** `ABPerson` (`First`, `Last`, `Organization`, `Note`, `Birthday`),
  `ABMultiValue` (phones/emails/urls by `record_id`, with `label`),
  `ABMultiValueLabel` (label text). Contact images in `ABPersonImage` — skipped
  in v1.
- **Model:** `Contact { first, last, organization, phones: Vec<Labeled>,
  emails: Vec<Labeled>, addresses: Vec<Labeled>, birthday, note }` where
  `Labeled { label: String, value: String }`.
- **Formats:** `vcf` (vCard 3.0, importable), `csv`, `json`, `html`, `pdf`.
- **Gotchas:** label strings like `_$!<Home>!$_` need normalizing; some rows have
  null names (company-only). Low risk.

### Call history

- **Store:** `HomeDomain` / `Library/CallHistoryDB/CallHistory.storedata`
  (Core Data SQLite). Older iOS variant:
  `Library/CallHistory/call_history.db` — detect by presence and branch.
- **Schema:** `ZCALLRECORD` — `ZADDRESS` (number, may be a blob),
  `ZDATE` (Core Data seconds since 2001-01-01 00:00:00 UTC; add **978307200** for
  Unix), `ZDURATION`, `ZORIGINATED` (1 outgoing / 0 incoming),
  `ZANSWERED` (0 missed), `ZCALLTYPE` / `ZSERVICE_PROVIDER` (phone vs FaceTime).
- **Model:** `Call { number, timestamp, duration_secs, direction, answered,
  kind }` (`direction`: Incoming/Outgoing; `kind`: Phone/FaceTimeAudio/Video).
- **Formats:** `csv`, `json`, `html`, `pdf`.
- **Gotchas:** the Core Data epoch offset; `ZADDRESS` sometimes stored as a blob;
  missed = `ZANSWERED == 0`. Low risk.

### Photos / videos

- **Media:** `CameraRollDomain` — files under `Media/DCIM/**` and
  `Media/PhotoData/**`.
- **Metadata DB:** `CameraRollDomain` / `Media/PhotoData/Photos.sqlite`.
- **Schema:** `ZASSET` — `ZFILENAME`, `ZDIRECTORY`, `ZDATECREATED` (Core Data
  epoch), `ZFAVORITE`, `ZLATITUDE`/`ZLONGITUDE`, `ZHEIGHT`/`ZWIDTH`,
  `ZKIND` (image/video). Albums via `ZGENERICALBUM` + `Z_*ASSETS` join tables.
  **Schema columns differ across iOS versions — select queries by
  `DeviceInfo.product_version`, using iLEAPP's Photos.sqlite parsers as the
  reference for each version.**
- **Model:** `Photo { filename, source_relative_path, taken, favorite, gps,
  dimensions, kind, albums: Vec<String> }`.
- **Output:**
  - Copy the asset files into `<out>/photos/` (flat, or `--by album|date`),
    converting HEIC → JPEG with `sips`/ImageMagick for the readable formats
    (keep originals on conversion failure).
  - `csv` / `json`: a metadata index of all assets.
  - `html`: a thumbnail gallery linking the copied files.
  - `pdf`: contact-sheet style gallery (via the WebKit/Quartz engine).
- **Gotchas:** volume can be many GB (date/album filters matter); Live Photos are
  a paired `.heic` + `.mov` (copy both, link the still); edited photos have a
  separate render in `Media/PhotoData/Mutations` — copy the original in v1;
  schema version drift is the main risk. Medium risk.

### Notes

- **Store (modern, iOS 9+):** `AppDomainGroup-group.com.apple.notes` /
  `NoteStore.sqlite`. Legacy (older iOS):
  `HomeDomain` / `Library/Notes/notes.sqlite` (plaintext) — branch on presence.
- **Schema (modern):** `ZICCLOUDSYNCINGOBJECT` (note records: title, folder,
  `ZMODIFICATIONDATE1`), note body in `ZICNOTEDATA.ZDATA` =
  **gzip-compressed Apple "note store" protobuf**. v1: gunzip, parse the
  protobuf, extract the top-level note text string (formatting/embedded objects
  ignored). iLEAPP's `notes`/`applenotes` parser is the reference for the
  protobuf layout.
- **Model:** `Note { title, folder, modified, text }`.
- **Formats:** `txt`/`md`, `html`, `pdf`.
- **Gotchas:** the gzip+protobuf body is the single hardest part of the whole
  project; protobuf field numbers and the embedded-object structure must be
  pinned from the reference. Tables, drawings, scanned docs, and attachments are
  out of scope for v1. High risk — schedule last.

## Formatters

A small set of traits keeps formats orthogonal to extractors:

- `Csv` / `Json`: blanket over any `serde::Serialize` model (`csv` + `serde_json`
  crates).
- `Html`: per-type template (askama, already a workspace dep) → an HTML file
  (plus a copied attachments/media folder where relevant).
- `Pdf`: render the type's HTML with the WebKit/Quartz `webkit2pdf` helper, as in
  `imessage-exporter`'s Quartz engine.
- `Vcard`: contacts only.

Each subcommand validates `-f` against the formats it supports and emits a clear
error otherwise.

## Error handling

- Encrypted backup with missing/wrong password → clear error before any work.
- Store absent (app/data not backed up) → reported, clean exit, no output.
- Per-record parse failure → logged and skipped; never aborts the whole export.
- HEIC/media conversion failure → keep the original file, continue.
- All `backup-core` failures surface a `BackupError` with context (which file).

## Testing

- **Per extractor:** unit tests against tiny synthetic fixture stores committed
  to the repo — a minimal `AddressBook.sqlitedb`, `CallHistory.storedata`,
  `Photos.sqlite`, and `NoteStore.sqlite` with 2–3 records each (mirrors
  `imessage-database`'s `test_data` approach). For Notes, a committed
  gzip+protobuf body fixture.
- **Formatters:** tested on fixed in-memory models (golden CSV/JSON/vCard/HTML
  strings).
- **`backup-core`:** tested against a tiny synthetic backup directory (a couple
  of files + a minimal `Manifest.db`); encryption path tested if a fixture
  encrypted backup is feasible, otherwise covered by `crabapple`'s own tests.
- PDF rendering (which needs a GUI session) stays a manual/real-data check, as in
  `imessage-exporter`.

## Build order (low risk first, value early)

1. **`backup-core`** — open/decrypt/fetch, device info, the synthetic-backup test.
2. **Contacts** — easiest store; establishes the extractor + formatter pattern
   (incl. vCard).
3. **Calls** — second easy store; proves date handling.
4. **Photos** — media copy + HEIC conversion + gallery; introduces media volume
   and schema-version handling.
5. **Notes** — last; the gzip+protobuf body, with the most reference work.

## Dependencies

- `crabapple` (backup access + decryption) — already used by `imessage-exporter`.
- `rusqlite` (read-only SQLite) — already a workspace dep.
- `serde` / `serde_json` / `csv` — structured output.
- `askama` (HTML templates) — already a workspace dep.
- `flate2` (gunzip Notes body) and a protobuf decoder (e.g. `prost`, or a
  hand-rolled minimal varint reader) — Notes only.
- The `webkit2pdf` helper from the PDF-export work — for PDF output.

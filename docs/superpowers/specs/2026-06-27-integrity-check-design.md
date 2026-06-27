# `integrity` — Backup Completeness Check — Design

**Status:** Approved (2026-06-27) — autonomous build per approved program roadmap
**Component:** `archive-core` (the check, over crabapple's manifest) + `archive`
(a read-only `integrity` command). No new dependency.

## Goal

Add an `archive integrity` command that answers "is this backup complete and
intact, or was the transfer truncated?" — for each file the backup's `Manifest.db`
claims, verify the corresponding stored file actually exists on disk (and, for
unencrypted backups, that its size matches). This is a read-only triage step
before relying on a backup for recovery.

## What can and cannot be verified (grounded)

`Manifest.db` records, per entry: `domain`, `relative_path`, `flags`, and metadata
(`size`, `mode`, …). The stored file lives at `<backup>/<id[..2]>/<id>` where
`id = SHA1(domain-relative_path)` (crabapple's `BackupFileEntry::source()`).

- **Completeness (always):** every entry that is a *regular file* must have its
  stored file present on disk. Missing files = an incomplete/truncated backup.
  Directory and symlink entries have no stored content and are excluded
  (detected via `mode & S_IFMT`: regular = `S_IFREG`).
- **Size (unencrypted only):** the on-disk file size should equal the manifest's
  recorded `size`. For **encrypted** backups the stored blob is AES-padded, so its
  on-disk size legitimately differs — size checking is **skipped** and reported as
  not performed.
- **Content hashing is not possible:** `Manifest.db` stores a *path*-derived file
  id, not a content hash, so per-file content integrity cannot be verified from the
  manifest alone. (Documented; out of scope.)

## `archive-core` API

```rust
pub struct IntegrityReport {
    pub total_files: usize,       // regular-file entries considered
    pub present: usize,
    pub missing: usize,
    pub size_checked: bool,       // false for encrypted backups
    pub size_mismatch: usize,     // 0 when size_checked is false
    pub missing_sample: Vec<String>,    // "domain:relative_path", capped
    pub mismatch_sample: Vec<String>,   // capped
}

impl Backup {
    /// Verify every regular-file manifest entry has its stored file on disk (and,
    /// for unencrypted backups, that the size matches). `sample_cap` bounds each
    /// sample list. Read-only; decrypts nothing.
    pub fn verify_integrity(&self, sample_cap: usize) -> Result<IntegrityReport, BackupError>;
}
```

Implementation: iterate `raw.entries()`; keep entries where
`is_regular_file(metadata.mode)`; for each, `present = backup_path.join(source()).exists()`;
when `size_checked`, compare `fs::metadata(..).len()` to `metadata.size`. Push the
first `sample_cap` missing / mismatched as `"<domain>:<relative_path>"`.
`is_regular_file(mode) = (mode & 0o170000) == 0o100000` — a pure, unit-tested helper.

## CLI surface

```
archive --backup <DIR> [--password <PW>] integrity
```

Read-only; does **not** need `--out` (like `inspect`). The `sample_cap` is fixed
(e.g. 20) so the envelope stays bounded.

## Output envelope

```json
{
  "ok": true,
  "command": "integrity",
  "complete": false,
  "total_files": 48213,
  "present": 48200,
  "missing": 13,
  "size_checked": true,
  "size_mismatch": 0,
  "missing_sample": ["CameraRollDomain:Media/DCIM/100APPLE/IMG_0007.HEIC", "..."],
  "mismatch_sample": [],
  "device": { "name": "...", "model": "...", "ios": "...", "serial": "...", "udid": "..." }
}
```

- `complete` = `missing == 0 && size_mismatch == 0`.
- `ok` stays `true` even when incomplete — this is a *report*, not a failure
  (exit 0). Auth/parse failures still use the normal error envelope/exit codes.
- `size_checked: false` (encrypted) → `size_mismatch: 0` and a `note` that size
  verification was skipped.
- Samples are capped; `missing`/`size_mismatch` carry the true totals.

## Error handling

| Situation | Behavior |
|---|---|
| Manifest unreadable | exit 1, `kind:"other"` |
| Encrypted/locked, no password | exit 2, `kind:"auth"` (open fails first) |
| Backup complete | `ok:true`, `complete:true`, `missing:0` |
| Backup incomplete | `ok:true`, `complete:false`, counts + samples |

## Testing

- `is_regular_file`: unit tests — `S_IFREG|0644` → true; `S_IFDIR|0755`,
  `S_IFLNK|0777`, `0` → false.
- `verify_integrity`: gated real-backup test (`ARCHIVE_TEST_BACKUP`) asserting
  `present + missing == total_files`, `complete` consistent with the counts, and
  samples capped at `sample_cap`; skipped without a backup (matches archive-core
  convention — no manifest can be faked without a full backup fixture).
- CLI: `integrity` parses and runs without `--out`; envelope shape (the
  `complete` field; `size_checked` reflects encryption).

## Out of scope (YAGNI)

- Per-file content hashing / corruption detection beyond size (not derivable from
  the manifest).
- Repairing or re-fetching missing files.
- Listing *all* missing files (the capped sample + totals suffice; a future
  `--full-list` could dump them).

## Global constraints (carried from the project)

- Agent-first contract: one JSON object on stdout; progress on stderr; exit
  `0`/`1`/`2`; clap errors the only non-JSON exit-2 case. A report with findings
  is still exit 0.
- Password via flag/env; never prompts.
- No new dependency.
- `archive` does not depend on the `imessage-*` crates.
- GPL-3.0-or-later; conventional commits; never squash.
```
